//! Reading TOB3 card fragments end to end: off-by-one stamps, frames that do
//! not start where the header ends, and junk that has neither.
//!
//! These are the defects a 2 GB card image brings that a logger's own file
//! never does. Each one is reproduced here as a synthetic file small enough to
//! live in the repository, with the shape taken from a real one — the file
//! names in the comments are the archive items the shape came from.

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use arrow_array::{Array, TimestampNanosecondArray};
use common::{VAL_STAMP, header_block, one_frame, temp_dir};
use tobconversor::{TobBatchReader, scan_frame_stats};

/// Rows, the first and last timestamp, and what the gate made of the frames.
struct Read {
    rows: usize,
    first_ns: i64,
    last_ns: i64,
    stats: tobconversor::FrameStats,
}

fn read(path: &Path) -> Read {
    let mut reader = TobBatchReader::open(path, false, 1 << 16).expect("open");
    let mut rows = 0usize;
    let mut first_ns = i64::MAX;
    let mut last_ns = i64::MIN;
    for batch in &mut reader {
        let batch = batch.expect("batch");
        rows += batch.num_rows();
        let ts = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .expect("TIMESTAMP column");
        for i in 0..ts.len() {
            first_ns = first_ns.min(ts.value(i));
            last_ns = last_ns.max(ts.value(i));
        }
    }
    let stats = reader.frame_stats();
    // Walking the frames without decoding has to reach the same verdict.
    assert_eq!(
        scan_frame_stats(path).expect("scan"),
        stats,
        "scan_frame_stats disagrees with the decoding read"
    );
    Read {
        rows,
        first_ns,
        last_ns,
        stats,
    }
}

fn write_file(tag: &str, blob: &[u8]) -> (PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("card.dat");
    fs::File::create(&path).unwrap().write_all(blob).unwrap();
    (dir, path)
}

/// Six-line prolog of the shape these card fragments actually have: a 100 MSEC
/// eddy-covariance table in 1008-byte frames, 8 × IEEE4B, so a frame holds 31
/// records and spans 3.1 s.
fn eddy_prolog() -> Vec<u8> {
    format!(
        "\"TOB3\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\r\n\
         \"AC_220\",\"100 MSEC\",\"1008\",\"0\",\"{VAL_STAMP}\",\"Sec100Usec\",\"0\",\"0\",\"0\"\r\n\
         \"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\",\"h\"\r\n\
         \"u\",\"u\",\"u\",\"u\",\"u\",\"u\",\"u\",\"u\"\r\n\
         \"Smp\",\"Smp\",\"Smp\",\"Smp\",\"Smp\",\"Smp\",\"Smp\",\"Smp\"\r\n\
         \"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\"\r\n"
    )
    .into_bytes()
}

/// One 1008-byte frame of that table.
fn eddy_frame(seconds: u32, beg: u32, stamp: u16) -> Vec<u8> {
    let mut f = Vec::with_capacity(1008);
    f.extend_from_slice(&seconds.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&beg.to_le_bytes());
    f.resize(1004, 0);
    f.extend_from_slice(&((stamp as u32) << 16).to_le_bytes());
    f
}

/// `n` genuine frames of the 20-byte, 1 SEC, one-record-per-frame table that
/// `header_block` describes, starting at second `t0` and record `beg0`.
fn run_of(n: u32, t0: u32, beg0: u32, stamp: u16) -> Vec<u8> {
    (0..n)
        .flat_map(|i| one_frame(t0 + i, beg0 + i, stamp))
        .collect()
}

/// EX-1 — a CardConvert / "repair card" fragment writes `val_stamp + 1` into
/// every main footer (`ea321b94…SM_150.dat`). Read strictly the file is empty;
/// the whole run is there.
#[test]
fn a_fragment_whose_footers_are_off_by_one_is_read() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    blob.extend(run_of(40, 1_000, 500, VAL_STAMP + 1));
    let (dir, path) = write_file("offbyone", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 40);
    assert_eq!(r.stats.frames_accepted, 40);
    assert!(r.stats.recovered, "this file only reads on the second pass");
    let _ = fs::remove_dir_all(&dir);
}

/// …and the same run written with the table's own stamp is read on the first
/// pass, without any of the recovery machinery being touched.
#[test]
fn the_same_run_with_the_declared_stamp_needs_no_recovery() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    blob.extend(run_of(40, 1_000, 500, VAL_STAMP));
    let (dir, path) = write_file("clean", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 40);
    assert!(!r.stats.recovered);
    let _ = fs::remove_dir_all(&dir);
}

/// EX-2 — the frames of a re-padded fragment sit a fixed number of bytes past
/// the prolog (`0e95e2ce…AC_221.dat`: half a frame). At the declared base every
/// footer lands mid-record and the file reads as empty.
#[test]
fn a_run_that_does_not_start_at_the_header_end_is_found() {
    for shift in [4usize, 12, 16] {
        let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
        let header_end = blob.len() as u64;
        blob.extend(std::iter::repeat_n(0x20u8, shift));
        blob.extend(run_of(40, 1_000, 500, VAL_STAMP));
        let (dir, path) = write_file(&format!("shift{shift}"), &blob);

        let r = read(&path);
        assert_eq!(r.rows, 40, "shift {shift}");
        assert_eq!(
            r.stats.frame_base,
            header_end + shift as u64,
            "shift {shift}"
        );
        assert!(r.stats.recovered, "shift {shift}");
        let _ = fs::remove_dir_all(&dir);
    }
}

/// EX-2, the other direction — a run far enough into the file that the reader
/// walks hundreds of thousands of junk frames before reaching it
/// (`389ca0df…AC_220.dat`: 396 MB in) still reads, and none of the junk does.
#[test]
fn a_run_far_past_the_header_is_read_and_the_junk_before_it_is_not() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    // Leftovers of some other table: right size, wrong stamp.
    for i in 0..5_000u32 {
        blob.extend(one_frame(
            0xFFFF_0000 ^ i,
            0x3333_0000 ^ i,
            VAL_STAMP ^ 0x0F0F,
        ));
    }
    blob.extend(run_of(40, 1_000, 500, VAL_STAMP + 1));
    let (dir, path) = write_file("faraway", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 40);
    // 1000 s after the CSI epoch, not 1990-01-01 and not 2126.
    assert_eq!(r.first_ns, (1_000 + tobconversor::TO_EPOCH) * 1_000_000_000);
    assert_eq!(r.last_ns, (1_039 + tobconversor::TO_EPOCH) * 1_000_000_000);
    let _ = fs::remove_dir_all(&dir);
}

/// EX-3 — a frame of a 100 MSEC table holds 31 records, so it spans 3.1 s, and
/// a logger that writes whole seconds steps by 3 or by 4 and never by 3.1
/// (`389ca0df…AC_220.dat`: 144,914 gaps of 3 s, 16,101 of 4 s). A reader that
/// wants the exact step rejects the run outright.
#[test]
fn a_run_whose_whole_second_cadence_alternates_is_read() {
    let mut blob = eddy_prolog();
    let frames = 200u32;
    for i in 0..frames {
        // Whole seconds only, as this family of files writes them: the true
        // step is 3.1 s, so the integer stamps go 3, 3, 3, 4, 3, 3, 3, 3, 4…
        let seconds = 967_908_321 + (i as f64 * 3.1).floor() as u32;
        // `val_stamp + 1`, exactly as the real fragments carry it.
        blob.extend(eddy_frame(seconds, 32_077_717 + i * 31, VAL_STAMP + 1));
    }
    let (dir, path) = write_file("cadence", &blob);

    let r = read(&path);
    assert_eq!(r.rows, frames as usize * 31, "every frame of the run");
    assert_eq!(r.stats.rejected_unconfirmed, 0);
    let _ = fs::remove_dir_all(&dir);
}

/// EX-4 — junk that "lines up twice": two leftover frames whose record numbers
/// happen to continue each other. Before the clock test that was enough to put
/// a frame's worth of 1990 rows into a 2025 file
/// (`58f16f26…CT_160.dat`: 50 such rows).
#[test]
fn junk_that_lines_up_on_record_numbers_but_not_in_time_is_not_emitted() {
    let mut blob = eddy_prolog();
    let frames = 60u32;
    for i in 0..frames {
        blob.extend(eddy_frame(
            1_130_494_850 + (i as f64 * 3.1).floor() as u32,
            77_664_892 + i * 31,
            VAL_STAMP,
        ));
    }
    // The pair from `58f16f26…CT_160.dat`, byte for byte in shape: `beg`
    // 1_032_597_859 then exactly 31 later — a perfect ladder — but both frames
    // carry the *same* `seconds`, and the second one would have to start 3.1 s
    // after the first one ends.
    blob.extend(eddy_frame(15_793_989, 1_032_597_859, VAL_STAMP));
    blob.extend(eddy_frame(15_793_989, 1_032_597_859 + 31, VAL_STAMP));
    let (dir, path) = write_file("liar", &blob);

    let r = read(&path);
    assert_eq!(
        r.rows,
        frames as usize * 31,
        "the genuine run and nothing else"
    );
    assert_eq!(r.stats.rejected_unconfirmed, 2);
    assert_eq!(
        r.first_ns,
        (1_130_494_850 + tobconversor::TO_EPOCH) * 1_000_000_000
    );
    let _ = fs::remove_dir_all(&dir);
}

/// EX-5 — a card whose declared table was never written here has scattered
/// footer collisions and no run at all (`94b2da64…ETO_CS.dat`). It must decode
/// to nothing, and say so, rather than emit the collisions or look the same as
/// a file that failed to parse.
#[test]
fn a_card_with_no_run_at_all_decodes_to_nothing_and_says_so() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    // Frame 0 of a ring is the one the logger writes first, so the reader
    // takes it on trust; on a card that was never written for this table it is
    // erased space like everything else, and does not validate.
    for i in 0..600u32 {
        if i > 0 && i % 71 == 0 {
            // A stamp collision, with the garbage record number and epoch-ish
            // timestamp that used to reach the output.
            blob.extend(one_frame(
                i.wrapping_mul(0x9E37_79B9),
                0xABCD_0000 ^ i,
                VAL_STAMP,
            ));
        } else {
            blob.extend(one_frame(i, i, VAL_STAMP ^ 0x0F0F));
        }
    }
    let (dir, path) = write_file("nojob", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 0);
    assert_eq!(r.stats.frames_accepted, 0, "the verdict, without a re-scan");
    assert!(r.stats.frames_read > 0);
    let _ = fs::remove_dir_all(&dir);
}

/// A card image can hold two instances of one table at two byte offsets
/// (`fa1fd3a4…AC_220.dat` holds 2020-11-26 → 12-01 at the header's own
/// alignment and 2020-12-01 → 12-16 sixteen bytes off). Both are the table's
/// and they are consecutive in time, so reading either one alone throws away
/// the other — the recovery pass reads every run it finds, oldest first.
#[test]
fn both_instances_on_a_reused_card_are_read_oldest_first() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    // The *later* instance comes first in the file, as it does on a card whose
    // ring was rewritten in place.
    blob.extend(run_of(40, 900_000, 7_000_000, VAL_STAMP + 1));
    blob.extend(std::iter::repeat_n(0u8, 8));
    // The earlier one, 8 bytes out of step, with the table's own stamp.
    blob.extend(run_of(40, 1_000, 500, VAL_STAMP));
    let (dir, path) = write_file("twoinst", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 80, "both instances, not the larger of the two");
    assert_eq!(r.first_ns, (1_000 + tobconversor::TO_EPOCH) * 1_000_000_000);
    assert_eq!(
        r.last_ns,
        (900_039 + tobconversor::TO_EPOCH) * 1_000_000_000
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Every byte of a recovered file is read at most once, so nothing is emitted
/// twice however many runs claim to overlap.
#[test]
fn a_recovered_file_never_emits_a_row_twice() {
    let mut blob = header_block("CR1000", 20, VAL_STAMP).into_bytes();
    blob.extend(run_of(200, 1_000, 500, VAL_STAMP + 1));
    let (dir, path) = write_file("nodupes", &blob);

    let r = read(&path);
    assert_eq!(r.rows, 200);
    assert_eq!(r.stats.frames_accepted, 200);
    // Frames read during the pass that produced the rows, not the whole file
    // twice over: the first pass emitted nothing, so it is not counted twice.
    assert_eq!(
        r.stats.frames_read,
        200 + 200,
        "one failed pass, then the run"
    );
    let _ = fs::remove_dir_all(&dir);
}
