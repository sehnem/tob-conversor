//! Additional datalogger models (CR800, CR3000, CR6) and FP2 NaN threshold behavior.

mod common;

use std::fs;
use std::io::Write;

use common::{
    VAL_STAMP, header_block_with_types, one_frame_with_payload, temp_dir, write_synthetic_tob3,
};
use tobconversor::convert_streaming;

#[test]
fn cr800_roundtrip() {
    let dir = temp_dir("cr800");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("cr800.dat");
    write_synthetic_tob3(&input, "CR800");
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert");
    assert!(n >= 1);
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("cr800_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    assert!(text.contains("CR800"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cr3000_roundtrip() {
    let dir = temp_dir("cr3000");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("cr3000.dat");
    write_synthetic_tob3(&input, "CR3000");
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert");
    assert!(n >= 1);
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("cr3000_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    assert!(text.contains("CR3000"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cr6_roundtrip() {
    let dir = temp_dir("cr6");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("cr6.dat");
    write_synthetic_tob3(&input, "CR6");
    let out = dir.join("out");
    let n = convert_streaming(&input, &out, 30, false).expect("convert");
    assert!(n >= 1);
    let p = fs::read_dir(&out)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("cr6_")
        })
        .unwrap();
    let text = fs::read_to_string(p).unwrap();
    assert!(text.contains("CR6"));
    let _ = fs::remove_dir_all(&dir);
}

/// FP2 raw 7500: CR10X → NaN (threshold 6999); CR1000 → numeric (7999).
#[test]
fn cr10x_fp2_nan_threshold() {
    let payload = [0x1Du8, 0x4C, 0x45, 0x8e]; // 7500, ~14.22 V

    let dir10 = temp_dir("cr10x_nan");
    let _ = fs::remove_dir_all(&dir10);
    fs::create_dir_all(&dir10).unwrap();
    let input10 = dir10.join("nan.dat");
    let hdr10 = header_block_with_types(
        "CR10X", 20, VAL_STAMP, "SecMsec", "A,B", "V,V", "Smp,Smp", "FP2,FP2",
    );
    let mut blob10 = hdr10.into_bytes();
    blob10.extend(one_frame_with_payload(&payload, 100, 1, VAL_STAMP));
    fs::File::create(&input10)
        .unwrap()
        .write_all(&blob10)
        .unwrap();
    let out10 = dir10.join("out");
    convert_streaming(&input10, &out10, 30, false).unwrap();
    let p10 = fs::read_dir(&out10)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("nan_")
        })
        .unwrap();
    let text10 = fs::read_to_string(p10).unwrap();
    assert!(
        text10.contains("\"NAN\""),
        "CR10X should decode 7500 FP2 as NaN: {text10}"
    );

    let dir1k = temp_dir("cr1000_num");
    let _ = fs::remove_dir_all(&dir1k);
    fs::create_dir_all(&dir1k).unwrap();
    let input1k = dir1k.join("num.dat");
    let hdr1k = header_block_with_types(
        "CR1000", 20, VAL_STAMP, "SecMsec", "A,B", "V,V", "Smp,Smp", "FP2,FP2",
    );
    let mut blob1k = hdr1k.into_bytes();
    blob1k.extend(one_frame_with_payload(&payload, 100, 1, VAL_STAMP));
    fs::File::create(&input1k)
        .unwrap()
        .write_all(&blob1k)
        .unwrap();
    let out1k = dir1k.join("out");
    convert_streaming(&input1k, &out1k, 30, false).unwrap();
    let p1k = fs::read_dir(&out1k)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|x| x == "dat").unwrap_or(false)
                && p.file_name().unwrap().to_string_lossy().starts_with("num_")
        })
        .unwrap();
    let text1k = fs::read_to_string(p1k).unwrap();
    assert!(
        text1k.contains("7500"),
        "CR1000 should keep 7500 as a number: {text1k}"
    );
    assert!(
        !text1k.contains("\"NAN\""),
        "CR1000 should not emit NaN for 7500: {text1k}"
    );

    let _ = fs::remove_dir_all(&dir10);
    let _ = fs::remove_dir_all(&dir1k);
}
