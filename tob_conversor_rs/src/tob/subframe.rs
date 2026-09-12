//! Sub-frame boundaries inside a main TOB3 frame (CR1000X-style ring layout).

use super::decode::time_ns_from_frame_header;
use super::header::TobHeader;

/// Skip optional padding, `4-byte sub-footer`, and `12-byte sub-header` when the
/// footer matches the table stamp and the sub-header record id matches `next_record_id`.
pub(crate) fn scan_and_skip_subframe_boundary(
    frame_buf: &[u8],
    off: &mut usize,
    data_end: usize,
    header: &TobHeader,
    next_record_id: u32,
    seconds: &mut u32,
    subseconds: &mut u32,
    frame_time_ns: &mut i64,
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
        let stamp_matches = stamp == header.val_stamp
            || stamp == header.comp_val_stamp
            || (stamp & 0xFFF0) == (header.val_stamp & 0xFFF0)
            || (stamp & 0xFFF0) == (header.comp_val_stamp & 0xFFF0);
        if !stamp_matches {
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
        *seconds = s;
        *subseconds = ss;
        *frame_time_ns = time_ns_from_frame_header(s, ss, header.frame_time_res);
        return true;
    }
    false
}

pub(crate) fn is_valid_main_frame(frame_buf: &[u8], header: &TobHeader) -> bool {
    let footer_raw = u32::from_le_bytes(frame_buf[header.frame_nbytes - 4..].try_into().unwrap());
    let footer_validation = (footer_raw >> 16) as u16;
    let flag_e = (footer_raw >> 13) & 1;

    if flag_e != 0 {
        return false;
    }
    footer_validation == header.val_stamp || footer_validation == header.comp_val_stamp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tob::header::parse_tob_header;
    use std::io::Cursor;

    #[test]
    fn valid_frame_footer_matches_stamp_empty_flag_clear() {
        let hdr = r#""TOB3","S","CR1000","1","O","P","G","D"
"T","1 SEC","20","0","4660","SecMsec","0","0","0"
"a","b"
"u","u"
"S","S"
"FP2","FP2"
"#;
        let mut c = Cursor::new(hdr);
        let h = parse_tob_header(&mut c).unwrap();
        let mut frame = one_frame_bytes(1, 1, 4660);
        assert!(is_valid_main_frame(&frame, &h));
        // empty flag (bit 13 of footer word)
        let w = u32::from_le_bytes(frame[16..20].try_into().unwrap());
        frame[16..20].copy_from_slice(&(w | (1 << 13)).to_le_bytes());
        assert!(!is_valid_main_frame(&frame, &h));
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
}
