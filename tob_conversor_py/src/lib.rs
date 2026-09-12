use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tobconversor::{
    TobBatchReader, batch_to_ipc_stream_bytes, convert_tob_to_parquet, parse_tob_header,
    schema_to_ipc_stream_bytes, to_arrow_ipc_bytes, to_arrow_ipc_file,
};

/// Parsed representation of the 6-line ASCII TOB header.
#[pyclass(frozen)]
pub struct Header {
    #[pyo3(get)]
    pub station_name: String,
    #[pyo3(get)]
    pub logger_model: String,
    #[pyo3(get)]
    pub logger_sn: String,
    #[pyo3(get)]
    pub logger_os: String,
    #[pyo3(get)]
    pub logger_program: String,
    #[pyo3(get)]
    pub table_name: String,
    /// Record interval in seconds.
    #[pyo3(get)]
    pub rec_intvl: f64,
    #[pyo3(get)]
    pub names: Vec<String>,
    #[pyo3(get)]
    pub units: Vec<String>,
    #[pyo3(get)]
    pub processing: Vec<String>,
    /// CsciType debug strings, e.g. "Ieee4b", "Fp2", "Ascii(16)".
    #[pyo3(get)]
    pub dtypes: Vec<String>,
    #[pyo3(get)]
    pub is_tob1: bool,
    #[pyo3(get)]
    pub is_tob2: bool,
}

#[pymethods]
impl Header {
    fn __repr__(&self) -> String {
        format!(
            "Header(station={:?}, model={:?}, table={:?})",
            self.station_name, self.logger_model, self.table_name
        )
    }
}

/// Parse the 6-line ASCII header from a TOB file without reading binary data.
#[pyfunction]
fn read_header(path: PathBuf) -> PyResult<Header> {
    let f = File::open(&path).map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
    let mut buf = BufReader::new(f);
    let h = parse_tob_header(&mut buf)
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
    Ok(Header {
        station_name: h.station_name,
        logger_model: h.logger_model,
        logger_sn: h.logger_sn,
        logger_os: h.logger_os,
        logger_program: h.logger_program,
        table_name: h.table_name,
        rec_intvl: h.rec_intvl,
        names: h.names,
        units: h.units,
        processing: h.processing,
        dtypes: h.csci_dtypes.iter().map(|t| format!("{t:?}")).collect(),
        is_tob1: h.is_tob1,
        is_tob2: h.is_tob2,
    })
}

/// Convert a TOB file to a Parquet file in `output_dir`.
///
/// Returns the number of rows written (0 if the file contained no valid frames).
#[pyfunction]
#[pyo3(signature = (input, output_dir, include_record = false))]
fn to_parquet(input: PathBuf, output_dir: PathBuf, include_record: bool) -> PyResult<usize> {
    convert_tob_to_parquet(&input, &output_dir, include_record)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))
}

/// Serialize a TOB file to Arrow IPC File format bytes (in memory).
///
/// Returns the raw IPC File bytes.  Pass to `pa.ipc.open_file(io.BytesIO(b))`
/// or `pl.scan_ipc(io.BytesIO(b))` on the Python side.
#[pyfunction]
#[pyo3(signature = (input, include_record = false))]
fn to_arrow_ipc(py: Python<'_>, input: PathBuf, include_record: bool) -> PyResult<Py<PyBytes>> {
    let bytes = to_arrow_ipc_bytes(&input, include_record)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
    Ok(PyBytes::new(py, &bytes).into())
}

/// Write a TOB file as Arrow IPC File format to `output_path` on disk.
///
/// Returns the total number of rows written, or 0 if the file was empty.
#[pyfunction]
#[pyo3(signature = (input, output, include_record = false))]
fn to_arrow_ipc_file_py(input: PathBuf, output: PathBuf, include_record: bool) -> PyResult<usize> {
    to_arrow_ipc_file(&input, &output, include_record)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))
}

/// Streaming reader that yields Arrow IPC Stream bytes one batch at a time.
///
/// Each `__next__` call returns a `bytes` object containing one complete
/// Arrow IPC Stream message (schema + one `RecordBatch`).  Deserialize on the
/// Python side with `pa.ipc.open_stream(io.BytesIO(chunk)).read_next_batch()`.
///
/// The `schema_ipc_bytes` property returns a schema-only IPC Stream message
/// that can be used to build a `pa.RecordBatchReader` externally.
#[pyclass]
struct TobReader {
    inner: TobBatchReader,
}

#[pymethods]
impl TobReader {
    #[new]
    #[pyo3(signature = (path, include_record = false, batch_size = 65536))]
    fn new(path: PathBuf, include_record: bool, batch_size: usize) -> PyResult<Self> {
        let inner = TobBatchReader::open(&path, include_record, batch_size)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
        Ok(Self { inner })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<Py<PyBytes>>> {
        match slf.inner.next() {
            None => Ok(None),
            Some(Err(e)) => Err(pyo3::exceptions::PyRuntimeError::new_err(e)),
            Some(Ok(batch)) => {
                let schema = slf.inner.schema();
                let buf = batch_to_ipc_stream_bytes(&schema, &batch)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                Ok(Some(PyBytes::new(py, &buf).into()))
            }
        }
    }

    /// Arrow IPC Stream bytes containing only the schema (no batches).
    #[getter]
    fn schema_ipc_bytes(&self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        let schema = self.inner.schema();
        let buf = schema_to_ipc_stream_bytes(&schema)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
        Ok(PyBytes::new(py, &buf).into())
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Header>()?;
    m.add_class::<TobReader>()?;
    m.add_function(wrap_pyfunction!(read_header, m)?)?;
    m.add_function(wrap_pyfunction!(to_parquet, m)?)?;
    m.add_function(wrap_pyfunction!(to_arrow_ipc, m)?)?;
    m.add_function(wrap_pyfunction!(to_arrow_ipc_file_py, m)?)?;
    Ok(())
}
