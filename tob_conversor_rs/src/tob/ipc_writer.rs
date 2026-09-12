//! Arrow IPC export: shared column buffers, batch reader, in-memory and on-disk IPC File output.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::Path;
use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Float32Builder, Float64Builder, Int16Builder, Int32Builder, StringBuilder,
    TimestampNanosecondBuilder, UInt16Builder, UInt32Builder,
};
use arrow_array::{ArrayRef, Int64Array, RecordBatch};
use arrow_ipc::writer::{FileWriter, StreamWriter};
use arrow_schema::{DataType, Field, Schema, TimeUnit};

use super::decode::{FieldValue, decode_field_value, time_ns_from_frame_header};
use super::header::{TobHeader, parse_tob_header};
use super::subframe::{is_valid_main_frame, scan_and_skip_subframe_boundary};
use super::types::CsciType;

/// Default batch size — keeps peak memory at ~4–10 MiB per chunk.
pub(crate) const BATCH_SIZE: usize = 65_536;

// ── Typed column buffers ──────────────────────────────────────────────────────

pub(crate) enum ColBuf {
    F32 { vals: Vec<f32>, valid: Vec<bool> },
    F64 { vals: Vec<f64>, valid: Vec<bool> },
    I16 { vals: Vec<i16>, valid: Vec<bool> },
    U16 { vals: Vec<u16>, valid: Vec<bool> },
    I32 { vals: Vec<i32>, valid: Vec<bool> },
    U32 { vals: Vec<u32>, valid: Vec<bool> },
    Bool { vals: Vec<bool>, valid: Vec<bool> },
    TsNs { vals: Vec<i64>, valid: Vec<bool> },
    Str(Vec<Option<String>>),
}

impl ColBuf {
    pub(crate) fn with_capacity(dt: &CsciType, cap: usize) -> Self {
        match dt {
            CsciType::Ieee4 | CsciType::Ieee4b | CsciType::Fp2 => ColBuf::F32 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Ieee8 | CsciType::Ieee8b | CsciType::Fp4 => ColBuf::F64 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Short | CsciType::Int2 => ColBuf::I16 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Ushort | CsciType::Uint2 => ColBuf::U16 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Long | CsciType::Int4 => ColBuf::I32 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Ulong | CsciType::Uint4 => ColBuf::U32 {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Bool | CsciType::Bool8 | CsciType::Bool2 | CsciType::Bool4 => ColBuf::Bool {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Nsec | CsciType::Secnano => ColBuf::TsNs {
                vals: Vec::with_capacity(cap),
                valid: Vec::with_capacity(cap),
            },
            CsciType::Ascii(_) => ColBuf::Str(Vec::with_capacity(cap)),
        }
    }

    pub(crate) fn push_field_value(&mut self, fv: FieldValue) {
        match (self, fv) {
            (ColBuf::F32 { vals, valid }, FieldValue::F32(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::F64 { vals, valid }, FieldValue::F64(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::I16 { vals, valid }, FieldValue::I16(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::U16 { vals, valid }, FieldValue::U16(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::I32 { vals, valid }, FieldValue::I32(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::U32 { vals, valid }, FieldValue::U32(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::Bool { vals, valid }, FieldValue::Bool(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::TsNs { vals, valid }, FieldValue::TimestampNs(v)) => {
                vals.push(v);
                valid.push(true);
            }
            (ColBuf::Str(v), FieldValue::Str(s)) => v.push(Some(s)),
            (buf, FieldValue::Null) => buf.push_null(),
            _ => unreachable!("ColBuf type mismatch — corrupt frame or header"),
        }
    }

    pub(crate) fn push_null(&mut self) {
        match self {
            ColBuf::F32 { vals, valid } => {
                vals.push(0.0);
                valid.push(false);
            }
            ColBuf::F64 { vals, valid } => {
                vals.push(0.0);
                valid.push(false);
            }
            ColBuf::I16 { vals, valid } => {
                vals.push(0);
                valid.push(false);
            }
            ColBuf::U16 { vals, valid } => {
                vals.push(0);
                valid.push(false);
            }
            ColBuf::I32 { vals, valid } => {
                vals.push(0);
                valid.push(false);
            }
            ColBuf::U32 { vals, valid } => {
                vals.push(0);
                valid.push(false);
            }
            ColBuf::Bool { vals, valid } => {
                vals.push(false);
                valid.push(false);
            }
            ColBuf::TsNs { vals, valid } => {
                vals.push(0);
                valid.push(false);
            }
            ColBuf::Str(v) => v.push(None),
        }
    }

    pub(crate) fn clear(&mut self) {
        match self {
            ColBuf::F32 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::F64 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::I16 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::U16 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::I32 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::U32 { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::Bool { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::TsNs { vals, valid } => {
                vals.clear();
                valid.clear();
            }
            ColBuf::Str(v) => v.clear(),
        }
    }

    pub(crate) fn to_array(&self) -> ArrayRef {
        match self {
            ColBuf::F32 { vals, valid } => {
                let mut b = Float32Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::F64 { vals, valid } => {
                let mut b = Float64Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::I16 { vals, valid } => {
                let mut b = Int16Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::U16 { vals, valid } => {
                let mut b = UInt16Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::I32 { vals, valid } => {
                let mut b = Int32Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::U32 { vals, valid } => {
                let mut b = UInt32Builder::with_capacity(vals.len());
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::Bool { vals, valid } => {
                let mut b = BooleanBuilder::with_capacity(vals.len());
                b.append_values(vals, valid)
                    .expect("Bool append_values length mismatch");
                Arc::new(b.finish())
            }
            ColBuf::TsNs { vals, valid } => {
                let mut b = TimestampNanosecondBuilder::with_capacity(vals.len())
                    .with_timezone(Arc::from("UTC"));
                b.append_values(vals, valid);
                Arc::new(b.finish())
            }
            ColBuf::Str(entries) => {
                let mut b = StringBuilder::with_capacity(entries.len(), entries.len() * 8);
                for opt in entries {
                    match opt {
                        Some(s) => b.append_value(s),
                        None => b.append_null(),
                    }
                }
                Arc::new(b.finish())
            }
        }
    }
}

// ── Column buffer collection ──────────────────────────────────────────────────

pub(crate) struct Buffers {
    pub timestamps: Vec<i64>,
    pub records: Vec<i64>,
    pub cols: Vec<ColBuf>,
}

impl Buffers {
    pub(crate) fn new(dtypes: &[CsciType]) -> Self {
        Self {
            timestamps: Vec::new(),
            records: Vec::new(),
            cols: dtypes
                .iter()
                .map(|dt| ColBuf::with_capacity(dt, 0))
                .collect(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.timestamps.clear();
        self.records.clear();
        for col in &mut self.cols {
            col.clear();
        }
    }
}

// ── Schema builder ────────────────────────────────────────────────────────────

pub(crate) fn json_array(items: &[String]) -> String {
    let inner: Vec<String> = items
        .iter()
        .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
        .collect();
    format!("[{}]", inner.join(","))
}

pub(crate) fn build_schema(header: &TobHeader, include_record: bool) -> Schema {
    let mut fields: Vec<Field> = Vec::new();

    fields.push(Field::new(
        "TIMESTAMP",
        DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
        false,
    ));

    if include_record {
        fields.push(Field::new("RECORD", DataType::Int64, true));
    }

    for i in 0..header.names.len() {
        let mut field_meta = HashMap::new();
        field_meta.insert("units".to_string(), header.units[i].clone());
        field_meta.insert("processing".to_string(), header.processing[i].clone());

        let field = Field::new(
            header.names[i].as_str(),
            header.csci_dtypes[i].to_arrow_type(),
            true,
        )
        .with_metadata(field_meta);

        fields.push(field);
    }

    let mut schema_meta = HashMap::new();
    schema_meta.insert("tob_station_name".to_string(), header.station_name.clone());
    schema_meta.insert("tob_logger_model".to_string(), header.logger_model.clone());
    schema_meta.insert("tob_logger_sn".to_string(), header.logger_sn.clone());
    schema_meta.insert("tob_logger_os".to_string(), header.logger_os.clone());
    schema_meta.insert(
        "tob_logger_program".to_string(),
        header.logger_program.clone(),
    );
    schema_meta.insert(
        "tob_logger_program_signature".to_string(),
        header.logger_program_signature.clone(),
    );
    schema_meta.insert("tob_table_name".to_string(), header.table_name.clone());
    schema_meta.insert(
        "tob_rec_intvl_sec".to_string(),
        header.rec_intvl.to_string(),
    );
    schema_meta.insert("tob_column_names".to_string(), json_array(&header.names));
    schema_meta.insert("tob_units".to_string(), json_array(&header.units));
    schema_meta.insert("tob_processing".to_string(), json_array(&header.processing));

    Schema::new_with_metadata(fields, schema_meta)
}

// ── Frame and record collectors ───────────────────────────────────────────────

pub(crate) fn collect_frame(frame_buf: &[u8], header: &TobHeader, buffers: &mut Buffers) {
    let mut seconds = u32::from_le_bytes(frame_buf[0..4].try_into().unwrap());
    let mut subseconds = u32::from_le_bytes(frame_buf[4..8].try_into().unwrap());
    let main_beg = if header.is_tob2 {
        0
    } else {
        u32::from_le_bytes(frame_buf[8..12].try_into().unwrap())
    };

    let mut frame_time_ns = time_ns_from_frame_header(seconds, subseconds, header.frame_time_res);
    let mut next_record_id = main_beg;

    let data_end = header.frame_nbytes.saturating_sub(4);
    let line_step = header.line_nbytes + header.data_line_padding;
    let mut off = if header.is_tob2 { 8usize } else { 12usize };

    while off < data_end {
        if scan_and_skip_subframe_boundary(
            frame_buf,
            &mut off,
            data_end,
            header,
            next_record_id,
            &mut seconds,
            &mut subseconds,
            &mut frame_time_ns,
        ) {
            continue;
        }

        if off + header.line_nbytes > data_end {
            break;
        }

        let line_bytes = &frame_buf[off..off + header.line_nbytes];
        off += line_step;

        buffers.timestamps.push(frame_time_ns);
        buffers.records.push(next_record_id as i64);
        let mut byte_off = 0;
        for (dt, col_buf) in header.csci_dtypes.iter().zip(buffers.cols.iter_mut()) {
            let sz = dt.size();
            let fv = decode_field_value(dt, &line_bytes[byte_off..byte_off + sz], header.fp2_nan);
            col_buf.push_field_value(fv);
            byte_off += sz;
        }

        next_record_id = next_record_id.wrapping_add(1);
        frame_time_ns += (header.rec_intvl * 1_000_000_000.0) as i64;
    }
}

pub(crate) fn collect_tob1_record(record_buf: &[u8], header: &TobHeader, buffers: &mut Buffers) {
    let seconds = u32::from_le_bytes(record_buf[0..4].try_into().unwrap());
    let subseconds = u32::from_le_bytes(record_buf[4..8].try_into().unwrap());
    let record_id = u32::from_le_bytes(record_buf[8..12].try_into().unwrap());
    let frame_time_ns = time_ns_from_frame_header(seconds, subseconds, header.frame_time_res);

    let line_bytes = &record_buf[12..];
    buffers.timestamps.push(frame_time_ns);
    buffers.records.push(record_id as i64);
    let mut byte_off = 0;
    for (dt, col_buf) in header.csci_dtypes.iter().zip(buffers.cols.iter_mut()) {
        let sz = dt.size();
        let fv = decode_field_value(dt, &line_bytes[byte_off..byte_off + sz], header.fp2_nan);
        col_buf.push_field_value(fv);
        byte_off += sz;
    }
}

// ── Record batch builder ──────────────────────────────────────────────────────

pub(crate) fn build_record_batch(
    schema: &Arc<Schema>,
    _header: &TobHeader,
    include_record: bool,
    buffers: &Buffers,
) -> Result<RecordBatch, String> {
    let n = buffers.len();
    let mut arrays: Vec<ArrayRef> = Vec::new();

    {
        let mut b = TimestampNanosecondBuilder::with_capacity(n).with_timezone(Arc::from("UTC"));
        b.append_values(&buffers.timestamps, &vec![true; n]);
        arrays.push(Arc::new(b.finish()));
    }

    if include_record {
        arrays.push(Arc::new(Int64Array::from(buffers.records.clone())));
    }

    for col in &buffers.cols {
        arrays.push(col.to_array());
    }

    debug_assert!(
        arrays.iter().all(|a| a.len() == n),
        "Array length mismatch in build_record_batch"
    );

    RecordBatch::try_new(schema.clone(), arrays).map_err(|e| e.to_string())
}

// ── Streaming batch reader ────────────────────────────────────────────────────

/// Streaming iterator over a TOB file, yielding Arrow `RecordBatch` chunks.
///
/// Each call to `next()` reads rows until `batch_size` is reached (or EOF),
/// then returns a single `RecordBatch`.  Reuses internal allocations so memory
/// is bounded to roughly one batch at a time.
pub struct TobBatchReader {
    reader: BufReader<File>,
    header: TobHeader,
    schema: Arc<Schema>,
    buffers: Buffers,
    include_record: bool,
    batch_size: usize,
    done: bool,
    frame_buf: Vec<u8>,
    record_buf: Vec<u8>,
}

impl TobBatchReader {
    /// Open a TOB file and prepare it for batch iteration.
    pub fn open(path: &Path, include_record: bool, batch_size: usize) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Open error: {}", e))?;
        let mut reader = BufReader::new(file);
        let header =
            parse_tob_header(&mut reader).map_err(|e| format!("Header parse error: {}", e))?;
        let schema = Arc::new(build_schema(&header, include_record));
        let buffers = Buffers::new(&header.csci_dtypes);

        let (frame_buf, record_buf) = if header.is_tob1 {
            (vec![], vec![0u8; 12 + header.line_nbytes])
        } else {
            if header.frame_nbytes == 0 {
                return Err("frame_nbytes is 0 in non-TOB1 file".to_string());
            }
            (vec![0u8; header.frame_nbytes], vec![])
        };

        Ok(Self {
            reader,
            header,
            schema,
            buffers,
            include_record,
            batch_size,
            done: false,
            frame_buf,
            record_buf,
        })
    }

    pub fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }

    pub fn header(&self) -> &TobHeader {
        &self.header
    }
}

impl Iterator for TobBatchReader {
    type Item = Result<RecordBatch, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        if self.header.is_tob1 {
            loop {
                match self.reader.read_exact(&mut self.record_buf) {
                    Ok(()) => {}
                    Err(_) => {
                        self.done = true;
                        break;
                    }
                }
                collect_tob1_record(&self.record_buf, &self.header, &mut self.buffers);
                if self.buffers.len() >= self.batch_size {
                    break;
                }
            }
        } else {
            loop {
                match self.reader.read_exact(&mut self.frame_buf) {
                    Ok(()) => {}
                    Err(_) => {
                        self.done = true;
                        break;
                    }
                }
                if !is_valid_main_frame(&self.frame_buf, &self.header) {
                    continue;
                }
                collect_frame(&self.frame_buf, &self.header, &mut self.buffers);
                if self.buffers.len() >= self.batch_size {
                    break;
                }
            }
        }

        if self.buffers.is_empty() {
            return None;
        }

        let result = build_record_batch(
            &self.schema,
            &self.header,
            self.include_record,
            &self.buffers,
        );
        self.buffers.clear();
        Some(result)
    }
}

// ── Public IPC export functions ───────────────────────────────────────────────

/// Serialize the entire TOB file to Arrow IPC **File** format in memory.
///
/// Uses `FileWriter` (seekable, has footer) — required by `pl.scan_ipc()`.
pub fn to_arrow_ipc_bytes(path: &Path, include_record: bool) -> Result<Vec<u8>, String> {
    let mut reader = TobBatchReader::open(path, include_record, BATCH_SIZE)?;
    let schema = reader.schema();

    let cursor = std::io::Cursor::new(Vec::<u8>::new());
    let mut writer = FileWriter::try_new(cursor, &schema).map_err(|e| e.to_string())?;

    for batch in &mut reader {
        let batch = batch?;
        writer.write(&batch).map_err(|e| e.to_string())?;
    }

    writer.finish().map_err(|e| e.to_string())?;
    let buf = writer.into_inner().map_err(|e| e.to_string())?.into_inner();
    Ok(buf)
}

/// Write the TOB file as Arrow IPC **File** format to `output_path` on disk.
///
/// Returns the total number of rows written, or 0 if the file contained no data.
pub fn to_arrow_ipc_file(
    path: &Path,
    output_path: &Path,
    include_record: bool,
) -> Result<usize, String> {
    let mut reader = TobBatchReader::open(path, include_record, BATCH_SIZE)?;
    let schema = reader.schema();

    let out_file = File::create(output_path).map_err(|e| e.to_string())?;
    let mut writer =
        FileWriter::try_new(BufWriter::new(out_file), &schema).map_err(|e| e.to_string())?;

    let mut total_rows = 0usize;
    for batch in &mut reader {
        let batch = batch?;
        total_rows += batch.num_rows();
        writer.write(&batch).map_err(|e| e.to_string())?;
    }

    if total_rows == 0 {
        drop(writer);
        let _ = std::fs::remove_file(output_path);
        return Ok(0);
    }

    writer.finish().map_err(|e| e.to_string())?;
    Ok(total_rows)
}

/// Serialize a single `RecordBatch` as a complete Arrow IPC **Stream** message.
///
/// The result is a self-describing byte sequence (schema + one batch) that can
/// be deserialized on the Python side with `pa.ipc.open_stream(io.BytesIO(buf))`.
pub fn batch_to_ipc_stream_bytes(
    schema: &Arc<Schema>,
    batch: &RecordBatch,
) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(Vec::<u8>::new());
    let mut writer = StreamWriter::try_new(cursor, schema).map_err(|e| e.to_string())?;
    writer.write(batch).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(writer.into_inner().map_err(|e| e.to_string())?.into_inner())
}

/// Serialize just the schema as an Arrow IPC Stream header (no batches).
pub fn schema_to_ipc_stream_bytes(schema: &Arc<Schema>) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(Vec::<u8>::new());
    let mut writer = StreamWriter::try_new(cursor, schema).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(writer.into_inner().map_err(|e| e.to_string())?.into_inner())
}
