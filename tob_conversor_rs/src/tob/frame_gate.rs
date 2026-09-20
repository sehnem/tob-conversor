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
//! This module adds the checks that make the test selective.  A frame is
//! emitted only once it is **corroborated by its neighbour**, on two
//! independent counts:
//!
//! 1. **Full footer validation** — not just the stamp and the `E` flag, but
//!    the offset field, which on a real frame is either 0 or the small
//!    sub-frame boundary overhead, and never exceeds the frame's data
//!    capacity.
//! 2. **Record-number continuity** ([`FrameGate`], TOB3 only) — a frame whose
//!    `beg record` does not continue the previous frame is held back until the
//!    *next* frame confirms it by starting exactly `rows` records later.
//! 3. **Frame-clock cadence** — the confirming frame must also *begin* where
//!    the held frame's clock *ended*.  Junk that lines up on `beg` (it
//!    happens: ~2 M junk frames per card) essentially never also lines up in
//!    time, and this is what keeps a 1990 or a 2040 run out of a 2025 file.
//!
//! Nothing here guesses or repairs a value: a frame is either emitted verbatim
//! or rejected and counted in [`FrameStats`].

use super::decode::time_ns_from_frame_header;
use super::header::TobHeader;
use super::subframe::FrameWalk;

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

/// Which footer stamps count as this table's.
///
/// Campbell firmware does not always write the stamp the header declares:
/// CardConvert and "repair card" fragments carry `val_stamp + 1` on every main
/// footer, and CR1000 1-minute tables write `val_stamp - 1` on their sub-frame
/// boundaries.  Accepting that neighbourhood recovers those files — but it
/// also triples the number of junk frames whose stamp collides, and two
/// *instances of the same table* on one card routinely differ by exactly one
/// (this archive holds `AL_101` as both 17915 and 17916).
///
/// So the neighbourhood is not the default.  A file is read with [`Exact`]
/// first, and only a file that yields **nothing at all** that way is re-read
/// with [`Neighbourhood`] — see [`crate::tob::base_scan::begin_recovery`].  A
/// healthy file therefore reads exactly as it always did.
///
/// [`Exact`]: StampRule::Exact
/// [`Neighbourhood`]: StampRule::Neighbourhood
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StampRule {
    /// The table's declared stamp and its complement, and nothing else.
    #[default]
    Exact,
    /// Also the two off-by-one neighbours of each, for damaged fragments.
    Neighbourhood,
}

/// True when `stamp` is one this table's main frames may carry under `rule`.
pub(crate) fn stamp_matches(stamp: u16, header: &TobHeader, rule: StampRule) -> bool {
    let exact = stamp == header.val_stamp || stamp == header.comp_val_stamp;
    match rule {
        StampRule::Exact => exact,
        StampRule::Neighbourhood => {
            exact
                || [header.val_stamp, header.comp_val_stamp]
                    .iter()
                    .any(|&s| stamp == s.wrapping_sub(1) || stamp == s.wrapping_add(1))
        }
    }
}

/// How many frames were accepted, and why the others were not.
///
/// Rejections are *counted*, never silent: a caller that cares (an ingest
/// pipeline logging provenance, say) can report exactly how much of a file was
/// unwritten ring space versus real data.  `frames_accepted == 0` is the
/// "nothing in this file belongs to the declared table" verdict, and it is
/// reached without the caller having to scan the file a second time.
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
    /// Byte offset the emitted frames were read from.
    ///
    /// Normally the end of the ASCII header; a recovered file can start
    /// elsewhere (see [`crate::tob::base_scan::find_frame_base`]).
    pub frame_base: u64,
    /// The rows came from the widened second pass, not from a clean read.
    pub recovered: bool,
}

/// True when `frame_buf`'s footer is a well-formed main-frame footer for this
/// table, judged by the table's declared stamp alone.
pub fn is_valid_main_frame(frame_buf: &[u8], header: &TobHeader) -> bool {
    is_valid_main_frame_with(frame_buf, header, StampRule::Exact)
}

/// True when `frame_buf`'s footer is a well-formed main-frame footer for this
/// table under `rule`.
///
/// Checks, in order: the validation stamp matches the table; the `E` (empty)
/// flag is clear; the offset field fits inside the frame's data segment; and a
/// non-zero offset only ever appears on a frame that also flags itself as
/// carrying a minor frame.
pub fn is_valid_main_frame_with(frame_buf: &[u8], header: &TobHeader, rule: StampRule) -> bool {
    let Some(footer) = frame_footer(frame_buf, header) else {
        return false;
    };

    if !stamp_matches((footer >> 16) as u16, header, rule) {
        return false;
    }
    if footer & footer::FLAG_EMPTY != 0 {
        return false;
    }

    let capacity = frame_capacity(header);
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

/// The 32-bit footer word of a frame, or `None` when `frame_buf` is too short
/// to hold one.
fn frame_footer(frame_buf: &[u8], header: &TobHeader) -> Option<u32> {
    if header.frame_nbytes < 4 || frame_buf.len() < header.frame_nbytes {
        return None;
    }
    Some(u32::from_le_bytes(
        frame_buf[header.frame_nbytes - 4..header.frame_nbytes]
            .try_into()
            .unwrap(),
    ))
}

/// Data-segment size of a frame: everything that is not frame header or footer.
pub(crate) fn frame_capacity(header: &TobHeader) -> usize {
    let header_size = if header.is_tob2 { 8 } else { 12 };
    header.frame_nbytes.saturating_sub(header_size + 4)
}

/// True when a footer word's offset field could describe this frame.
///
/// Holds for a main frame footer and for the sub-footer that opens a sub-frame
/// boundary: both count bytes *inside* the frame, so a value past the data
/// segment means the word is not a footer at all.
pub(crate) fn footer_offset_fits(word: u32, header: &TobHeader) -> bool {
    (word & footer::OFFSET_MASK) as usize <= frame_capacity(header)
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

/// The timestamp of a frame's first record, from its `seconds`/`subseconds`.
pub(crate) fn frame_start_time_ns(frame_buf: &[u8], header: &TobHeader) -> i64 {
    time_ns_from_frame_header(
        u32::from_le_bytes(frame_buf[0..4].try_into().unwrap()),
        u32::from_le_bytes(frame_buf[4..8].try_into().unwrap()),
        header.frame_time_res,
    )
}

/// How far a frame's start time may sit from where the previous frame's clock
/// ended before the two stop looking like neighbours.
///
/// Zero would be the letter of the format, and on files whose frame headers
/// carry sub-second counts (`Sec100Usec` and friends) that is what the
/// arithmetic actually gives: a 100 MSEC table with 31 records a frame steps
/// 3.1 s, exactly.  But a logger that writes whole seconds only quantizes each
/// frame start independently, so consecutive frames differ by `floor(step)` or
/// `ceil(step)` and never by `step` itself — the ±1 s band is for them.  One
/// record interval on top absorbs a frame that starts a record early or late.
///
/// Against junk this is still overwhelming: a false `beg` match is already a
/// 2^-32 coincidence, and a junk `seconds` field lands within a few seconds of
/// a specific point in a 136-year range about as often.
pub(crate) fn tolerated_frame_drift_ns(header: &TobHeader) -> i64 {
    const ONE_SECOND_NS: i64 = 1_000_000_000;
    let interval_ns = (header.rec_intvl.max(0.0) * 1e9) as i64;
    ONE_SECOND_NS.saturating_add(interval_ns)
}

/// True when `start_ns` is where `walk`'s clock left off, within tolerance.
///
/// [`FrameWalk::end_time_ns`] is the time the record *after* the held frame's
/// last one would carry, and a sub-frame boundary inside that frame has
/// already been folded into it — so this compares like with like even across
/// the clock restatements that make raw `seconds` deltas useless.
fn continues_in_time(walk: &FrameWalk, start_ns: i64, header: &TobHeader) -> bool {
    if header.rec_intvl <= 0.0 {
        // No declared cadence (event-driven table): nothing to check against.
        return true;
    }
    let drift = start_ns.saturating_sub(walk.end_time_ns).saturating_abs();
    drift <= tolerated_frame_drift_ns(header)
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

/// A footer-valid frame at a discontinuity, awaiting confirmation.
struct Held {
    beg: u32,
    walk: FrameWalk,
    bytes: Vec<u8>,
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
    /// Instant the next genuine frame must start at.
    expected_time_ns: Option<i64>,
    /// `beg` of the frame most recently emitted.
    last_beg: u32,
    /// A footer-valid frame at a discontinuity, awaiting confirmation.
    held: Option<Held>,
    /// The next frame offered is the first one of the ring.
    at_ring_start: bool,
    /// Trust frame 0 of the ring without corroboration.
    trust_ring_start: bool,
    rule: StampRule,
    stats: FrameStats,
}

impl FrameGate {
    /// A gate for the ordinary first read of a file: the table's own stamp
    /// only, and frame 0 of the ring taken on trust.
    pub fn new() -> Self {
        Self {
            expected_beg: None,
            expected_time_ns: None,
            last_beg: 0,
            held: None,
            at_ring_start: true,
            trust_ring_start: true,
            rule: StampRule::Exact,
            stats: FrameStats::default(),
        }
    }

    /// A gate for the second, widened read of a file the first read found
    /// nothing in.
    ///
    /// Two things change, both because the premises of the first read have
    /// already failed on this file. The off-by-one stamps firmware writes on
    /// CardConvert fragments are accepted; and nothing is taken on trust, not
    /// even the frame at `frame_base` — "the logger fills its ring from the
    /// front" is exactly the assumption a file that needs recovering has
    /// broken, and a free pass there is how a lone junk frame used to become a
    /// frame's worth of 1990 rows.
    pub fn recovering(frame_base: u64) -> Self {
        Self {
            trust_ring_start: false,
            rule: StampRule::Neighbourhood,
            stats: FrameStats {
                frame_base,
                recovered: true,
                ..FrameStats::default()
            },
            ..Self::new()
        }
    }

    /// A gate that has been read out and folded into a total, kept only so
    /// the field is never empty. It trusts nothing, and its counts are zero,
    /// so folding it a second time adds nothing.
    pub fn spent() -> Self {
        Self {
            trust_ring_start: false,
            ..Self::new()
        }
    }

    /// Record where the frames being fed in start, for [`FrameStats`].
    pub fn set_frame_base(&mut self, frame_base: u64) {
        self.stats.frame_base = frame_base;
    }

    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Decide what to do with the frame just read.
    ///
    /// `walk` is called at most once, and only for a frame that lands at a
    /// discontinuity: it must return what that frame would emit.
    pub fn admit<F>(&mut self, frame_buf: &[u8], header: &TobHeader, walk: F) -> Admit
    where
        F: FnOnce(&[u8]) -> FrameWalk,
    {
        self.stats.frames_read += 1;
        let at_ring_start = std::mem::replace(&mut self.at_ring_start, false);

        if !is_valid_main_frame_with(frame_buf, header, self.rule) {
            self.stats.rejected_footer += 1;
            return Admit::Skip;
        }

        let beg = match frame_beg_record(frame_buf, header) {
            // TOB2: no record numbers, the footer check is all we have.
            None => {
                self.stats.frames_accepted += 1;
                return Admit::Current;
            }
            Some(b) => b,
        };

        // The run continues: this is the frame the previous one promised, by
        // record number and by the clock. A frame that continues the ladder
        // but jumps decades in time is a corrupt or overwritten frame, not the
        // next one — `698c80b9…AC_299.dat` has one of those, thirteen rows
        // dated 2056 in the middle of a 2022 run. Dropping it here costs
        // nothing else: the frame after it no longer continues the ladder
        // either, so it goes through the same hold-and-confirm path as any
        // other discontinuity and is emitted as soon as its neighbour backs it
        // up.
        let start_ns = frame_start_time_ns(frame_buf, header);
        if self.expected_beg == Some(beg) && self.continues_expected_time(start_ns, header) {
            self.drop_held();
            self.last_beg = beg;
            self.stats.frames_accepted += 1;
            return Admit::Current;
        }

        // A logger fills its ring from the front, so the frame at the base is
        // real data by construction; it is the frames *after* the write
        // pointer that are leftovers. That holds for a file the logger itself
        // wrote — `recovering` turns it off for one that needs repairing.
        if at_ring_start && self.trust_ring_start {
            self.drop_held();
            self.last_beg = beg;
            self.stats.frames_accepted += 1;
            return Admit::Current;
        }

        // Discontinuity. This frame is unproven on its own — park it until the
        // next one corroborates it, by record number *and* by the clock.
        if let Some(held) = self.held.as_ref() {
            let continues_by_record = beg == held.beg.wrapping_add(held.walk.rows as u32);
            if continues_by_record && continues_in_time(&held.walk, start_ns, header) {
                self.last_beg = beg;
                self.stats.frames_accepted += 2;
                return Admit::HeldThenCurrent;
            }
            self.stats.rejected_unconfirmed += 1;
        }

        let walk = walk(frame_buf);
        if walk.rows == 0 {
            // A frame the walk gets no records out of cannot corroborate
            // anything: "the next frame starts `rows` records and `rows`
            // intervals later" degenerates to "the next frame is identical",
            // which is a test two copies of the same junk frame pass. Junk is
            // where these come from — a real frame carries records — and
            // `45139a6c…AC_220.dat` had a pair of them put 22 rows dated 2040
            // into a 2020 file.
            self.held = None;
            self.stats.rejected_unconfirmed += 1;
            return Admit::Skip;
        }
        self.held = Some(Held {
            beg,
            walk,
            bytes: frame_buf.to_vec(),
        });
        Admit::Skip
    }

    /// Take the held frame's bytes after [`Admit::HeldThenCurrent`].
    pub fn take_held(&mut self) -> Option<Vec<u8>> {
        self.held.take().map(|h| h.bytes)
    }

    /// Record what the frame just emitted held, fixing the record number and
    /// the instant the next frame has to start at.
    ///
    /// Only the emitting walk knows either: the row count is not a constant (a
    /// frame carrying a sub-frame boundary holds one row fewer) and neither is
    /// the clock (that boundary restates it outright), so taking the time from
    /// the frame header plus `rows × interval` would make every
    /// boundary-carrying frame look like a jump.
    pub fn advance(&mut self, emitted: FrameWalk) {
        self.expected_beg = Some(self.last_beg.wrapping_add(emitted.rows as u32));
        self.expected_time_ns = Some(emitted.end_time_ns);
    }

    /// True when a frame starting at `start_ns` picks the run up where the
    /// last emitted frame left off.
    fn continues_expected_time(&self, start_ns: i64, header: &TobHeader) -> bool {
        match self.expected_time_ns {
            None => true,
            Some(expected) => {
                header.rec_intvl <= 0.0
                    || start_ns.saturating_sub(expected).saturating_abs()
                        <= tolerated_frame_drift_ns(header)
            }
        }
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
    use crate::tob::subframe::walk_frame;
    use std::io::Cursor;

    /// TOB3, frame_nbytes = 44, one FP2 column (2 bytes), stamp 4660, 1 SEC.
    /// Data segment = 44 - 12 - 4 = 28 bytes = 14 rows, so a frame spans 14 s.
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

    /// The row count and end clock the gate asks for, done for real.
    fn walk(header: &TobHeader) -> impl Fn(&[u8]) -> FrameWalk + '_ {
        move |f: &[u8]| walk_frame(f, header, |_, _, _| {})
    }

    /// Offer a frame and, when it is emitted, tell the gate what it held —
    /// the two-step dance every real caller does.
    fn feed(g: &mut FrameGate, f: &[u8], h: &TobHeader) -> Admit {
        let verdict = g.admit(f, h, walk(h));
        if verdict != Admit::Skip {
            g.advance(walk_frame(f, h, |_, _, _| {}));
        }
        verdict
    }

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
        assert!(!is_valid_main_frame(&frame(1, 0, 4662 << 16), &header()));
        assert!(!is_valid_main_frame_with(
            &frame(1, 0, 4662 << 16),
            &header(),
            StampRule::Neighbourhood
        ));
    }

    /// EX-1: CardConvert fragments carry `val_stamp + 1` on every main footer.
    /// The default read must not take them — two instances of one table on a
    /// card differ by exactly this much — but the recovery read must.
    #[test]
    fn off_by_one_stamps_are_recovery_only() {
        let h = header();
        for stamp in [4659u32, 4661] {
            assert!(
                !is_valid_main_frame(&frame(1, 0, stamp << 16), &h),
                "stamp {stamp} must not pass the default rule"
            );
            assert!(
                is_valid_main_frame_with(&frame(1, 0, stamp << 16), &h, StampRule::Neighbourhood),
                "stamp {stamp} must pass the recovery rule"
            );
        }
    }

    #[test]
    fn complement_stamp_is_valid_under_both_rules() {
        let h = header();
        let comp = (0xFFFFu32 ^ 4660) << 16;
        assert!(is_valid_main_frame(&frame(1, 0, comp), &h));
        assert!(is_valid_main_frame_with(
            &frame(1, 0, comp),
            &h,
            StampRule::Neighbourhood
        ));
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
        assert_eq!(feed(&mut g, &frame(0, 0, STAMP), &h), Admit::Current);
        assert_eq!(feed(&mut g, &frame(14, 14, STAMP), &h), Admit::Current);
        assert_eq!(feed(&mut g, &frame(28, 28, STAMP), &h), Admit::Current);
        assert_eq!(g.stats().frames_accepted, 3);
        assert_eq!(g.stats().rejected_unconfirmed, 0);
    }

    #[test]
    fn gate_drops_an_isolated_stamp_collision_inside_a_run() {
        let h = header();
        let mut g = FrameGate::new();
        feed(&mut g, &frame(0, 0, STAMP), &h);
        feed(&mut g, &frame(14, 14, STAMP), &h);
        // Junk frame: footer passes, record number is garbage.
        assert_eq!(
            feed(&mut g, &frame(0x7FFF_FFFF, 0xABCD_1234, STAMP), &h),
            Admit::Skip
        );
        // The run resumes exactly where it left off.
        assert_eq!(feed(&mut g, &frame(28, 28, STAMP), &h), Admit::Current);
        g.finish();
        assert_eq!(g.stats().rejected_unconfirmed, 1);
        assert_eq!(g.stats().frames_accepted, 3);
    }

    #[test]
    fn gate_resyncs_after_a_genuine_record_number_reset() {
        let h = header();
        let mut g = FrameGate::new();
        feed(&mut g, &frame(0, 0, STAMP), &h);
        feed(&mut g, &frame(14, 14, STAMP), &h);
        // Logger restarted: record numbers begin again at 0, and the two
        // frames after the restart are still 14 s apart.
        assert_eq!(feed(&mut g, &frame(100, 0, STAMP), &h), Admit::Skip);
        assert_eq!(
            feed(&mut g, &frame(114, 14, STAMP), &h),
            Admit::HeldThenCurrent
        );
        g.take_held();
        assert_eq!(g.stats().frames_accepted, 4);
        assert_eq!(g.stats().rejected_unconfirmed, 0);
    }

    /// A frame that picks the record ladder up but jumps decades in time is a
    /// corrupt frame, not the next one — `698c80b9…AC_299.dat` has thirteen
    /// rows dated 2056 in the middle of a 2022 run. Dropping it costs nothing
    /// else: the run picks up again as soon as two frames agree.
    #[test]
    fn a_frame_that_continues_the_ladder_but_not_the_clock_is_dropped() {
        let h = header();
        let mut g = FrameGate::new();
        feed(&mut g, &frame(0, 0, STAMP), &h);
        feed(&mut g, &frame(14, 14, STAMP), &h);
        // Right record number, clock decades out.
        assert_eq!(
            feed(&mut g, &frame(2_100_000_000, 28, STAMP), &h),
            Admit::Skip
        );
        // The next two frames corroborate each other, so nothing after the
        // corrupt frame is lost.
        assert_eq!(feed(&mut g, &frame(42, 42, STAMP), &h), Admit::Skip);
        assert_eq!(
            feed(&mut g, &frame(56, 56, STAMP), &h),
            Admit::HeldThenCurrent
        );
        g.take_held();
        g.finish();
        assert_eq!(
            g.stats().frames_accepted,
            4,
            "only the corrupt frame is lost"
        );
        assert_eq!(g.stats().rejected_unconfirmed, 1);
    }

    /// EX-4: two junk frames whose `beg` happens to continue each other. At
    /// ~2 M junk frames a card this is not rare, and before the clock test it
    /// was enough to put a frame's worth of 1990 rows into a 2025 file.
    #[test]
    fn a_beg_ladder_that_does_not_line_up_in_time_is_not_a_run() {
        let h = header();
        let w = walk(&h);
        let mut g = FrameGate::new();
        // Not at the ring base: this junk sits well past the write pointer,
        // where the real run has long since ended.
        g.admit(&frame(0, 0, 0x1111 << 16), &h, &w);
        // Both junk frames carry the same `seconds`, as the pair in
        // `58f16f26…CT_160.dat` does — a real pair would be 14 s apart.
        assert_eq!(
            g.admit(&frame(15_793_989, 1_032_597_859, STAMP), &h, &w),
            Admit::Skip
        );
        assert_eq!(
            g.admit(&frame(15_793_989, 1_032_597_873, STAMP), &h, &w),
            Admit::Skip
        );
        g.finish();
        assert_eq!(g.stats().frames_accepted, 0);
        assert_eq!(g.stats().rejected_unconfirmed, 2);
    }

    /// EX-3: `recs × interval` is frequently not a whole number (31 × 0.1 s),
    /// and a logger that writes whole seconds steps by `floor` or `ceil`. A
    /// test that demands the exact step rejects the run.
    #[test]
    fn whole_second_cadence_quantization_is_accepted() {
        // 100 MSEC, frame_nbytes 1008, 8 × IEEE4B → 31 records = 3.1 s a frame.
        let hdr = "\"TOB3\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                   \"T\",\"100 MSEC\",\"1008\",\"0\",\"4660\",\"Sec100Usec\",\"0\",\"0\",\"0\"\n\
                   \"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\",\"h\"\n\
                   \"u\",\"u\",\"u\",\"u\",\"u\",\"u\",\"u\",\"u\"\n\
                   \"S\",\"S\",\"S\",\"S\",\"S\",\"S\",\"S\",\"S\"\n\
                   \"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\",\"IEEE4B\"\n";
        let h = parse_tob_header(&mut Cursor::new(hdr)).unwrap();
        assert_eq!(frame_capacity(&h) / h.line_nbytes, 31);

        let big = |seconds: u32, beg: u32| {
            let mut f = Vec::with_capacity(1008);
            f.extend_from_slice(&seconds.to_le_bytes());
            f.extend_from_slice(&0u32.to_le_bytes());
            f.extend_from_slice(&beg.to_le_bytes());
            f.resize(1004, 0);
            f.extend_from_slice(&STAMP.to_le_bytes());
            f
        };
        let w = walk(&h);
        // Deltas of 3 and 4 s alternate around the true 3.1 s step; both must
        // confirm. Neither run starts at the ring base, so both are corroborated.
        for delta in [3u32, 4] {
            let mut g = FrameGate::new();
            g.admit(&big(0xDEAD_BEEF, 0xFFFF_0000), &h, &w);
            assert_eq!(g.admit(&big(967_908_321, 32_077_717), &h, &w), Admit::Skip);
            assert_eq!(
                g.admit(&big(967_908_321 + delta, 32_077_717 + 31), &h, &w),
                Admit::HeldThenCurrent,
                "a {delta} s step must confirm a 3.1 s frame"
            );
        }
    }

    /// A frame the walk gets nothing out of proves nothing. Two copies of one
    /// junk frame satisfy "the next frame starts `rows` records later" for
    /// free when `rows` is zero, and that is how `45139a6c…AC_220.dat` ended up
    /// with 22 rows dated 2040 in it.
    #[test]
    fn a_frame_holding_no_records_cannot_corroborate_anything() {
        // 16-byte frames: 12 header + 4 footer, so the data segment is empty.
        let hdr = "\"TOB3\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                   \"T\",\"1 SEC\",\"16\",\"0\",\"4660\",\"SecMsec\",\"0\",\"0\",\"0\"\n\
                   \"a\"\n\"u\"\n\"Smp\"\n\"FP2\"\n";
        let h = parse_tob_header(&mut Cursor::new(hdr)).unwrap();
        let empty = |beg: u32| {
            let mut f = Vec::with_capacity(16);
            f.extend_from_slice(&1_590_728_769u32.to_le_bytes());
            f.extend_from_slice(&0u32.to_le_bytes());
            f.extend_from_slice(&beg.to_le_bytes());
            f.extend_from_slice(&STAMP.to_le_bytes());
            f
        };
        let w = walk(&h);
        assert_eq!(walk_frame(&empty(0), &h, |_, _, _| {}).rows, 0);

        let mut g = FrameGate::new();
        g.admit(&empty(1), &h, &w); // consume the ring-start free pass
        // Two identical junk frames: same record number, same instant.
        assert_eq!(g.admit(&empty(3_315_647_298), &h, &w), Admit::Skip);
        assert_eq!(g.admit(&empty(3_315_647_298), &h, &w), Admit::Skip);
        g.finish();
        assert_eq!(g.stats().rejected_unconfirmed, 2);
    }

    #[test]
    fn tob2_falls_back_to_the_footer_check_alone() {
        let hdr = "\"TOB2\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                   \"T\",\"1 SEC\",\"44\",\"0\",\"4660\",\"SecMsec\",\"0\",\"0\",\"0\"\n\
                   \"a\"\n\"u\"\n\"Smp\"\n\"FP2\"\n";
        let h = parse_tob_header(&mut Cursor::new(hdr)).unwrap();
        let w = walk(&h);
        let mut g = FrameGate::new();
        assert_eq!(g.admit(&frame(0, 0xDEAD, STAMP), &h, &w), Admit::Current);
    }

    /// The recovery gate gives nothing a free pass, so a card whose only
    /// footer-valid frames are scattered junk decodes to no rows at all —
    /// `94b2da64…ETO_CS.dat` and `b96668f5…CS_150.dat` are this shape.
    #[test]
    fn the_recovery_gate_emits_nothing_without_a_corroborated_pair() {
        let h = header();
        let w = walk(&h);
        let mut g = FrameGate::recovering(0);
        assert_eq!(g.admit(&frame(0, 0, STAMP), &h, &w), Admit::Skip);
        assert_eq!(
            g.admit(&frame(0x7FFF_FFFF, 0xABCD_1234, STAMP), &h, &w),
            Admit::Skip
        );
        g.finish();
        assert_eq!(g.stats().frames_accepted, 0);
        assert!(g.stats().recovered);
    }

    /// …but a real run still reads under the recovery gate, off-by-one stamp
    /// and all: `ea321b94…SM_150.dat` is a `val_stamp + 1` fragment.
    #[test]
    fn the_recovery_gate_reads_an_off_by_one_run() {
        let h = header();
        let w = walk(&h);
        let plus_one: u32 = 4661 << 16;
        let mut g = FrameGate::recovering(0);
        assert_eq!(g.admit(&frame(0, 0, plus_one), &h, &w), Admit::Skip);
        assert_eq!(
            g.admit(&frame(14, 14, plus_one), &h, &w),
            Admit::HeldThenCurrent
        );
        g.take_held();
        g.advance(walk_frame(&frame(14, 14, plus_one), &h, |_, _, _| {}));
        assert_eq!(g.admit(&frame(28, 28, plus_one), &h, &w), Admit::Current);
        assert_eq!(g.stats().frames_accepted, 3);
    }
}
