//! Roundtrip tests for CR1000, CR1000X, and CR10X (original synthetic layout).

mod common;

use std::fs;

use common::write_synthetic_tob3;
use tobconversor::convert_streaming;

#[test]
fn synthetic_cr1000_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "tob3_int_cr1000_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("minimal.dat");
    write_synthetic_tob3(&input, "CR1000");
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert");
    assert!(n >= 1, "expected at least one split output file");

    let outs: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("minimal_")
        })
        .collect();
    assert_eq!(outs.len(), 1, "single row should land in one 30-min file");
    let text = fs::read_to_string(&outs[0]).unwrap();
    assert!(text.contains("\"TIMESTAMP\""));
    assert!(text.contains("\"A\""));
    assert!(text.contains("\"B\""));
    assert!(!text.lines().nth(1).unwrap().contains("RECORD"));
    let data_line = text
        .lines()
        .find(|l| l.contains("14.219999") || l.contains("14.22"))
        .expect("data row with FP2 battery-like values");
    assert!(data_line.starts_with('"'));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn synthetic_cr1000x_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "tob3_int_cr1000x_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("x.dat");
    write_synthetic_tob3(&input, "CR1000X");
    let out = dir.join("outx");
    let n = convert_streaming(&input, &out, 30, true).expect("convert");
    assert!(n >= 1);
    let outs: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|e| e == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("x_")
        })
        .collect();
    let text = fs::read_to_string(&outs[0]).unwrap();
    assert!(text.lines().nth(1).unwrap().contains("RECORD"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn synthetic_cr10x_uses_fp2_threshold_in_header_only() {
    let dir = std::env::temp_dir().join(format!(
        "tob3_int_cr10x_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("c10.dat");
    write_synthetic_tob3(&input, "CR10X");
    let out = dir.join("out10");
    convert_streaming(&input, &out, 30, false).unwrap();
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("c10_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    assert!(text.contains("CR10X"));
    let _ = fs::remove_dir_all(&dir);
}
