//! Reading a card image whose frames are not where the ASCII header says.
//!
//! The reader's normal assumption — frame 0 begins at the first byte after the
//! 6-line prolog, and everything from there is one run — holds for every file a
//! logger wrote itself. It does not hold for a 2 GB card image:
//!
//! * a fragment whose prolog was re-padded has its frames a fixed number of
//!   bytes off (`0e95e2ce…AC_221.dat`: half a frame);
//! * a prolog whose CRLF padding overlaps frame 0 puts the frames *before* the
//!   apparent end of the header;
//! * and a card that has been reused holds **more than one run**, at more than
//!   one alignment. `fa1fd3a4…AC_220.dat` carries 2020-11-26 → 12-01 at the
//!   header's own alignment and 2020-12-01 → 12-16 sixteen bytes off; the two
//!   are consecutive in time and both are this table's, one instance of it
//!   after another. Reading either one alone loses the other.
//!
//! Decoded at the wrong offset every footer lands mid-record and the file reads
//! as empty, which is the signal this module keys on: a file that yields
//! **nothing** on the ordinary read gets one second chance, and only then.
//! [`FramePass`] scans it for runs, widens the stamp rule (see [`StampRule`]),
//! and reads each run it found. A file that read clean the first time never
//! pays for any of this and is not affected by any of it.
//!
//! What the scan looks for is not a stamp — stamps collide roughly once per
//! 2^17 junk frames, ~14 times per card — but a **corroborated pair**: two
//! consecutive frames where the second starts at the record number *and* at the
//! instant the first one ends at. Junk manages that at about 2^-32 per
//! candidate, and a run has to manage it [`MIN_RUN_FRAMES`] times over.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::{Read, Seek, SeekFrom};

use super::frame_gate::{
    FrameGate, FrameStats, StampRule, frame_start_time_ns, is_valid_main_frame_with,
    tolerated_frame_drift_ns,
};
use super::header::TobHeader;
use super::subframe::walk_frame;

/// File bytes held at once while scanning. Large enough that the per-window
/// overlap is negligible, small enough to stay out of the way.
const WINDOW_BYTES: usize = 4 << 20;

/// Frames a recovered run must hold before it is read.
///
/// Three corroborated pairs in a row. One is already a 2^-32 coincidence, so
/// this is not about the arithmetic — it is about never turning the recovery
/// pass into a licence to emit the handful of leftover frames a junk-only card
/// does have (`94b2da64…ETO_CS.dat` has eight of them).
const MIN_RUN_FRAMES: u64 = 4;

/// A stretch of consecutive frames that corroborate each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    /// Byte offset of the run's first frame.
    pub start: u64,
    /// Byte offset just past the run's last frame.
    pub end: u64,
    /// Timestamp of the first frame, used to read the runs in time order.
    pub first_time_ns: i64,
}

impl Run {
    fn frames(&self, frame_nbytes: u64) -> u64 {
        (self.end - self.start) / frame_nbytes
    }
}

/// Where a frame-reading loop is in a file, and what it still has left to read.
///
/// Both frame loops in this crate — the TOA5 writer's and the Arrow reader's —
/// drive one of these, so that "read it again, differently" means the same
/// thing in both and happens on the same evidence.
pub(crate) struct FramePass {
    /// The gate for the stretch of file currently being read.
    pub gate: FrameGate,
    header_end: u64,
    file_len: u64,
    /// The second pass has been tried; there is no third.
    recovery_spent: bool,
    /// Runs the recovery pass has yet to read, last one first.
    queue: Vec<Run>,
    /// Bytes of the run being read that have not been handed out yet.
    run_remaining: u64,
    /// Gates already retired, summed.
    retired: FrameStats,
    /// Where the first frames that were actually emitted came from.
    emitted_base: Option<u64>,
}

impl FramePass {
    /// Begin the ordinary first pass. `reader` must sit at the end of the
    /// ASCII header, which is where the frames are assumed to start.
    pub fn start<R: Seek>(reader: &mut R) -> Result<Self, String> {
        let header_end = reader.stream_position().map_err(|e| e.to_string())?;
        let file_len = stream_len(reader).map_err(|e| e.to_string())?;
        let mut gate = FrameGate::new();
        gate.set_frame_base(header_end);
        Ok(Self {
            gate,
            header_end,
            file_len,
            recovery_spent: false,
            queue: Vec::new(),
            run_remaining: 0,
            retired: FrameStats::default(),
            emitted_base: None,
        })
    }

    /// Read the next frame this pass has to offer into `frame_buf`.
    ///
    /// `false` means the file is finished: the first pass reached the end and
    /// either produced rows or had nothing to recover, and any recovered runs
    /// have been read.
    pub fn read_frame<R: Read + Seek>(
        &mut self,
        reader: &mut R,
        header: &TobHeader,
        frame_buf: &mut [u8],
    ) -> bool {
        let frame_nbytes = header.frame_nbytes as u64;
        loop {
            if self.recovery_spent {
                if self.run_remaining < frame_nbytes {
                    if !self.open_next_run(reader) {
                        return false;
                    }
                    continue;
                }
                if reader.read_exact(frame_buf).is_err() {
                    // Truncated mid-run; there may still be later runs.
                    self.run_remaining = 0;
                    continue;
                }
                self.run_remaining -= frame_nbytes;
                return true;
            }

            if reader.read_exact(frame_buf).is_ok() {
                return true;
            }
            if !self.begin_recovery(reader, header) {
                return false;
            }
        }
    }

    /// What the whole read made of the file's frames, across every run.
    pub fn stats(&self) -> FrameStats {
        let open = self.gate.stats();
        let base = self
            .emitted_base
            .or_else(|| (open.frames_accepted > 0).then_some(open.frame_base));
        FrameStats {
            frames_read: self.retired.frames_read + open.frames_read,
            frames_accepted: self.retired.frames_accepted + open.frames_accepted,
            rejected_footer: self.retired.rejected_footer + open.rejected_footer,
            rejected_unconfirmed: self.retired.rejected_unconfirmed + open.rejected_unconfirmed,
            frame_base: base.unwrap_or(self.header_end),
            recovered: self.retired.recovered || open.recovered,
        }
    }

    /// Close the file out, so a frame still awaiting confirmation is dropped
    /// and counted rather than forgotten.
    pub fn finish(&mut self) {
        self.retire_gate();
    }

    /// Scan a file the first pass found nothing in, and queue what it holds.
    fn begin_recovery<R: Read + Seek>(&mut self, reader: &mut R, header: &TobHeader) -> bool {
        self.retire_gate();
        let nothing_read = self.retired.frames_accepted == 0;
        if self.recovery_spent || !nothing_read {
            // Either already tried, or the ordinary read worked — and a read
            // that produced rows is never replayed, which is what makes the
            // second pass incapable of duplicating one.
            self.recovery_spent = true;
            return false;
        }
        self.recovery_spent = true;

        let mut runs = find_runs(reader, header, self.header_end, self.file_len);
        if runs.is_empty() {
            return false;
        }
        // Popped from the back, so reverse into first-to-read order.
        runs.reverse();
        self.queue = runs;
        self.open_next_run(reader)
    }

    /// Seek to the next queued run and give it its own gate.
    fn open_next_run<R: Seek>(&mut self, reader: &mut R) -> bool {
        self.retire_gate();
        let Some(run) = self.queue.pop() else {
            return false;
        };
        if reader.seek(SeekFrom::Start(run.start)).is_err() {
            return false;
        }
        self.gate = FrameGate::recovering(run.start);
        self.run_remaining = run.end - run.start;
        true
    }

    /// Fold the current gate's counts into the totals and stand a spent one in
    /// its place, so nothing is double-counted when this is called twice.
    fn retire_gate(&mut self) {
        self.gate.finish();
        let s = self.gate.stats();
        self.retired.frames_read += s.frames_read;
        self.retired.frames_accepted += s.frames_accepted;
        self.retired.rejected_footer += s.rejected_footer;
        self.retired.rejected_unconfirmed += s.rejected_unconfirmed;
        self.retired.recovered |= s.recovered;
        if s.frames_accepted > 0 && self.emitted_base.is_none() {
            self.emitted_base = Some(s.frame_base);
        }
        self.gate = FrameGate::spent();
    }
}

/// Total length of a seekable stream, leaving the position where it was.
fn stream_len<R: Seek>(reader: &mut R) -> std::io::Result<u64> {
    let here = reader.stream_position()?;
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(here))?;
    Ok(end)
}

/// Every run of corroborating frames in the file, in the order to read them.
///
/// Runs come back in **time** order rather than file order: a reused card holds
/// its instances in whatever order the ring was rewritten, and a caller asking
/// for a DataFrame would rather not have to sort it.
pub(crate) fn find_runs<R: Read + Seek>(
    reader: &mut R,
    header: &TobHeader,
    header_end: u64,
    file_len: u64,
) -> Vec<Run> {
    let frame_nbytes = header.frame_nbytes;
    // TOB1 has no frames at all, and a TOB2 frame header carries no record
    // number — there is nothing for one frame to corroborate the next with, so
    // this cannot tell a run from a stretch of measurements. Both are left to
    // the ordinary read.
    if header.is_tob1 || header.is_tob2 || frame_nbytes < 16 || !frame_nbytes.is_multiple_of(4) {
        return Vec::new();
    }
    // A candidate needs a whole frame and its successor inside the window.
    let Some(pair_bytes) = frame_nbytes.checked_mul(2) else {
        return Vec::new();
    };
    if file_len.saturating_sub(header_end) < pair_bytes as u64 {
        return Vec::new();
    }
    let frame_span = frame_nbytes as u64;
    let pair_span = pair_bytes as u64;

    let window = WINDOW_BYTES.max(pair_bytes * 2);
    let mut buf = vec![0u8; window];
    // One run in progress per alignment of a frame start relative to
    // `header_end`: two instances of a table on one card are usually at
    // different alignments, and their pairs interleave as the scan goes by.
    let mut open: HashMap<u64, Run> = HashMap::new();
    let mut found: Vec<Run> = Vec::new();
    let mut window_start = header_end;

    while window_start + pair_span <= file_len {
        let want = window.min((file_len - window_start) as usize);
        if reader.seek(SeekFrom::Start(window_start)).is_err() {
            break;
        }
        if reader.read_exact(&mut buf[..want]).is_err() {
            break;
        }

        let mut start = 0usize;
        while start + pair_bytes <= want {
            let frame = &buf[start..];
            if is_corroborated_pair(frame, header) {
                let at = window_start + start as u64;
                let alignment = (at - header_end) % frame_span;
                let fresh = Run {
                    start: at,
                    end: at + pair_span,
                    first_time_ns: frame_start_time_ns(frame, header),
                };
                match open.entry(alignment) {
                    Entry::Vacant(slot) => {
                        slot.insert(fresh);
                    }
                    Entry::Occupied(mut slot) => {
                        let run = slot.get_mut();
                        if run.end >= at + pair_span {
                            // Already covered — windows overlap by a pair so
                            // that one across the seam is not missed, which
                            // means the pairs there are seen twice.
                        } else if run.end == at + frame_span {
                            run.end = at + pair_span;
                        } else {
                            let closed = std::mem::replace(run, fresh);
                            keep_if_long_enough(&mut found, closed, frame_span);
                        }
                    }
                }
            }
            start += 4;
        }

        window_start += (want - pair_bytes + 4) as u64;
    }

    for run in open.into_values() {
        keep_if_long_enough(&mut found, run, frame_span);
    }
    order_runs(&mut found, reader, header);
    found
}

fn keep_if_long_enough(found: &mut Vec<Run>, run: Run, frame_span: u64) {
    if run.frames(frame_span) >= MIN_RUN_FRAMES {
        found.push(run);
    }
}

/// Extend each run backwards, fold the ones that overlap together, and sort by
/// time.
fn order_runs<R: Read + Seek>(runs: &mut Vec<Run>, reader: &mut R, header: &TobHeader) {
    for run in runs.iter_mut() {
        run.start = walk_back_to_first_frame(reader, header, run.start);
    }

    // Two runs never share bytes: whichever starts first owns them, so a frame
    // is read at most once however many alignments end up claiming it.
    runs.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut kept: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs.iter() {
        match kept.last_mut() {
            Some(last) if run.start < last.end => last.end = last.end.max(run.end),
            _ => kept.push(*run),
        }
    }

    kept.sort_by_key(|r| (r.first_time_ns, r.start));
    *runs = kept;
}

/// True when the frame at `buf[0]` is followed by one that continues it, by
/// record number *and* by the clock.
fn is_corroborated_pair(buf: &[u8], header: &TobHeader) -> bool {
    let rule = StampRule::Neighbourhood;
    if !is_valid_main_frame_with(buf, header, rule) {
        return false;
    }
    let next = &buf[header.frame_nbytes..];
    if !is_valid_main_frame_with(next, header, rule) {
        return false;
    }

    let beg = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    let next_beg = u32::from_le_bytes(next[8..12].try_into().unwrap());
    let walk = walk_frame(buf, header, |_, _, _| {});
    if next_beg != beg.wrapping_add(walk.rows as u32) {
        return false;
    }
    if header.rec_intvl <= 0.0 {
        return true;
    }
    let drift = frame_start_time_ns(next, header)
        .saturating_sub(walk.end_time_ns)
        .saturating_abs();
    drift <= tolerated_frame_drift_ns(header)
}

/// Step back from a known-good frame, whole frames at a time, for as long as
/// the frame before it corroborates the one after it.
///
/// The scan can only see a pair once both its frames are inside a window that
/// starts at the end of the prolog, so a run whose first frame sits *before*
/// that — a prolog whose CRLF padding overlaps frame 0 — would lose it. The
/// step back applies the same two-frame test rather than settling for a valid
/// footer, because one frame back from a real run is exactly where the
/// leftovers of the previous write live.
fn walk_back_to_first_frame<R: Read + Seek>(reader: &mut R, header: &TobHeader, from: u64) -> u64 {
    let frame_nbytes = header.frame_nbytes as u64;
    let mut base = from;
    let mut pair = vec![0u8; header.frame_nbytes * 2];
    while base >= frame_nbytes {
        let candidate = base - frame_nbytes;
        if reader.seek(SeekFrom::Start(candidate)).is_err() {
            break;
        }
        if reader.read_exact(&mut pair).is_err() {
            break;
        }
        if !is_corroborated_pair(&pair, header) {
            break;
        }
        base = candidate;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tob::header::parse_tob_header;
    use std::io::Cursor;

    /// TOB3, 1 SEC, frame_nbytes 44, stamp 4660, one FP2 column → 14 rows.
    const PROLOG: &str = "\"TOB3\",\"S\",\"CR1000\",\"1\",\"O\",\"P\",\"G\",\"D\"\n\
                          \"T\",\"1 SEC\",\"44\",\"0\",\"4660\",\"SecMsec\",\"0\",\"0\",\"0\"\n\
                          \"a\"\n\"u\"\n\"Smp\"\n\"FP2\"\n";

    const FRAME: u64 = 44;

    fn header() -> TobHeader {
        parse_tob_header(&mut Cursor::new(PROLOG)).unwrap()
    }

    fn frame(seconds: u32, beg: u32, stamp: u16) -> Vec<u8> {
        let mut f = Vec::with_capacity(44);
        f.extend_from_slice(&seconds.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&beg.to_le_bytes());
        f.resize(40, 0);
        f.extend_from_slice(&((stamp as u32) << 16).to_le_bytes());
        f
    }

    /// A run of `n` frames: 14 records and 14 s a frame.
    fn run_bytes(n: u32, t0: u32, beg0: u32, stamp: u16) -> Vec<u8> {
        (0..n)
            .flat_map(|i| frame(t0 + i * 14, beg0 + i * 14, stamp))
            .collect()
    }

    fn runs_of(blob: Vec<u8>, header_end: u64) -> Vec<Run> {
        let len = blob.len() as u64;
        find_runs(&mut Cursor::new(blob), &header(), header_end, len)
    }

    /// A file whose frames start `shift` bytes after the prolog ends.
    fn shifted_file(shift: usize, stamp: u16, frames: u32) -> (Vec<u8>, u64) {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        blob.extend(std::iter::repeat_n(0x20u8, shift));
        blob.extend(run_bytes(frames, 0, 0, stamp));
        (blob, header_end)
    }

    #[test]
    fn finds_a_run_that_is_where_the_header_says_it_is() {
        let (blob, header_end) = shifted_file(0, 4660, 40);
        let runs = runs_of(blob, header_end);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, header_end);
        assert_eq!(runs[0].frames(FRAME), 40);
    }

    /// EX-2: `…AC_221.dat` fragments in this archive sit half a frame past the
    /// prolog, and at the declared base every footer lands on a data byte.
    #[test]
    fn finds_a_run_offset_from_the_header() {
        for shift in [4usize, 16, 20] {
            let (blob, header_end) = shifted_file(shift, 4660, 40);
            let runs = runs_of(blob, header_end);
            assert_eq!(runs.len(), 1, "shift {shift}");
            assert_eq!(runs[0].start, header_end + shift as u64, "shift {shift}");
        }
    }

    /// The scan runs under the widened stamp rule, because the files that need
    /// it are the CardConvert fragments that carry `val_stamp + 1`.
    #[test]
    fn finds_a_run_with_off_by_one_stamps() {
        let (blob, header_end) = shifted_file(8, 4661, 40);
        let runs = runs_of(blob, header_end);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, header_end + 8);
    }

    #[test]
    fn a_file_with_no_run_at_all_yields_none() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        // Footer-valid frames, but scattered and never continuing each other.
        for i in 0..40u32 {
            if i % 7 == 0 {
                blob.extend(frame(0xDEAD_0000 + i, 0xABCD_0000u32.wrapping_mul(i), 4660));
            } else {
                blob.extend(frame(i, i, 0x1111));
            }
        }
        assert!(runs_of(blob, header_end).is_empty());
    }

    /// A stretch too short to be anything but a coincidence is not a run.
    #[test]
    fn a_lone_corroborated_pair_is_not_a_run() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        blob.extend(run_bytes(60, 0, 0, 0x1111)); // some other table
        blob.extend(run_bytes(2, 5_000, 900, 4660)); // two frames that agree
        blob.extend(run_bytes(60, 0, 0, 0x1111));
        assert!(runs_of(blob, header_end).is_empty());
    }

    /// A run that begins before the apparent end of the prolog — the shape of
    /// `CS_141.dat`, whose CRLF padding overlaps frame 0 — keeps that frame.
    #[test]
    fn walks_back_past_the_header_end_when_frame_zero_is_there() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let base = blob.len() as u64 - 8;
        blob.truncate(base as usize);
        blob.extend(run_bytes(40, 0, 0, 4660));
        let runs = runs_of(blob, base + 8);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, base);
    }

    /// A run that starts far past the header is found without a window that
    /// happens to contain it: the scan keeps going until it does.
    #[test]
    fn finds_a_run_that_starts_far_into_the_file() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        blob.extend(std::iter::repeat_n(0u8, 12));
        for i in 0..200_000u32 {
            blob.extend(frame(0xFFFF_0000 ^ i, 0x3333_0000 ^ i, 0x2222));
        }
        let first = blob.len() as u64;
        blob.extend(run_bytes(40, 0, 0, 4660));
        let runs = runs_of(blob, header_end);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, first);
    }

    /// EX-2, the case no single base can answer: a reused card holds two
    /// instances of the table at two alignments, consecutive in time
    /// (`fa1fd3a4…AC_220.dat`: 2020-11-26 → 12-01 at the header's alignment,
    /// 2020-12-01 → 12-16 sixteen bytes off). Both are the table's, and they
    /// come back oldest first however they are laid out in the file.
    #[test]
    fn finds_both_instances_on_a_reused_card_oldest_first() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        // The *later* instance comes first in the file.
        blob.extend(run_bytes(40, 900_000, 7_000_000, 4661));
        blob.extend(std::iter::repeat_n(0u8, 8));
        let older_at = blob.len() as u64;
        blob.extend(run_bytes(40, 1_000, 500, 4660));

        let runs = runs_of(blob, header_end);
        assert_eq!(runs.len(), 2, "one run per instance");
        assert_eq!(runs[0].start, older_at, "the older run is read first");
        assert_eq!(runs[1].start, header_end);
        assert!(runs[0].first_time_ns < runs[1].first_time_ns);
    }

    /// Runs never overlap, so no frame is read — or emitted — twice.
    #[test]
    fn runs_never_claim_the_same_bytes_twice() {
        let mut blob = PROLOG.as_bytes().to_vec();
        let header_end = blob.len() as u64;
        blob.extend(run_bytes(60, 1_000, 500, 4660));
        blob.extend(std::iter::repeat_n(0u8, 4));
        blob.extend(run_bytes(60, 900_000, 7_000_000, 4661));
        let data_bytes = blob.len() as u64 - header_end;

        let mut runs = runs_of(blob, header_end);
        runs.sort_by_key(|r| r.start);
        for pair in runs.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
        let total: u64 = runs.iter().map(|r| r.end - r.start).sum();
        assert!(
            total <= data_bytes,
            "runs claim {total} of {data_bytes} bytes"
        );
    }
}
