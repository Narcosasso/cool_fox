//! `victory compare`: the frame lines of runs' logs side by side, so that
//! two settings are compared on the same numbers instead of by eye.
//!
//! A run logs, every five seconds, its frame line (fps, ms a frame, ms
//! preparing submissions, ms on the GPU, draws a frame, over the last
//! second) and its card memory line (in use, budget, the PC's memory shared
//! with the card in use). Menus and loading screens draw little, so only
//! intervals with at least `--min-draws` draws a frame count, as in
//! docs/bug-journal.md (1,000).
//!
//! Only the standard library: the parsing and the statistics are tested on
//! any host.

use std::path::{Path, PathBuf};

/// One frame line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub ms: f64,
    pub preparing: f64,
    pub gpu: f64,
    pub draws: f64,
}

/// One card memory line, MB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Memory {
    pub used: f64,
    pub budget: f64,
    pub shared: f64,
}

/// The number that ends right before byte `end` of `line`.
fn number_before(line: &str, end: usize) -> Option<f64> {
    let head = &line[..end];
    let start = head
        .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or(0, |i| i + 1);
    head[start..].parse().ok()
}

/// The number right before the first `marker` at or after byte `from`, and
/// where the marker ends.
fn before(line: &str, from: usize, marker: &str) -> Option<(f64, usize)> {
    let at = from + line.get(from..)?.find(marker)?;
    Some((number_before(line, at)?, at + marker.len()))
}

/// The frame line's numbers, if `line` is one.
pub fn interval(line: &str) -> Option<Interval> {
    let start = line.find(" fps, ")?;
    let (ms, at) = before(line, start, " ms/frame (")?;
    let (preparing, at) = before(line, at, " preparing")?;
    let (gpu, at) = before(line, at, " on the GPU")?;
    let (draws, _) = before(line, at, " draws/frame")?;
    Some(Interval { ms, preparing, gpu, draws })
}

/// The card memory line's numbers, if `line` is one.
pub fn memory(line: &str) -> Option<Memory> {
    let start = line.find("card memory: ")?;
    let (used, at) = before(line, start, " MB in use of a ")?;
    let (budget, at) = before(line, at, " MB budget, ")?;
    let (shared, _) = before(line, at, " MB of the PC's shared memory")?;
    Some(Memory { used, budget, shared })
}

/// The refresh prediction line's numbers (`VICTORY_PREDICT=1`): ms of
/// threads' time of refreshes that copied, and the share of it predicted.
pub fn prediction(line: &str) -> Option<(f64, f64)> {
    let start = line.find("refresh prediction (")?;
    let (ms, at) = before(line, start, " ms of threads' time")?;
    let (share, _) = before(line, at, " % of their time")?;
    Some((ms, share))
}

/// The value at fraction `q` of `sorted` (nearest rank); `NaN` if empty.
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn sorted(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(f64::total_cmp);
    v
}

/// What one log says.
#[derive(Debug)]
pub struct Summary {
    pub intervals: usize,
    /// Intervals with enough draws.
    pub counted: usize,
    pub ms: [f64; 3],
    pub preparing: f64,
    pub gpu: f64,
    pub draws: f64,
    /// Peak in use, budget at that peak, peak shared, median shared.
    pub memory: Option<[f64; 4]>,
    /// Median refresh ms and median share predicted (`VICTORY_PREDICT=1`).
    pub prediction: Option<[f64; 2]>,
}

pub fn summarize(text: &str, min_draws: f64) -> Summary {
    let all: Vec<Interval> = text.lines().filter_map(interval).collect();
    let counted: Vec<&Interval> = all.iter().filter(|i| i.draws >= min_draws).collect();
    let ms = sorted(counted.iter().map(|i| i.ms));
    let median = |f: fn(&Interval) -> f64| quantile(&sorted(counted.iter().map(|i| f(i))), 0.5);
    let memory: Vec<Memory> = text.lines().filter_map(memory).collect();
    let peak = memory.iter().max_by(|a, b| a.used.total_cmp(&b.used));
    let memory = peak.map(|p| {
        let shared = sorted(memory.iter().map(|m| m.shared));
        [p.used, p.budget, shared[shared.len() - 1], quantile(&shared, 0.5)]
    });
    let predictions: Vec<(f64, f64)> = text.lines().filter_map(prediction).collect();
    let prediction = (!predictions.is_empty()).then(|| {
        [
            quantile(&sorted(predictions.iter().map(|p| p.0)), 0.5),
            quantile(&sorted(predictions.iter().map(|p| p.1)), 0.5),
        ]
    });
    Summary {
        prediction,
        intervals: all.len(),
        counted: counted.len(),
        ms: [quantile(&ms, 0.5), quantile(&ms, 0.9), quantile(&ms, 1.0)],
        preparing: median(|i| i.preparing),
        gpu: median(|i| i.gpu),
        draws: median(|i| i.draws),
        memory,
    }
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The table for `logs`; the first is the base the others are compared to.
pub fn table(logs: &[(String, Summary)]) -> String {
    let mut out = String::new();
    let width = logs.iter().map(|l| l.0.len()).max().unwrap_or(0).max(8);
    let row = |out: &mut String, label: &str, cells: Vec<String>| {
        out.push_str(&format!("{label:<26}"));
        for c in cells {
            out.push_str(&format!(" {c:>width$}"));
        }
        out.push('\n');
    };
    let cells = |f: &dyn Fn(&Summary) -> String| logs.iter().map(|l| f(&l.1)).collect::<Vec<_>>();
    row(&mut out, "", logs.iter().map(|l| l.0.clone()).collect());
    row(&mut out, "intervals (counted)", cells(&|s| format!("{} ({})", s.intervals, s.counted)));
    row(&mut out, "ms/frame median", cells(&|s| format!("{:.1}", s.ms[0])));
    row(&mut out, "ms/frame p90", cells(&|s| format!("{:.1}", s.ms[1])));
    row(&mut out, "ms/frame worst", cells(&|s| format!("{:.1}", s.ms[2])));
    row(&mut out, "preparing ms median", cells(&|s| format!("{:.1}", s.preparing)));
    row(&mut out, "on the GPU ms median", cells(&|s| format!("{:.1}", s.gpu)));
    row(&mut out, "draws/frame median", cells(&|s| format!("{:.0}", s.draws)));
    let mem = |i: usize| move |s: &Summary| s.memory.map_or("-".into(), |m| format!("{:.0}", m[i]));
    row(&mut out, "card MB peak in use", cells(&mem(0)));
    row(&mut out, "card MB budget", cells(&mem(1)));
    row(&mut out, "shared MB peak", cells(&mem(2)));
    row(&mut out, "shared MB median", cells(&mem(3)));
    let pred = |i: usize| move |s: &Summary| s.prediction.map_or("-".into(), |p| format!("{:.1}", p[i]));
    row(&mut out, "refresh ms (threads)", cells(&pred(0)));
    row(&mut out, "predicted % of it", cells(&pred(1)));
    if let Some((_, base)) = logs.first() {
        row(
            &mut out,
            "median vs first",
            cells(&|s| {
                let change = (s.ms[0] / base.ms[0] - 1.0) * 100.0;
                if change.is_finite() { format!("{change:+.1} %") } else { "-".into() }
            }),
        );
    }
    out
}

pub fn run(paths: &[PathBuf], min_draws: f64) -> std::io::Result<()> {
    let mut logs = Vec::new();
    for path in paths {
        // Logs redirected from PowerShell may be UTF-16; the lines we read
        // are ASCII either way.
        let bytes = std::fs::read(path)?;
        let text = if bytes.starts_with(&[0xff, 0xfe]) {
            let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        logs.push((name(path), summarize(&text, min_draws)));
    }
    print!("{}", table(&logs));
    println!("(intervals of one second, every five seconds; counted: at least {min_draws} draws a frame)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: &str = "2026-11-27T10:00:00Z  INFO victory_gpu::host: 18.4 fps, 54.3 ms/frame (47.9 preparing: copies 10.2 ms \
        3.1 MB, textures 4.0 ms 12.0 MB; 41.2 on the GPU; 1.1 copying back 16.0 MB), 2868 draws/frame, dropped 0 without";

    #[test]
    fn a_frame_line_is_read() {
        assert_eq!(interval(FRAME), Some(Interval { ms: 54.3, preparing: 47.9, gpu: 41.2, draws: 2868.0 }));
        assert_eq!(interval("INFO victory_gpu::host: host GPU"), None);
        // Colours around the target do not reach the numbers.
        let coloured = FRAME.replace("victory_gpu::host:", "\x1b[2mvictory_gpu::host\x1b[0m\x1b[2m:\x1b[0m");
        assert_eq!(interval(&coloured).map(|i| i.ms), Some(54.3));
    }

    #[test]
    fn a_memory_line_is_read() {
        let line = "INFO victory_gpu::host: card memory: 7012 MB in use of a 7340 MB budget, 1830 MB of the PC's shared memory in use";
        assert_eq!(memory(line), Some(Memory { used: 7012.0, budget: 7340.0, shared: 1830.0 }));
    }

    #[test]
    fn a_prediction_line_is_read() {
        let line = "INFO victory_gpu::host: refresh prediction (VICTORY_PREDICT=1), per frame: 10 submissions, 200 refreshes, \
            50 of them copied 2.0 MB in 10.0 ms of threads' time; the frame before predicted 40 of those (75 % of their time, \
            7.5 ms; 75 % of their bytes)";
        assert_eq!(prediction(line), Some((10.0, 75.0)));
        assert_eq!(prediction(FRAME), None);
    }

    #[test]
    fn quantiles_take_the_nearest_rank() {
        let v = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(quantile(&v, 0.5), 20.0);
        assert_eq!(quantile(&v, 0.9), 40.0);
        assert_eq!(quantile(&v, 0.0), 10.0);
        assert!(quantile(&[], 0.5).is_nan());
    }

    #[test]
    fn menus_do_not_count() {
        let menu = FRAME.replace("54.3 ms/frame", "33.3 ms/frame").replace("2868 draws", "120 draws");
        let slow = FRAME.replace("54.3 ms/frame", "80.0 ms/frame");
        let text = [menu.as_str(), FRAME, slow.as_str(), FRAME].join("\n");
        let s = summarize(&text, 1000.0);
        assert_eq!((s.intervals, s.counted), (4, 3));
        assert_eq!(s.ms, [54.3, 80.0, 80.0]);
        assert_eq!(s.memory, None);
    }

    #[test]
    fn the_table_compares_to_the_first() {
        let fast = FRAME.replace("54.3 ms/frame", "48.87 ms/frame");
        let logs = vec![("a.log".to_string(), summarize(FRAME, 1000.0)), ("b.log".to_string(), summarize(&fast, 1000.0))];
        let table = table(&logs);
        assert!(table.contains("-10.0 %"), "{table}");
        assert!(table.contains("+0.0 %"), "{table}");
    }
}
