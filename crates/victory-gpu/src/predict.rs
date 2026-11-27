//! Whether the refreshes of host copies can be known a frame ahead
//! (`VICTORY_PREDICT=1`; off by default). Stage 1 only learns and counts:
//! nothing it does changes what is copied or when.
//!
//! The thread that prepares submissions spends most of a frame bringing host
//! copies of title memory up to date (docs/findings.md: 48 of 55 ms). That
//! work waits until the submission's turn comes, although the title has
//! written the bytes by the time it submits. Frames repeat: the same
//! submission of the next frame mostly refreshes the same ranges of the same
//! copies. If it does, those ranges can be refreshed early, on other
//! threads, from the moment the title submits (stage 2), and the preparing
//! thread finds them already up to date.
//!
//! A submission is known by its place among the frame's submissions and its
//! number of lists. A refresh counts as predicted when the same submission
//! of the frame before refreshed an overlapping range of the same copy. The
//! numbers that matter, per frame: the time refreshes took that copied
//! something, and how much of that time was predicted. Times are the
//! threads' (refreshes run on several at once), not the frame's.
//!
//! What stage 2 must keep (found reading `Shadow::update_free`): the write
//! watch is asked with a reset, so two threads must never refresh the same
//! copy at once, or one may find "nothing new" while the other still copies.
//! The early refreshes of a submission are joined before its own.

use std::collections::HashMap;

/// A submission: its place in the frame, its number of lists.
pub type Key = (u32, u32);

/// Ranges refreshed, by copy.
type Spans = HashMap<u64, Vec<(usize, usize)>>;

/// Most ranges kept per submission (a frame refreshes some thousands).
const MOST: usize = 16_384;

/// Counts since the last [`Predictor::take`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub submissions: u64,
    pub refreshes: u64,
    /// Refreshes that copied something, and their time (µs).
    pub worked: u64,
    pub worked_us: u64,
    /// Of those, the ones the frame before predicted.
    pub predicted: u64,
    pub predicted_us: u64,
    /// Bytes they copied, and of them predicted.
    pub bytes: u64,
    pub predicted_bytes: u64,
}

#[derive(Default)]
pub struct Predictor {
    frame: u64,
    ordinal: u32,
    current: Key,
    before: HashMap<Key, Spans>,
    now: HashMap<Key, Spans>,
    kept: HashMap<Key, usize>,
    stats: Stats,
}

impl Predictor {
    /// A submission of frame `frame` with `lists` lists begins.
    pub fn begin(&mut self, frame: u64, lists: usize) {
        if frame != self.frame {
            self.before = std::mem::take(&mut self.now);
            self.kept.clear();
            (self.frame, self.ordinal) = (frame, 0);
        }
        self.current = (self.ordinal, lists.min(u32::MAX as usize) as u32);
        self.ordinal += 1;
        self.stats.submissions += 1;
    }

    /// Whether the same submission of the frame before refreshed a range
    /// of `copy` overlapping `[from, to)`.
    pub fn predicted(&self, copy: u64, from: usize, to: usize) -> bool {
        self.before
            .get(&self.current)
            .and_then(|s| s.get(&copy))
            .is_some_and(|r| r.iter().any(|&(a, b)| a < to && from < b))
    }

    /// The current submission refreshed `[from, to)` of `copy`, copying
    /// `copied` bytes in `us` microseconds.
    pub fn note(&mut self, copy: u64, from: usize, to: usize, copied: usize, us: u64) {
        let s = &mut self.stats;
        s.refreshes += 1;
        if copied > 0 {
            let predicted = self.predicted(copy, from, to);
            let s = &mut self.stats;
            s.worked += 1;
            s.worked_us += us;
            s.bytes += copied as u64;
            if predicted {
                s.predicted += 1;
                s.predicted_us += us;
                s.predicted_bytes += copied as u64;
            }
        }
        let kept = self.kept.entry(self.current).or_insert(0);
        if *kept < MOST {
            *kept += 1;
            self.now.entry(self.current).or_default().entry(copy).or_default().push((from, to));
        }
    }

    pub fn take(&mut self) -> Stats {
        std::mem::take(&mut self.stats)
    }
}

/// `VICTORY_PREDICT=1`.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_PREDICT").is_some_and(|v| v == "1"))
}

static PREDICTOR: std::sync::Mutex<Option<Predictor>> = std::sync::Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Predictor) -> T) -> T {
    let mut p = PREDICTOR.lock().unwrap_or_else(|e| e.into_inner());
    f(p.get_or_insert_with(Predictor::default))
}

/// [`Predictor::begin`], if on. Submissions are prepared one at a time.
pub fn begin(frame: u64, lists: usize) {
    if on() {
        with(|p| p.begin(frame, lists));
    }
}

/// [`Predictor::note`], if on; from any of the refreshing threads.
pub fn note(copy: u64, from: usize, to: usize, copied: usize, us: u64) {
    if on() {
        with(|p| p.note(copy, from, to, copied, us));
    }
}

/// The log line over `frames` frames, if on.
pub fn line(frames: f64) -> Option<String> {
    if !on() || frames <= 0.0 {
        return None;
    }
    let s = with(Predictor::take);
    Some(text(&s, frames))
}

pub fn text(s: &Stats, frames: f64) -> String {
    let share = |part: u64, all: u64| if all == 0 { 0.0 } else { part as f64 * 100.0 / all as f64 };
    format!(
        "refresh prediction (VICTORY_PREDICT=1), per frame: {:.0} submissions, {:.0} refreshes, {:.0} of them copied \
         {:.1} MB in {:.1} ms of threads' time; the frame before predicted {:.0} of those ({:.0} % of their time, \
         {:.1} ms; {:.0} % of their bytes)",
        s.submissions as f64 / frames,
        s.refreshes as f64 / frames,
        s.worked as f64 / frames,
        s.bytes as f64 / frames / 1e6,
        s.worked_us as f64 / frames / 1e3,
        s.predicted as f64 / frames,
        share(s.predicted_us, s.worked_us),
        s.predicted_us as f64 / frames / 1e3,
        share(s.predicted_bytes, s.bytes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two frames of two submissions each; the second frame repeats the
    /// first but for one range of a copy the first did not touch.
    #[test]
    fn the_same_submission_of_the_frame_before_predicts() {
        let mut p = Predictor::default();
        p.begin(1, 3);
        p.note(7, 0x1000, 0x2000, 4096, 10);
        p.begin(1, 1);
        p.note(8, 0x0, 0x100, 256, 5);
        let first = p.take();
        assert_eq!((first.submissions, first.worked, first.predicted), (2, 2, 0));

        p.begin(2, 3);
        p.note(7, 0x1800, 0x1900, 256, 4); // overlaps what it refreshed before
        p.note(9, 0x0, 0x10, 16, 6); // a copy it did not refresh before
        p.begin(2, 1);
        p.note(7, 0x1000, 0x2000, 4096, 10); // copy 7 belonged to the other submission
        p.note(8, 0x80, 0x90, 0, 1); // copied nothing: no work to predict
        let second = p.take();
        assert_eq!(second.submissions, 2);
        assert_eq!(second.refreshes, 4);
        assert_eq!(second.worked, 3);
        assert_eq!((second.predicted, second.predicted_us, second.predicted_bytes), (1, 4, 256));
        assert_eq!(second.worked_us, 20);
    }

    #[test]
    fn a_submission_with_another_number_of_lists_is_another() {
        let mut p = Predictor::default();
        p.begin(1, 2);
        p.note(7, 0, 100, 100, 1);
        p.begin(2, 3);
        assert!(!p.predicted(7, 0, 100));
    }

    #[test]
    fn only_the_frame_before_counts() {
        let mut p = Predictor::default();
        p.begin(1, 1);
        p.note(7, 0, 100, 100, 1);
        p.begin(2, 1);
        assert!(p.predicted(7, 50, 60));
        assert!(!p.predicted(7, 100, 200), "touching is not overlapping");
        p.begin(3, 1);
        assert!(!p.predicted(7, 50, 60), "frame 2 refreshed nothing");
    }

    #[test]
    fn what_is_kept_is_bounded() {
        let mut p = Predictor::default();
        p.begin(1, 1);
        for i in 0..MOST + 10 {
            p.note(1, i, i + 1, 1, 1);
        }
        assert_eq!(p.now[&(0, 1)][&1].len(), MOST);
    }

    #[test]
    fn the_line_reads_per_frame() {
        let s = Stats { submissions: 20, refreshes: 400, worked: 100, worked_us: 20_000, predicted: 80, predicted_us: 15_000, bytes: 4_000_000, predicted_bytes: 3_000_000 };
        let line = text(&s, 2.0);
        assert!(line.contains("10 submissions"), "{line}");
        assert!(line.contains("in 10.0 ms"), "{line}");
        assert!(line.contains("predicted 40 of those (75 % of their time, 7.5 ms; 75 % of their bytes)"), "{line}");
    }
}
