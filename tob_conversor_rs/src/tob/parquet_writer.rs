//! Parquet export: thin wrapper around `TobBatchReader` from `ipc_writer`.

use std::fs::File;
use std::path::Path;

use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use super::ipc_writer::{BATCH_SIZE, TobBatchReader};

/// Convert a TOB1/TOB2/TOB3 file to a single Parquet file.
///
/// Returns `1` on success (one output file), or `0` if no data frames were found.
pub fn convert_tob_to_parquet(
    input: &Path,
    output_dir: &Path,
    include_record: bool,
) -> Result<usize, String> {
    let mut reader = TobBatchReader::open(input, include_record, BATCH_SIZE)?;
    let schema = reader.schema();

    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;

    let base_name = input.file_stem().unwrap().to_string_lossy();
    let out_path = output_dir.join(format!("{}.parquet", base_name));
    let out_file = File::create(&out_path).map_err(|e| e.to_string())?;

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer =
        ArrowWriter::try_new(out_file, schema.clone(), Some(props)).map_err(|e| e.to_string())?;

    let mut total_rows: usize = 0;
    for batch in &mut reader {
        let batch = batch?;
        total_rows += batch.num_rows();
        writer.write(&batch).map_err(|e| e.to_string())?;
    }

    if total_rows == 0 {
        drop(writer);
        let _ = std::fs::remove_file(&out_path);
        return Ok(0);
    }

    writer.close().map_err(|e| e.to_string())?;
    Ok(1)
}
