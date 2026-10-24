//! The interactive progress display.

use std::path::PathBuf;
use std::time::Duration;

use boltget_engine::{Completed, DownloadHandle, Error, Progress, segment::Segment};
use indicatif::{HumanBytes, HumanDuration, ProgressBar, ProgressStyle};

use crate::Speed;

const MAP_WIDTH: usize = 48;

/// Runs the download with a live display on stderr. Returns the partial path on error.
pub(crate) async fn run(
    handle: DownloadHandle,
    show: bool,
) -> Result<Completed, (Error, Option<PathBuf>)> {
    let pb = if show {
        ProgressBar::new_spinner()
    } else {
        ProgressBar::hidden()
    };
    pb.set_style(probing_style());
    pb.set_message("connecting");
    pb.enable_steady_tick(Duration::from_millis(120));

    let mut speed = Speed::default();
    let mut styled = false;
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tick.tick().await;
        let p = handle.progress();
        if !styled && (p.total.is_some() || p.downloaded > 0) {
            styled = true;
            match p.total {
                Some(t) => {
                    pb.set_length(t);
                    pb.set_style(bar_style());
                }
                None => pb.set_style(stream_style()),
            }
            if let Some(info) = handle.info() {
                pb.println(format!(
                    "  {} -> {}",
                    info.probe.filename,
                    info.path
                        .parent()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default()
                ));
            }
        }
        update(&pb, &p, speed.sample(p.downloaded));
        if p.state.is_terminal() {
            break;
        }
    }
    let partial = handle.info().map(|i| i.partial_path.clone());
    pb.finish_and_clear();
    handle.wait().await.map_err(|e| (e, partial))
}

fn update(pb: &ProgressBar, p: &Progress, bps: f64) {
    pb.set_position(p.downloaded);
    let eta = match p.total {
        Some(t) if bps > 0.0 => HumanDuration(Duration::from_secs_f64(
            t.saturating_sub(p.downloaded) as f64 / bps,
        ))
        .to_string(),
        _ => "-".into(),
    };
    pb.set_prefix(format!(
        "{:>10}/s  ETA {eta:<11} {:>2} conn",
        HumanBytes(bps as u64).to_string(),
        p.connections
    ));
    pb.set_message(segment_map(&p.segments, p.total));
}

/// A one-line map of the file: each cell is a slice of the file, shaded by how
/// much of it has arrived.
pub(crate) fn segment_map(segments: &[Segment], total: Option<u64>) -> String {
    let Some(total) = total.filter(|t| *t > 0) else {
        return String::new();
    };
    let cell = (total as f64 / MAP_WIDTH as f64).max(1.0);
    let mut out = String::with_capacity(MAP_WIDTH * 3);
    for i in 0..MAP_WIDTH {
        let a = (i as f64 * cell) as u64;
        let b = (((i + 1) as f64 * cell) as u64).min(total);
        if a >= b {
            break;
        }
        let covered: u64 = segments
            .iter()
            .map(|s| {
                let done_end = s.start + s.received;
                done_end.min(b).saturating_sub(s.start.max(a))
            })
            .sum();
        let f = covered as f64 / (b - a) as f64;
        out.push(match f {
            f if f >= 1.0 => '█',
            f if f >= 0.66 => '▓',
            f if f >= 0.33 => '▒',
            f if f > 0.0 => '░',
            _ => '·',
        });
    }
    format!("[{out}] {} segments", segments.len())
}

fn probing_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg}").expect("valid template")
}

fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.cyan} {bar:32.cyan/blue} {bytes:>10} / {total_bytes:<10} {percent:>3}%  {prefix}\n  {wide_msg:.dim}",
    )
    .expect("valid template")
    .progress_chars("█▉▊▋▌▍▎▏ ")
}

fn stream_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {bytes:>10}  {prefix}").expect("valid template")
}

/// The final line after a successful download.
pub(crate) fn summary(done: &Completed) {
    let secs = done.elapsed.as_secs_f64().max(1e-3);
    println!(
        "Saved {} ({}) in {:.1}s, {}/s average, {} connection{}, {} segment{}",
        done.path.display(),
        HumanBytes(done.bytes),
        secs,
        HumanBytes((done.bytes as f64 / secs) as u64),
        done.max_connections,
        if done.max_connections == 1 { "" } else { "s" },
        done.segments,
        if done.segments == 1 { "" } else { "s" },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, received: u64) -> Segment {
        Segment {
            idx: 0,
            start,
            end,
            received,
            durable: 0,
            worker: None,
        }
    }

    #[test]
    fn map_shading() {
        let total = 96;
        let m = segment_map(&[seg(0, 48, 48), seg(48, 96, 0)], Some(total));
        assert!(
            m.starts_with(&format!("[{}{}]", "█".repeat(24), "·".repeat(24))),
            "{m}"
        );
        assert!(m.ends_with("2 segments"));
        assert_eq!(segment_map(&[], None), "");
    }
}
