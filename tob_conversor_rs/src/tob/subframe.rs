//! Sub-frame boundaries inside a main TOB3 frame (CR1000X-style ring layout),
//! and the walk that turns one main frame into rows.

use super::decode::time_ns_from_frame_header;
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

/// Skip optional padding, `4-byte sub-footer`, and `12-byte sub-header` when the
/// footer matches the table stamp and the sub-header record id matches `next_record_id`.
///
/// The stamp must match **exactly**.  An earlier version accepted any stamp in
/// the same 16-value bucket (`& 0xFFF0`), which multiplied the odds that a run
/// of ordinary measurement bytes would be mistaken for a boundary by 16 — and a
/// phantom boundary is expensive: it resets the frame clock from data bytes and
/// shifts every following row in the frame by 16 bytes, so every column reads
/// one field late.  Files whose frames carry a `stamp + 1` footer (CardConvert
/// card fragments) are handled by rewriting the header stamp, not by loosening
/// this test.
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
        let stamp = (word >> 16) as u16;
        if stamp != header.val_stamp && stamp != header.comp_val_stamp {
            continue;
        }
        if (word >> 13) & 1 != 0 {
            continue;
        }

        if !header.is_tob2 {
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
