//! Check the frame walk against each file's own ground truth.
//!
//! A TOB3 frame header carries `beg`, the record number of its first record, so
//! inside a contiguous run of written frames the number of records a frame
//! really holds is `next.beg - this.beg`. That is not a guess and not a
//! convention — it is what the logger wrote. The walk, by contrast, *derives* a
//! record count by stepping over the data segment and skipping sub-frame
//! boundaries, and it is wrong whenever it misses one.
//!
//! So this test does not assert a number baked in from some sample file. It
//! asserts that the walk agrees with `beg`, on whatever file you point it at:
//!
//! ```text
//! TOB3_BEG_CHECK=/path/to/a.dat:/path/to/b.dat cargo test --test subframe_beg_agreement
//! TOB3_BEG_CHECK=/path/to/a/directory          cargo test --test subframe_beg_agreement
//! ```
//!
//! Without the variable it is a no-op, like `convert_real_file_smoke`. Real
//! logger files cannot live in the repository, but the invariant travels with
//! anyone who has them: point it at a new site's card images and a stamp
//! variant this reader has never seen shows up as a failure rather than as
//! quietly missing rows.

use std::fs;
use std::path::{Path, PathBuf};

use tobconversor::{TobBatchReader, parse_tob_header};

fn targets() -> Vec<PathBuf> {
    let Some(raw) = std::env::var_os("TOB3_BEG_CHECK") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in raw.to_string_lossy().split(':').filter(|p| !p.is_empty()) {
        let p = Path::new(part);
        if p.is_dir() {
            if let Ok(entries) = fs::read_dir(p) {
                out.extend(
                    entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_file()),
                );
            }
        } else if p.is_file() {
            out.push(p.to_path_buf());
        }
    }
    out.sort();
    out
}

/// Frame headers of the leading run of frames whose footer validates.
///
/// Stops at the first frame that does not, which is where a pre-allocated ring
/// stops being written and starts being whatever was on the card before.
fn written_run(path: &Path) -> Option<(Vec<u32>, usize)> {
    let bytes = fs::read(path).ok()?;
    let header = parse_tob_header(&mut std::io::Cursor::new(&bytes[..])).ok()?;
    if header.is_tob1 || header.is_tob2 || header.frame_nbytes == 0 {
        return None;
    }
    let start = {
        let mut off = 0usize;
        for _ in 0..6 {
            off += bytes[off..].iter().position(|&b| b == b'\n')? + 1;
        }
        off
    };

    let fnb = header.frame_nbytes;
    let capacity = fnb.saturating_sub(16);
    let mut begs = Vec::new();
    let mut off = start;
    while off + fnb <= bytes.len() {
        let footer = u32::from_le_bytes(bytes[off + fnb - 4..off + fnb].try_into().unwrap());
        let stamp = (footer >> 16) as u16;
        let ok = (stamp == header.val_stamp || stamp == header.comp_val_stamp)
            && footer & (1 << 13) == 0
            && (footer & 0x7FF) as usize <= capacity;
        if !ok {
            break;
        }
        begs.push(u32::from_le_bytes(
            bytes[off + 8..off + 12].try_into().unwrap(),
        ));
        off += fnb;
    }
    Some((begs, fnb))
}

#[test]
fn walked_record_counts_match_the_begs_the_logger_wrote() {
    let files = targets();
    if files.is_empty() {
        return;
    }

    let mut checked_files = 0usize;
    let mut checked_frames = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for path in &files {
        let Some((begs, _)) = written_run(path) else {
            continue;
        };
        if begs.len() < 2 {
            continue;
        }

        // What the walk produced, frame by frame: RECORD is the logger's own
        // record number, so the first record of each emitted frame is its `beg`.
        let Ok(mut reader) = TobBatchReader::open(path, true, 1 << 16) else {
            continue;
        };
        let mut walked: Vec<i64> = Vec::new();
        for batch in &mut reader {
            let Ok(batch) = batch else { break };
            let col = batch.column(1);
            let arr = col
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .expect("RECORD column is Int64");
            walked.extend(arr.values().iter().copied());
        }

        // Records the logger says are in frames [0, n-1).
        let expected_total = (begs[begs.len() - 1] - begs[0]) as usize;
        let emitted_before_last = walked
            .iter()
            .position(|&r| r as u32 == begs[begs.len() - 1])
            .unwrap_or(walked.len());

        checked_files += 1;
        checked_frames += begs.len();

        if emitted_before_last != expected_total {
            failures.push(format!(
                "{}: walked {} records before beg {}, but the frame headers say {} \
                 (difference {}) — the walk is missing or inventing sub-frame boundaries",
                path.display(),
                emitted_before_last,
                begs[begs.len() - 1],
                expected_total,
                emitted_before_last as i64 - expected_total as i64,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} file(s) disagree with their own frame headers:\n{}",
        failures.len(),
        checked_files,
        failures.join("\n")
    );
    eprintln!("beg agreement: {checked_files} file(s), {checked_frames} frame(s), all agree");
}
