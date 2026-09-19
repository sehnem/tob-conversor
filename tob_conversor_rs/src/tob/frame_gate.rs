//! Which main frames of a TOB2/TOB3 ring file actually belong to the table.
//!
//! A TOB3 file is a **pre-allocated ring buffer**: the logger reserves
//! `intended table size` records up front and only ever overwrites frames in
//! place.  Everything past the write pointer is whatever happened to be on the
//! card before — frames of a *different* table, an older program, or plain
//! flash garbage.  Those bytes are not data and must never reach the output.
//!
//! The only per-frame marker is the 16-bit validation stamp in the frame
//! footer, so a stamp-only test misfires roughly once per 2^17 junk frames.
//! That sounds rare until you meet a 2 GB card image: ~2 M junk frames, ~14
//! false positives, each decoding its first 12 bytes as `seconds` /
//! `subseconds` / `beg record` and emitting a frame's worth of rows stamped
//! anywhere between 1990-01-01 (seconds ≈ 0) and 2126-02-12 (seconds ≈ 2^32).
//! A single such row is enough to wreck a min/max, a partition key or a plot.
//!
//! This module adds the two checks that make the test selective:
//!
//! 1. **Full footer validation** — not just the stamp and the `E` flag, but
//!    the offset field, which on a real frame is either 0 or the small
//!    sub-frame boundary overhead, and never exceeds the frame's data
//!    capacity.
//! 2. **Record-number continuity** ([`FrameGate`], TOB3 only) — a frame whose
//!    `beg record` does not continue the previous frame is held back until the
//!    *next* frame confirms it.  Junk never gets confirmed; a genuine
//!    discontinuity (logger restart, ring wrap) is confirmed by the frame that
//!    follows it, so real data is not lost.
//!
//! Nothing here guesses or repairs a value: a frame is either emitted verbatim
//! or rejected and counted in [`FrameStats`].

use super::header::TobHeader;

/// Footer bit layout of a TOB2/TOB3 main frame (4 bytes, little-endian).
mod footer {
    /// Bytes of sub-frame boundary consumed inside this frame (bits 0-10).
    pub const OFFSET_MASK: u32 = 0x7FF;
    /// `F` — file mark.
    pub const FLAG_FILE_MARK: u32 = 1 << 11;
    /// `R` — remove mark.
    pub const FLAG_REMOVE_MARK: u32 = 1 << 12;
    /// `E` — frame is empty (never written).
    pub const FLAG_EMPTY: u32 = 1 << 13;
    /// `M` — frame carries a minor (sub) frame boundary.
    pub const FLAG_MINOR: u32 = 1 << 14;
    /// Set alongside `M` by CR1000/CR6 firmware on minor frames.
    pub const FLAG_MINOR_HI: u32 = 1 << 15;
}

/// How many frames were accepted, and why the others were not.
///
/// Rejections are *counted*, never silent: a caller that cares (an ingest
/// pipeline logging provenance, say) can report exactly how much of a file was
/// unwritten ring space versus real data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Main frames read from the file.
    pub frames_read: u64,
    /// Frames emitted as data.
    pub frames_accepted: u64,
    /// Frames whose footer is not a valid main-frame footer for this table.
    pub rejected_footer: u64,
    /// Footer-valid frames whose record number never lined up with a neighbour.
    pub rejected_unconfirmed: u64,
}

/// True when `frame_buf`'s footer is a well-formed main-frame footer for this
/// table.
///
/// Checks, in order: the validation stamp matches the table exactly; the `E`
/// (empty) flag is clear; the offset field fits inside the frame's data
/// segment; and a non-zero offset only ever appears on a frame that also flags
/// itself as carrying a minor frame.
pub fn is_valid_main_frame(frame_buf: &[u8], header: &TobHeader) -> bool {
    if header.frame_nbytes < 4 || frame_buf.len() < header.frame_nbytes {
        return false;
    }
    let footer = u32::from_le_bytes(
        frame_buf[header.frame_nbytes - 4..header.frame_nbytes]
            .try_into()
            .unwrap(),
    );

    let stamp = (footer >> 16) as u16;
    if stamp != header.val_stamp && stamp != header.comp_val_stamp {
        return false;
    }
    if footer & footer::FLAG_EMPTY != 0 {
        return false;
    }

    // The data segment: frame bytes minus the frame header and the footer.
    let header_size = if header.is_tob2 { 8 } else { 12 };
    let capacity = header.frame_nbytes.saturating_sub(header_size + 4);

    let offset = (footer & footer::OFFSET_MASK) as usize;
    if offset > capacity {
        // Junk: a real offset counts bytes *inside* this frame.
        return false;
    }
    if offset != 0 && footer & (footer::FLAG_MINOR | footer::FLAG_MINOR_HI) == 0 {
        // A byte count with nothing to count.
        return false;
    }
    let _ = (footer::FLAG_FILE_MARK, footer::FLAG_REMOVE_MARK);
    true
}

/// The record number of the first record in a TOB3 frame (`beg`).
///
/// TOB2 frames carry no record number, so this returns `None` for them.
fn frame_beg_record(frame_buf: &[u8], header: &TobHeader) -> Option<u32> {
    if header.is_tob2 || frame_buf.len() < 12 {
        return None;
    }
    Some(u32::from_le_bytes(frame_buf[8..12].try_into().unwrap()))
}

/// What the caller should emit for the frame it just read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Emit nothing — the frame was rejected, or is being held for confirmation.
    Skip,
    /// Emit the frame that was just read.
    Current,
    /// Emit the held frame first (see [`FrameGate::take_held`]), then the current one.
    HeldThenCurrent,
}

/// Running record-number continuity check over a TOB3 file's main frames.
///
/// Feed every frame through [`admit`](Self::admit) and tell the gate how many
/// rows each emitted frame actually produced via [`advance`](Self::advance) —
/// the row count is what the next frame's `beg record` must match, and only
/// the emitting walk knows it (a frame carrying a minor-frame boundary holds
/// one row fewer).
pub struct FrameGate {
    /// Record number the next genuine frame must start at.
    expected_beg: Option<u32>,
    /// `beg` of the frame most recently emitted.
    last_beg: u32,
    /// A footer-valid frame at a discontinuity, awaiting confirmation.
    held: Option<(u32, usize, Vec<u8>)>,
    /// Nothing has been emitted yet, so the next valid frame anchors the run.
    anchored: bool,
    stats: FrameStats,
}

impl FrameGate {
    pub fn new() -> Self {
        Self {
            expected_beg: None,
            last_beg: 0,
            held: None,
            anchored: false,
            stats: FrameStats::default(),
        }
    }

    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Decide what to do with the frame just read.
    ///
    /// `row_count` is called at most once, and only for a frame that lands at a
    /// discontinuity: it must return how many rows that frame would emit.
    pub fn admit<F>(&mut self, frame_buf: &[u8], header: &TobHeader, row_count: F) -> Admit
    where
        F: FnOnce(&[u8]) -> usize,
    {
        self.stats.frames_read += 1;

        if !is_valid_main_frame(frame_buf, header) {
            self.stats.rejected_footer += 1;
            return Admit::Skip;
        }

        let beg = match frame_beg_record(frame_buf, header) {
            // TOB2: no record numbers, the footer check is all we have.
            None => {
                self.anchored = true;
                self.stats.frames_accepted += 1;
                return Admit::Current;
            }
            Some(b) => b,
        };

        // The first valid frame anchors the run. A logger fills its ring from
        // the front, so frame 0 is always real data; it is the frames *after*
        // the write pointer that are leftovers, and those have to earn their
        // place.
        if !self.anchored || self.expected_beg == Some(beg) {
            self.drop_held();
            self.anchored = true;
            self.last_beg = beg;
            self.stats.frames_accepted += 1;
            return Admit::Current;
        }

        // Discontinuity, or the first frame of the file. Either way this frame
        // is unproven on its own — park it until the next one corroborates it.
        if let Some((held_beg, held_rows, _)) = self.held.as_ref() {
            if beg == held_beg.wrapping_add(*held_rows as u32) {
                self.last_beg = beg;
                self.stats.frames_accepted += 2;
                return Admit::HeldThenCurrent;
            }
            self.stats.rejected_unconfirmed += 1;
        }
        self.held = Some((beg, row_count(frame_buf), frame_buf.to_vec()));
        Admit::Skip
    }

    /// Take the held frame's bytes after [`Admit::HeldThenCurrent`].
    pub fn take_held(&mut self) -> Option<Vec<u8>> {
        self.held.take().map(|(_, _, bytes)| bytes)
    }

    /// Record how many rows the frame just emitted produced, fixing the record
    /// number the next frame has to start at.
    pub fn advance(&mut self, rows_emitted: usize) {
        self.expected_beg = Some(self.last_beg.wrapping_add(rows_emitted as u32));
    }

    /// Finish the file: a frame still awaiting confirmation was never
    /// corroborated and is not emitted.
    pub fn finish(&mut self) {
        self.drop_held();
    }

    fn drop_held(&mut self) {
        if self.held.take().is_some() {
            self.stats.rejected_unconfirmed += 1;
        }
    }
}

impl Default for FrameGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tob::header::parse_tob_header;
    use std::io::Cursor;

    /// TOB3, frame_nbytes = 44, one FP2 column (2 bytes), stamp 4660.
    /// Data segment = 44 - 12 - 4 = 28 bytes = 14 rows.
    fn header() -> TobHeader {
        let hdr = "\"TOB3\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                   \"T\",\"1 SEC\",\"44\",\"0\",\"4660\",\"SecMsec\",\"0\",\"0\",\"0\"\n\
                   \"a\"\n\"u\"\n\"Smp\"\n\"FP2\"\n";
        parse_tob_header(&mut Cursor::new(hdr)).unwrap()
    }

    fn frame(seconds: u32, beg: u32, footer: u32) -> Vec<u8> {
        let mut f = Vec::with_capacity(44);
        f.extend_from_slice(&seconds.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&beg.to_le_bytes());
        f.resize(40, 0);
        f.extend_from_slice(&footer.to_le_bytes());
        f
    }

    const STAMP: u32 = 4660 << 16;

    #[test]
    fn plain_major_frame_is_valid() {
        assert!(is_valid_main_frame(&frame(1, 0, STAMP), &header()));
    }

    #[test]
    fn empty_flag_rejects() {
        let f = frame(1, 0, STAMP | (1 << 13));
        assert!(!is_valid_main_frame(&f, &header()));
    }

    #[test]
    fn wrong_stamp_rejects() {
        assert!(!is_valid_main_frame(&frame(1, 0, 4661 << 16), &header()));
    }

    #[test]
    fn minor_frame_with_small_offset_is_valid() {
        // Real CR1000/CR6 minor frame: M | MINOR_HI, offset = 16 boundary bytes.
        let f = frame(1, 0, STAMP | (1 << 15) | (1 << 14) | 16);
        assert!(is_valid_main_frame(&f, &header()));
    }

    #[test]
    fn offset_larger_than_the_frame_rejects() {
        // The shape of a real false positive: stamp collides, offset is nonsense.
        let f = frame(0xDEAD_BEEF, 0x1234_5678, STAMP | (1 << 14) | 1751);
        assert!(!is_valid_main_frame(&f, &header()));
    }

    #[test]
    fn offset_without_the_minor_flag_rejects() {
        assert!(!is_valid_main_frame(&frame(1, 0, STAMP | 16), &header()));
    }

    #[test]
    fn gate_emits_a_contiguous_run_from_the_first_frame() {
        let h = header();
        let mut g = FrameGate::new();
        assert_eq!(g.admit(&frame(0, 0, STAMP), &h, |_| 14), Admit::Current);
        g.advance(14);
        assert_eq!(g.admit(&frame(14, 14, STAMP), &h, |_| 14), Admit::Current);
        g.advance(14);
        assert_eq!(g.admit(&frame(28, 28, STAMP), &h, |_| 14), Admit::Current);
        g.advance(14);
        assert_eq!(g.stats().frames_accepted, 3);
        assert_eq!(g.stats().rejected_unconfirmed, 0);
    }

    #[test]
    fn gate_drops_an_isolated_stamp_collision_inside_a_run() {
        let h = header();
        let mut g = FrameGate::new();
        g.admit(&frame(0, 0, STAMP), &h, |_| 14);
        g.advance(14);
        g.admit(&frame(14, 14, STAMP), &h, |_| 14);
        g.advance(14);
        // Junk frame: footer passes, record number is garbage.
        assert_eq!(
            g.admit(&frame(0x7FFF_FFFF, 0xABCD_1234, STAMP), &h, |_| 14),
            Admit::Skip
        );
        // The run resumes exactly where it left off.
        assert_eq!(g.admit(&frame(28, 28, STAMP), &h, |_| 14), Admit::Current);
        g.advance(14);
        g.finish();
        assert_eq!(g.stats().rejected_unconfirmed, 1);
        assert_eq!(g.stats().frames_accepted, 3);
    }

    #[test]
    fn gate_resyncs_after_a_genuine_record_number_reset() {
        let h = header();
        let mut g = FrameGate::new();
        g.admit(&frame(0, 0, STAMP), &h, |_| 14);
        g.advance(14);
        g.admit(&frame(14, 14, STAMP), &h, |_| 14);
        g.advance(14);
        // Logger restarted: record numbers begin again at 0.
        assert_eq!(g.admit(&frame(100, 0, STAMP), &h, |_| 14), Admit::Skip);
        assert_eq!(
            g.admit(&frame(114, 14, STAMP), &h, |_| 14),
            Admit::HeldThenCurrent
        );
        g.take_held();
        g.advance(14);
        assert_eq!(g.stats().frames_accepted, 4);
        assert_eq!(g.stats().rejected_unconfirmed, 0);
    }

    #[test]
    fn tob2_falls_back_to_the_footer_check_alone() {
        let hdr = "\"TOB2\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                   \"T\",\"1 SEC\",\"44\",\"0\",\"4660\",\"SecMsec\",\"0\",\"0\",\"0\"\n\
                   \"a\"\n\"u\"\n\"Smp\"\n\"FP2\"\n";
        let h = parse_tob_header(&mut Cursor::new(hdr)).unwrap();
        let mut g = FrameGate::new();
        assert_eq!(
            g.admit(&frame(0, 0xDEAD, STAMP), &h, |_| 14),
            Admit::Current
        );
    }
}
