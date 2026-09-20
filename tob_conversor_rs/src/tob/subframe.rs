//! Sub-frame boundaries inside a main TOB3 frame (CR1000X-style ring layout),
//! and the walk that turns one main frame into rows.

use super::decode::time_ns_from_frame_header;
use super::frame_gate::footer_offset_fits;
use super::header::TobHeader;

/// What one main frame holds: how many records, and where its clock stands
/// once they are walked.
///
/// The two travel together because the frame gate needs both to decide whether
/// the *next* frame continues this one, and only the walk knows either: the
/// record count is not a constant (a frame carrying a sub-frame boundary holds
/// one record fewer) and neither is the clock (a boundary restates it outright).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameWalk {
    /// Records the frame holds.
    pub rows: usize,
    /// Timestamp the record *after* the last one would carry — where a frame
    /// that genuinely continues this one has to begin.
    pub end_time_ns: i64,
}

/// Where in time the next record of a frame sits.
///
/// A sub-frame boundary restates the frame's whole time base, so the raw
/// seconds/subseconds pair and the nanosecond value derived from it are
/// replaced together rather than one at a time.
pub(crate) struct FrameClock {
    pub time_ns: i64,
}

impl FrameClock {
    fn new(seconds: u32, subseconds: u32, frame_time_res: f64) -> Self {
        Self {
            time_ns: time_ns_from_frame_header(seconds, subseconds, frame_time_res),
        }
    }
}

/// True when `word` could be the sub-footer that opens a sub-frame boundary.
///
/// Two independent structural tests, neither fitted to a particular file:
///
/// * the stamp is the table's validation stamp, its complement, or **either one
///   off by one**. Campbell firmware writes off-by-one stamps in both
///   directions — `val_stamp - 1` on the sub-frame boundaries of CR1000 1-minute
///   tables, `val_stamp + 1` on the main footers of CardConvert card fragments.
///   The rule follows that arithmetic, rather than the older `& 0xFFF0` bucket,
///   which covers `val_stamp - 1` only when the low nibble happens not to
///   borrow: for a stamp of `0x6110` it would miss `0x610F` entirely.
/// * the offset field fits inside the frame, the same invariant every main
///   footer obeys.
///
/// Scored against `beg`-derived ground truth over 16,024 frames in 8 files from
/// 6 sites, counting frames whose walked record count disagreed: exact stamp 3,
/// no stamp test 3, offset test alone 1, this rule 0. See
/// `tests/subframe_beg_agreement.rs`, which re-runs that scoring on any file.
fn boundary_word_is_plausible(word: u32, header: &TobHeader) -> bool {
    let stamp = (word >> 16) as u16;
    let near = [header.val_stamp, header.comp_val_stamp]
        .iter()
        .any(|&s| stamp == s || stamp == s.wrapping_sub(1) || stamp == s.wrapping_add(1));
    near && footer_offset_fits(word, header)
}

/// Skip optional padding, `4-byte sub-footer`, and `12-byte sub-header` when the
/// footer looks like the table stamp and the sub-header record id matches
/// `next_record_id`.
///
/// **The record id is the real test, not the stamp.** `next_record_id` is a
/// 32-bit value the caller already knows, so requiring it to match exactly puts
/// the odds of mistaking measurement bytes for a boundary at about 2^-32 on its
/// own. The stamp is a cheap pre-filter in front of it.
///
/// And the stamp genuinely varies: sub-frame boundaries in this archive are
/// routinely written with `val_stamp - 1` (Estancia_chale `CS_131.dat`, where
/// the table stamp is 24855 and every boundary carries 24854), the same
/// off-by-one family as the CardConvert fragments whose *main* frame footers
/// carry `val_stamp + 1`. Demanding an exact match here makes the scanner walk
/// straight past those boundaries, over-count the frame by a record, and
/// desynchronise the record sequence for everything after it — which then looks
/// like a discontinuity and costs real rows.
///
/// So: accept the neighbourhood of the stamp, and let the record id decide.
/// TOB2 has no record id in its sub-header, so there the stamp has to carry the
/// whole check and stays exact.
pub(crate) fn scan_and_skip_subframe_boundary(
    frame_buf: &[u8],
    off: &mut usize,
    data_end: usize,
    header: &TobHeader,
    next_record_id: u32,
    clock: &mut FrameClock,
) -> bool {
    const MAX_PAD_SCAN: usize = 160;

    let gap_len = if header.is_tob2 { 12 } else { 16 };

    let max_delta = MAX_PAD_SCAN.min(data_end.saturating_sub(*off + gap_len));
    for delta in 0..=max_delta {
        let o = *off + delta;
        if o + gap_len > data_end {
            break;
        }
        let word = u32::from_le_bytes(frame_buf[o..o + 4].try_into().unwrap());
        if !boundary_word_is_plausible(word, header) {
            continue;
        }
        if (word >> 13) & 1 != 0 {
            continue;
        }

        if header.is_tob2 {
            // No record id to corroborate with: the stamp must be exact.
            let stamp = (word >> 16) as u16;
            if stamp != header.val_stamp && stamp != header.comp_val_stamp {
                continue;
            }
        } else {
            let hdr_rec = u32::from_le_bytes(frame_buf[o + 12..o + 16].try_into().unwrap());
            if hdr_rec != next_record_id {
                continue;
            }
        }

        let s = u32::from_le_bytes(frame_buf[o + 4..o + 8].try_into().unwrap());
        let ss = u32::from_le_bytes(frame_buf[o + 8..o + 12].try_into().unwrap());

        *off = o + gap_len;
        *clock = FrameClock::new(s, ss, header.frame_time_res);
        return true;
    }
    false
}

/// Walk one main frame, calling `on_row(line_bytes, frame_time_ns, record_id)`
/// for every record it holds.
///
/// This is the single definition of "how many rows are in this frame and when
/// did each happen" — the TOA5 writer, the Arrow collector and the frame gate
/// all go through it, so they cannot drift apart.  Neither half of
/// [`FrameWalk`] is a constant: a frame carrying a sub-frame boundary spends 16
/// of its data bytes on that boundary, so it holds one record fewer, and the
/// boundary restates the frame's time base on top of that.
pub(crate) fn walk_frame<F>(frame_buf: &[u8], header: &TobHeader, mut on_row: F) -> FrameWalk
where
    F: FnMut(&[u8], i64, u32),
{
    let main_beg = if header.is_tob2 {
        0
    } else {
        u32::from_le_bytes(frame_buf[8..12].try_into().unwrap())
    };

    let mut clock = FrameClock::new(
        u32::from_le_bytes(frame_buf[0..4].try_into().unwrap()),
        u32::from_le_bytes(frame_buf[4..8].try_into().unwrap()),
        header.frame_time_res,
    );
    let mut next_record_id = main_beg;

    let data_end = header.frame_nbytes.saturating_sub(4);
    let line_step = header.line_nbytes + header.data_line_padding;
    let mut off = if header.is_tob2 { 8usize } else { 12usize };
    let mut rows = 0usize;

    while off < data_end {
        if scan_and_skip_subframe_boundary(
            frame_buf,
            &mut off,
            data_end,
            header,
            next_record_id,
            &mut clock,
        ) {
            continue;
        }

        if off + header.line_nbytes > data_end {
            break;
        }

        let line_bytes = &frame_buf[off..off + header.line_nbytes];
        off += line_step;

        on_row(line_bytes, clock.time_ns, next_record_id);
        rows += 1;

        next_record_id = next_record_id.wrapping_add(1);
        // Saturating: a junk frame can decode a `seconds` field near 2^32, and
        // stepping past the end of i64 nanoseconds must not panic on the way
        // to rejecting it.
        clock.time_ns = clock
            .time_ns
            .saturating_add((header.rec_intvl * 1_000_000_000.0) as i64);
    }

    FrameWalk {
        rows,
        end_time_ns: clock.time_ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tob::header::parse_tob_header;
    use std::io::Cursor;

    fn header() -> TobHeader {
        let hdr = r#""TOB3","S","CR1000","1","O","P","G","D"
"T","1 SEC","20","0","4660","SecMsec","0","0","0"
"a","b"
"u","u"
"S","S"
"FP2","FP2"
"#;
        parse_tob_header(&mut Cursor::new(hdr)).unwrap()
    }

    fn one_frame_bytes(seconds: u32, beg: u32, stamp: u16) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&seconds.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&beg.to_le_bytes());
        f.extend_from_slice(&[0x45u8, 0x8e, 0x45, 0x8e]);
        f.extend_from_slice(&((stamp as u32) << 16).to_le_bytes());
        f
    }

    /// Frame with one record, a sub-frame boundary, then one more record.
    /// `boundary_stamp` lets a test write the `val_stamp - 1` that real
    /// loggers use. 36 bytes: 12 header + 2 + 16 + 2 + 4 footer.
    fn frame_with_boundary(beg: u32, boundary_stamp: u16) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&100u32.to_le_bytes()); // seconds
        f.extend_from_slice(&0u32.to_le_bytes()); // subseconds
        f.extend_from_slice(&beg.to_le_bytes()); // beg record
        f.extend_from_slice(&[0x45, 0x8e]); // record `beg`
        f.extend_from_slice(&((boundary_stamp as u32) << 16).to_le_bytes()); // sub-footer
        f.extend_from_slice(&200u32.to_le_bytes()); // sub seconds (new time base)
        f.extend_from_slice(&0u32.to_le_bytes()); // sub subseconds
        f.extend_from_slice(&(beg + 1).to_le_bytes()); // sub-header record id
        f.extend_from_slice(&[0x45, 0x8e]); // record `beg + 1`
        f.extend_from_slice(&((4660u32) << 16).to_le_bytes()); // main footer
        assert_eq!(f.len(), 36);
        f
    }

    fn header_36() -> TobHeader {
        let hdr = r#""TOB3","S","CR1000","1","O","P","G","D"
"T","1 SEC","36","0","4660","SecMsec","0","0","0"
"a"
"u"
"S"
"FP2"
"#;
        parse_tob_header(&mut Cursor::new(hdr)).unwrap()
    }

    /// The boundary stamp in this archive is routinely `val_stamp - 1`
    /// (Estancia_chale CS_131.dat: table stamp 24855, every boundary 24854).
    /// Missing it makes the walker read the 16 boundary bytes as data, so the
    /// frame reports more records than it holds and every later frame looks
    /// like a discontinuity.
    #[test]
    fn subframe_boundary_is_found_when_its_stamp_is_off_by_one() {
        let h = header_36();
        for stamp in [4659u16, 4660, 4661] {
            let frame = frame_with_boundary(27_698, stamp);
            let mut ids = Vec::new();
            let n = walk_frame(&frame, &h, |_, _, rec| ids.push(rec)).rows;
            assert_eq!(n, 2, "stamp {stamp}: expected 2 records, walked {n}");
            assert_eq!(ids, vec![27_698, 27_699], "stamp {stamp}");
        }
    }

    /// The record id is what actually confirms a boundary: a stamp in range but
    /// a record id that does not continue the frame is not a boundary.
    #[test]
    fn a_near_stamp_with_the_wrong_record_id_is_not_a_boundary() {
        let h = header_36();
        let mut frame = frame_with_boundary(27_698, 4659);
        // Break only the sub-header record id.
        frame[26..30].copy_from_slice(&9_999_999u32.to_le_bytes());
        let n = walk_frame(&frame, &h, |_, _, _| {}).rows;
        assert!(
            n > 2,
            "boundary must not be taken on the stamp alone (got {n})"
        );
    }

    /// The sub-frame boundary restates the frame clock.
    #[test]
    fn crossing_a_boundary_resets_the_frame_clock() {
        let h = header_36();
        let frame = frame_with_boundary(27_698, 4659);
        let mut times = Vec::new();
        walk_frame(&frame, &h, |_, ns, _| times.push(ns));
        assert_eq!(times.len(), 2);
        // 100 s then 200 s from the Campbell epoch, not 100 s then 101 s.
        assert_eq!(times[1] - times[0], 100 * 1_000_000_000);
    }

    #[test]
    fn walk_counts_the_records_in_a_frame() {
        let h = header();
        let frame = one_frame_bytes(1, 0, 4660);
        // 20-byte frame: 12 header + 4 data + 4 footer = one 4-byte record.
        let mut seen = Vec::new();
        let n = walk_frame(&frame, &h, |bytes, _ns, rec| seen.push((bytes.len(), rec))).rows;
        assert_eq!(n, 1);
        assert_eq!(seen, vec![(4, 0)]);
    }

    #[test]
    fn walk_row_count_matches_what_it_yields() {
        let h = header();
        let frame = one_frame_bytes(7, 42, 4660);
        let mut counted = 0usize;
        let n = walk_frame(&frame, &h, |_, _, _| counted += 1).rows;
        assert_eq!(n, counted);
    }

    /// The walk reports where its clock ended, and a sub-frame boundary is
    /// folded into that. The frame gate compares the next frame's start
    /// against this number, so getting it from the raw frame header instead
    /// would make every boundary-carrying frame look like a time jump.
    #[test]
    fn the_walk_reports_the_clock_after_the_last_record() {
        let h = header_36();
        // Boundary restates the base to 200 s; one record follows it at 1 SEC.
        let walk = walk_frame(&frame_with_boundary(27_698, 4659), &h, |_, _, _| {});
        assert_eq!(walk.rows, 2);
        assert_eq!(
            walk.end_time_ns,
            time_ns_from_frame_header(201, 0, h.frame_time_res)
        );
    }

    /// EX-6: a TOB2 sub-header carries no record id, so the stamp is the whole
    /// test there and has to stay exact. This is the guard rail for that: if
    /// the off-by-one neighbourhood ever reaches the TOB2 branch, a boundary
    /// gets taken on a coincidence with nothing to corroborate it.
    #[test]
    fn tob2_boundaries_still_demand_an_exact_stamp() {
        let hdr = r#""TOB2","S","CR1000","1","O","P","G","D"
"T","1 SEC","28","0","4660","SecMsec","0","0","0"
"a"
"u"
"S"
"FP2"
"#;
        let h = parse_tob_header(&mut Cursor::new(hdr)).unwrap();
        assert!(h.is_tob2);
        // 28 bytes: 8 header + 12 boundary + 2 data + 2 data + 4 footer.
        let tob2_frame = |boundary_stamp: u16| {
            let mut f = Vec::new();
            f.extend_from_slice(&100u32.to_le_bytes()); // seconds
            f.extend_from_slice(&0u32.to_le_bytes()); // subseconds
            f.extend_from_slice(&((boundary_stamp as u32) << 16).to_le_bytes()); // sub-footer
            f.extend_from_slice(&200u32.to_le_bytes()); // restated seconds
            f.extend_from_slice(&0u32.to_le_bytes()); // restated subseconds
            f.extend_from_slice(&[0x45, 0x8e]); // one record
            f.extend_from_slice(&[0x45, 0x8e]); // one more record
            f.extend_from_slice(&(4660u32 << 16).to_le_bytes()); // main footer
            assert_eq!(f.len(), 28);
            f
        };
        // The table's own stamp is taken as a boundary: two records on the
        // restated time base, not on the one the frame header declares.
        let mut times = Vec::new();
        let exact = walk_frame(&tob2_frame(4660), &h, |_, ns, _| times.push(ns));
        assert_eq!(exact.rows, 2);
        assert_eq!(
            times[0],
            time_ns_from_frame_header(200, 0, h.frame_time_res)
        );
        // One off is not: the 12 boundary bytes are read as measurements, so
        // the frame reports far more records than it holds.
        assert_eq!(walk_frame(&tob2_frame(4659), &h, |_, _, _| {}).rows, 8);
    }
}
