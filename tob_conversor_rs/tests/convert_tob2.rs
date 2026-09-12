//! Tests for decoding TOB2 files natively.

mod common;

use std::fs;
use std::io::Write;

use common::{VAL_STAMP, temp_dir};
use tobconversor::convert_streaming;

pub fn header_block_tob2(logger_model: &str, frame_nbytes: u32, val_stamp: u16) -> String {
    format!(
        r#""TOB2","ST","{}","123","OS","PROG","SIG","2020-01-01 00:00:00"
"T1","1 SEC","{}","0","{}","SecMsec","0","0","0"
"A","B"
"V","V"
"Smp","Smp"
"FP2","FP2"
"#,
        logger_model, frame_nbytes, val_stamp
    )
}

/// TOB2 valid frame: 8 B header + 4 B payload (2×FP2) + 4 B footer (`val_stamp` in high word).
/// Total size = 16 bytes.
pub fn one_frame_tob2(seconds: u32, val_stamp: u16) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&seconds.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes()); // subseconds
    // payload: 2 FP2 values
    f.extend_from_slice(&[0x45u8, 0x8e, 0x45, 0x8e]);
    // footer
    let footer_word = (val_stamp as u32) << 16;
    f.extend_from_slice(&footer_word.to_le_bytes());
    assert_eq!(f.len(), 16);
    f
}

pub fn write_synthetic_tob2(path: &std::path::Path, logger_model: &str) {
    let mut blob = header_block_tob2(logger_model, 16, VAL_STAMP).into_bytes();
    blob.extend(one_frame_tob2(100, VAL_STAMP));
    blob.extend(one_frame_tob2(101, VAL_STAMP));
    blob.extend(one_frame_tob2(102, VAL_STAMP));
    let mut file = fs::File::create(path).unwrap();
    file.write_all(&blob).unwrap();
}

#[test]
fn synthetic_tob2_streaming_reads_all_frames() {
    let dir = temp_dir("tob2");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("test.dat");
    write_synthetic_tob2(&input, "CR1000");

    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert TOB2 error");
    assert!(n >= 1, "expected at least one split output file from TOB2");

    let outs: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("test_")
        })
        .collect();

    assert_eq!(outs.len(), 1, "rows should land in one 30-min file");
    let text = fs::read_to_string(&outs[0]).unwrap();

    // Assert 6 header lines + 3 data lines
    let line_count = text.lines().count();
    assert_eq!(
        line_count,
        4 + 3,
        "Output should contain 4 lines of header + 3 lines of data"
    );

    let third_data_line = text.lines().nth(6).unwrap();
    assert!(
        third_data_line.starts_with('"'),
        "Should be quoted timestamp"
    );

    let _ = fs::remove_dir_all(&dir);
}
