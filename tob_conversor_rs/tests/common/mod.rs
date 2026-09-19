// Shared test helper utilities: synthetic TOB1/TOB3 blobs and scratch directories.
//
// Each integration test binary compiles this module separately, so any helper
// a given binary does not use looks dead to the compiler.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Validation stamp used by every synthetic frame in the test suite.
pub const VAL_STAMP: u16 = 4660; // 0x1234

/// FP2 encoding of ~14.22 V — the "battery voltage" sentinel the assertions look for.
pub const FP2_BATT: [u8; 2] = [0x45, 0x8e];

/// A unique scratch directory under the system temp dir, tagged for readability.
pub fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tob3_test_{}_{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Six-line TOB3 prolog for a two-column FP2 table.
pub fn header_block(logger_model: &str, frame_nbytes: u32, val_stamp: u16) -> String {
    format!(
        r#""TOB3","ST","{}","123","OS","PROG","SIG","2020-01-01 00:00:00"
"T1","1 SEC","{}","0","{}","SecMsec","0","0","0"
"A","B"
"V","V"
"Smp","Smp"
"FP2","FP2"
"#,
        logger_model, frame_nbytes, val_stamp
    )
}

/// Quote each item of a comma-separated list: `A,B` -> `"A","B"`.
fn quote_list(items: &str) -> String {
    items
        .split(',')
        .map(|s| format!("\"{}\"", s))
        .collect::<Vec<_>>()
        .join(",")
}

/// Six-line TOB3 prolog for a table of arbitrary Campbell types.
///
/// `name`/`unit`/`processing`/`dtype` are comma-separated lists, one entry per
/// column, so a single call can describe a one- or many-column table.
#[allow(clippy::too_many_arguments)]
pub fn header_block_with_types(
    logger_model: &str,
    frame_nbytes: u32,
    val_stamp: u16,
    frame_time_res: &str,
    name: &str,
    unit: &str,
    processing: &str,
    dtype: &str,
) -> String {
    format!(
        r#""TOB3","ST","{}","123","OS","PROG","SIG","2020-01-01 00:00:00"
"T1","1 SEC","{}","0","{}","{}","0","0","0"
{}
{}
{}
{}
"#,
        logger_model,
        frame_nbytes,
        val_stamp,
        frame_time_res,
        quote_list(name),
        quote_list(unit),
        quote_list(processing),
        quote_list(dtype)
    )
}

/// One valid 20-byte TOB3 main frame: 12 B header, 4 B payload (2×FP2), 4 B footer.
pub fn one_frame(seconds: u32, beg: u32, val_stamp: u16) -> Vec<u8> {
    one_frame_with_payload(&[FP2_BATT, FP2_BATT].concat(), seconds, beg, val_stamp)
}

/// As [`one_frame`], but with a footer stamp that does not match the table —
/// the reader must skip it.
pub fn one_frame_invalid(seconds: u32, beg: u32, val_stamp: u16) -> Vec<u8> {
    one_frame_with_payload(
        &[FP2_BATT, FP2_BATT].concat(),
        seconds,
        beg,
        val_stamp ^ 0x0F0F,
    )
}

/// One valid TOB3 main frame carrying `payload` verbatim.
/// Frame size is `12 + payload.len() + 4`, which is what the header must declare.
pub fn one_frame_with_payload(payload: &[u8], seconds: u32, beg: u32, val_stamp: u16) -> Vec<u8> {
    let mut f = Vec::with_capacity(16 + payload.len());
    f.extend_from_slice(&seconds.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes()); // subseconds
    f.extend_from_slice(&beg.to_le_bytes()); // beg record number
    f.extend_from_slice(payload);
    // Footer: validation stamp in the high word, no flags, zero offset.
    f.extend_from_slice(&(((val_stamp as u32) << 16).to_le_bytes()));
    f
}

/// Write a minimal one-frame TOB3 file: two FP2 columns, one record.
pub fn write_synthetic_tob3(path: &Path, logger_model: &str) {
    let mut blob = header_block(logger_model, 20, VAL_STAMP).into_bytes();
    blob.extend(one_frame(100, 1, VAL_STAMP));
    fs::File::create(path).unwrap().write_all(&blob).unwrap();
}

/// Five-line TOB1 prolog — TOB1 has no frame-geometry line. It declares
/// SECONDS/NANOSECONDS/RECORD as real columns; the reader strips them and
/// takes them from each record's binary prefix instead.
pub fn header_block_tob1(logger_model: &str) -> String {
    format!(
        r#""TOB1","ST","{}","123","OS","PROG","SIG","T1"
"SECONDS","NANOSECONDS","RECORD","A","B"
"SECONDS","NANOSECONDS","RN","V","V"
"","","","Smp","Smp"
"ULONG","ULONG","ULONG","FP2","FP2"
"#,
        logger_model
    )
}

/// One TOB1 record: seconds, subseconds and record number, then the payload.
pub fn one_tob1_record(seconds: u32, subseconds: u32, record: u32, data: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(12 + data.len());
    r.extend_from_slice(&seconds.to_le_bytes());
    r.extend_from_slice(&subseconds.to_le_bytes());
    r.extend_from_slice(&record.to_le_bytes());
    r.extend_from_slice(data);
    r
}
