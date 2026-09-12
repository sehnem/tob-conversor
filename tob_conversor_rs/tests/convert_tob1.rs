//! TOB1 format: frameless sequential records with embedded timestamps.

mod common;

use std::fs;
use std::io::Write;

use common::{header_block_tob1, one_tob1_record, temp_dir};
use tobconversor::convert_streaming;

/// FP2 encoding for ~14.22 V battery voltage (same sentinel used in other tests).
const FP2_BATT: [u8; 2] = [0x45, 0x8e];

#[test]
fn tob1_three_records_produces_three_data_rows() {
    let dir = temp_dir("tob1_3rows");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("test.dat");

    let data: Vec<u8> = [FP2_BATT, FP2_BATT].concat();
    let mut blob = header_block_tob1("CR1000").into_bytes();
    blob.extend(one_tob1_record(100, 0, 1, &data));
    blob.extend(one_tob1_record(101, 0, 2, &data));
    blob.extend(one_tob1_record(102, 0, 3, &data));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();

    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert TOB1");
    assert_eq!(n, 1, "3 records in same 30-min window → 1 output file");

    let outs: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "dat").unwrap_or(false))
        .collect();
    assert_eq!(outs.len(), 1);

    let text = fs::read_to_string(&outs[0]).unwrap();
    let data_lines: Vec<_> = text.lines().skip(4).collect();
    assert_eq!(
        data_lines.len(),
        3,
        "expected 3 data rows, got: {:?}",
        data_lines
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tob1_timestamp_at_epoch_zero_is_correct() {
    let dir = temp_dir("tob1_ts");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("ts.dat");

    let data: Vec<u8> = [FP2_BATT, FP2_BATT].concat();
    let mut blob = header_block_tob1("CR1000").into_bytes();
    // seconds=0 from the CSI epoch (1990-01-01 00:00:00) → output "1990-01-01 00:00:00"
    blob.extend(one_tob1_record(0, 0, 1, &data));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();

    let out = dir.join("out");
    convert_streaming(&input, &out, 30, false).expect("convert TOB1");

    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "dat").unwrap_or(false))
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    let data_row = text.lines().nth(4).expect("data row");
    assert!(
        data_row.contains("1990-01-01 00:00:00"),
        "seconds=0 should yield 1990-01-01 00:00:00, got: {data_row}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tob1_splits_across_30min_boundary() {
    let dir = temp_dir("tob1_split");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("split.dat");

    let data: Vec<u8> = [FP2_BATT, FP2_BATT].concat();
    let mut blob = header_block_tob1("CR1000").into_bytes();
    blob.extend(one_tob1_record(100, 0, 1, &data));
    // 2000 s later = 33+ min → different 30-min bucket
    blob.extend(one_tob1_record(100 + 2000, 0, 2, &data));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();

    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert TOB1");
    assert_eq!(n, 2, "records in different 30-min windows → 2 output files");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tob1_partial_record_at_eof_does_not_panic() {
    let dir = temp_dir("tob1_eof");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("eof.dat");

    let data: Vec<u8> = [FP2_BATT, FP2_BATT].concat();
    let mut blob = header_block_tob1("CR1000").into_bytes();
    blob.extend(one_tob1_record(100, 0, 1, &data));
    // Append 6 bytes of a truncated record (record_size = 12 + 4 = 16, so 6 < 16)
    blob.extend_from_slice(&[0u8; 6]);
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();

    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("should not error on partial EOF");
    assert_eq!(n, 1, "partial trailing record should be silently dropped");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tob1_include_record_column() {
    let dir = temp_dir("tob1_rec");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("rec.dat");

    let data: Vec<u8> = [FP2_BATT, FP2_BATT].concat();
    let mut blob = header_block_tob1("CR1000").into_bytes();
    blob.extend(one_tob1_record(100, 0, 42, &data));
    fs::File::create(&input).unwrap().write_all(&blob).unwrap();

    let out = dir.join("out");
    convert_streaming(&input, &out, 30, true).expect("convert TOB1 with record");

    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "dat").unwrap_or(false))
        .unwrap();
    let text = fs::read_to_string(p).unwrap();

    let col_names = text.lines().nth(1).unwrap();
    assert!(
        col_names.contains("\"RECORD\""),
        "RECORD column missing from header: {col_names}"
    );

    let data_row = text.lines().nth(4).unwrap();
    let fields: Vec<&str> = data_row.split(',').collect();
    assert_eq!(
        fields[1], "42",
        "RECORD value should be 42, got: {data_row}"
    );

    let _ = fs::remove_dir_all(&dir);
}
