//! Invalid frames, empty payload, split intervals, column types, and RECORD layout.

mod common;

use std::fs;
use std::io::Write;

use common::{
    VAL_STAMP, header_block, header_block_with_types, one_frame, one_frame_invalid,
    one_frame_with_payload, temp_dir, write_synthetic_tob3,
};
use tobconversor::convert_streaming;

#[test]
fn multiple_frames_file_split() {
    let dir = temp_dir("split");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("split.dat");
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    blob.extend(one_frame(100, 1, VAL_STAMP));
    blob.extend(one_frame(100 + 1900, 2, VAL_STAMP));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).unwrap();
    assert_eq!(n, 2, "30-min split: frames 100s and 2000s apart → 2 files");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_frame_is_skipped() {
    let dir = temp_dir("invalid");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("inv.dat");
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    blob.extend(one_frame_invalid(100, 1, VAL_STAMP));
    blob.extend(one_frame(200, 1, VAL_STAMP));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).unwrap();
    assert_eq!(n, 1);
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("inv_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    let data_lines: Vec<_> = text.lines().skip(4).collect();
    assert_eq!(
        data_lines.len(),
        1,
        "only one valid row; got {:?}",
        data_lines
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_frames_returns_zero() {
    let dir = temp_dir("empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("empty.dat");
    fs::File::create(&input)
        .unwrap()
        .write_all(header_block("CR1000", 20, VAL_STAMP).as_bytes())
        .unwrap();
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).unwrap();
    assert_eq!(n, 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ieee4_column_roundtrip() {
    let dir = temp_dir("ieee4");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("f.dat");
    let hdr = header_block_with_types("CR1000", 20, VAL_STAMP, "SecMsec", "X", "V", "Smp", "IEEE4");
    let v: f32 = 1.25;
    let mut blob = hdr.into_bytes();
    blob.extend(one_frame_with_payload(&v.to_le_bytes(), 100, 1, VAL_STAMP));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();
    let out = dir.join("out");
    convert_streaming(&input, &out, 30, false).unwrap();
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("f_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    let data = text.lines().nth(4).expect("data row after 4-line header");
    assert!(data.contains("1.25"), "expected IEEE4 1.25 in row: {data}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bool_column_roundtrip() {
    let dir = temp_dir("bool");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("b.dat");
    let hdr = header_block_with_types(
        "CR1000", 17, VAL_STAMP, "SecMsec", "Flag", "", "Smp", "BOOL",
    );
    let mut blob = hdr.into_bytes();
    blob.extend(one_frame_with_payload(&[1u8], 100, 1, VAL_STAMP));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();
    let out = dir.join("out");
    convert_streaming(&input, &out, 30, false).unwrap();
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("b_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    let data = text.lines().nth(4).expect("data row");
    assert_eq!(
        data.split(',').nth(1),
        Some("1"),
        "BOOL should decode to unquoted 1: {data}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn include_record_column_position() {
    let dir = temp_dir("recpos");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("rec.dat");
    write_synthetic_tob3(&input, "CR1000");
    let out = dir.join("out");
    convert_streaming(&input, &out, 30, true).unwrap();
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("rec_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    let col_names = text.lines().nth(1).unwrap();
    assert!(
        col_names.starts_with("\"TIMESTAMP\",\"RECORD\""),
        "RECORD should be second column: {col_names}"
    );
    let data = text.lines().nth(4).unwrap();
    assert_eq!(
        data.split(',').nth(1),
        Some("1"),
        "second CSV field should be RECORD (unquoted): {data}"
    );
    let _ = fs::remove_dir_all(&dir);
}
