//! Optional smoke test with a real `.dat` on disk (`TOB3_TEST_FILE`).

use std::fs;
use std::path::PathBuf;

use tobconversor::convert_streaming;

#[test]
fn optional_real_file_smoke() {
    let Some(path) = std::env::var_os("TOB3_TEST_FILE").map(PathBuf::from) else {
        return;
    };
    if !path.is_file() {
        eprintln!("TOB_TEST_FILE set but not a file: {:?}", path);
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "tob3_real_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let n = convert_streaming(&path, &dir, 30, false).expect("real file convert");
    assert!(n >= 1, "real file should produce ≥1 output");
    let _ = fs::remove_dir_all(&dir);
}
