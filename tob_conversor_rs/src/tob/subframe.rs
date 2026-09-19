//! Sub-frame boundaries inside a main TOB3 frame (CR1000X-style ring layout),
//! and the walk that turns one main frame into rows.

use super::decode::time_ns_from_frame_header;
use super::frame_gate::footer_offset_fits;
use super::header::TobHeader;

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
/// for every record it holds.  Returns the number of rows walked.
///
/// This is the single definition of "how many rows are in this frame and when
/// did each happen" — the TOA5 writer, the Arrow collector and the frame gate's
/// row count all go through it, so they cannot drift apart.  The count is not a
/// constant: a frame carrying a sub-frame boundary spends 16 of its data bytes
/// on that boundary and so holds one record fewer.
pub(crate) fn walk_frame<F>(frame_buf: &[u8], header: &TobHeader, mut on_row: F) -> usize
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
        clock.time_ns += (header.rec_intvl * 1_000_000_000.0) as i64;
    }

    rows
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
            let n = walk_frame(&frame, &h, |_, _, rec| ids.push(rec));
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
        let n = walk_frame(&frame, &h, |_, _, _| {});
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
        let n = walk_frame(&frame, &h, |bytes, _ns, rec| seen.push((bytes.len(), rec)));
        assert_eq!(n, 1);
        assert_eq!(seen, vec![(4, 0)]);
    }

    #[test]
    fn walk_row_count_matches_what_it_yields() {
        let h = header();
        let frame = one_frame_bytes(7, 42, 4660);
        let mut counted = 0usize;
        let n = walk_frame(&frame, &h, |_, _, _| counted += 1);
        assert_eq!(n, counted);
    }
}
