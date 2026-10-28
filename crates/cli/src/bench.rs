//! `boltget bench`: repeated, interleaved downloads with timing statistics.
//!
//! Each run uses a fresh engine with nothing recorded (like `boltget get
//! --no-state`), downloads into a scratch directory and deletes the file
//! afterwards. Targets come from URLs on the command line or from a versioned
//! targets file (`bench/targets.toml` in the repository). See
//! `docs/BENCHMARK.md` for the method and the comparison with IDM.

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use boltget_engine::{
    AddRequest, Checksum, ConflictPolicy, Destination, Engine, EngineConfig, Error, ProxyRules,
    RequestContext,
};
use clap::Args;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const HELP: &str = "\
Each run downloads the target with a fresh engine (nothing is recorded) into
--dir, measures it, and deletes the file. Warm-up runs are reported but not
counted. Within a round every connection setting runs once, and the order
rotates from round to round (A B, B A, A B, ...) so drift in the network does
not favour one setting.

Targets file (TOML, `version = 1`):
  [[target]]
  name   = \"cachefly-100mb\"                     # unique, used by --target
  url    = \"http://cachefly.cachefly.net/100mb.test\"
  size   = 104857600                            # optional: checked after each run
  sha256 = \"<64 hex digits>\"                    # optional: verified while downloading
  notes  = \"...\"                                # optional, free text
A URL may start with {lab}, which --lab replaces with the base URL of a local
`boltget-testserver`; such targets are skipped without --lab.

Times are wall-clock from adding the download to the file being complete under
its final name (including probing and checksum verification). ttfb is the time
to the first downloaded byte and tail the time from 95% to 100%, both sampled
every 20 ms. Statistics: median, interquartile range (q1-q3, linear
interpolation) and min-max over the measured runs that completed; MiB/s is the
size divided by the median time.

With --json, stdout carries one JSON object per line: a `run` object per run
{target, setting, phase, round, ok, bytes, wall_ms, engine_ms, ttfb_ms, tail_ms,
mib_s, max_connections, segments, sha256, error} and then a `summary` object per
target and setting {target, url, setting, runs, failed, median_s, q1_s, q3_s,
min_s, max_s, median_mib_s, median_ttfb_ms, median_tail_ms, max_connections}.
With --list --json, one `target` object per target {name, url, size, sha256,
notes}, with the full URL.

Exit code 0 if every run completed; otherwise the exit code of the first
failure (see `boltget help get`; 9 also covers a size mismatch). A bad command
line or targets file exits with 2.";

/// Arguments of `boltget bench`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent command-line flags"
)]
#[derive(Debug, Args)]
pub(crate) struct BenchArgs {
    /// URLs to measure (in addition to the targets file, if any).
    #[arg(value_name = "URL")]
    urls: Vec<String>,

    /// Targets file (TOML), for example bench/targets.toml.
    #[arg(long, value_name = "FILE")]
    targets: Option<PathBuf>,

    /// Only the targets with this name (repeatable; a trailing `*` matches a
    /// prefix).
    #[arg(short = 't', long = "target", value_name = "NAME")]
    only: Vec<String>,

    /// Measured runs per target and connection setting.
    #[arg(short = 'n', long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=1000))]
    runs: u32,

    /// Warm-up runs per target and setting, not counted in the statistics.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(0..=100))]
    warmup: u32,

    /// Connection settings to compare, comma-separated: `auto` (adaptive, the
    /// default of `boltget get`) or a fixed count 1-32. Example: auto,1,8
    #[arg(short, long, value_name = "LIST", value_delimiter = ',', default_value = "auto", value_parser = parse_setting)]
    connections: Vec<Setting>,

    /// Base URL of a local `boltget-testserver`, substituted for {lab} in
    /// target URLs.
    #[arg(long, value_name = "URL")]
    lab: Option<String>,

    /// Directory to download into (default: a new directory under the system
    /// temporary directory, removed afterwards).
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,

    /// Keep the downloaded files instead of deleting each one after its run.
    #[arg(long)]
    keep: bool,

    /// Print line-delimited JSON (`run` and `summary` objects) instead of a table.
    #[arg(long)]
    json: bool,

    /// Also write every run to this CSV file.
    #[arg(long, value_name = "FILE")]
    csv: Option<PathBuf>,

    /// List the selected targets and exit without downloading.
    #[arg(long)]
    list: bool,

    /// Connect directly, ignoring proxy environment variables.
    #[arg(long)]
    no_proxy: bool,
}

/// A connection setting: adaptive, or a fixed count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Setting {
    Auto,
    Fixed(u32),
}

impl std::fmt::Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setting::Auto => f.write_str("auto"),
            Setting::Fixed(n) => write!(f, "{n}"),
        }
    }
}

fn parse_setting(s: &str) -> Result<Setting, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("adaptive") {
        return Ok(Setting::Auto);
    }
    match s.parse::<u32>() {
        Ok(n @ 1..=32) => Ok(Setting::Fixed(n)),
        _ => Err(format!("{s:?} is not `auto` or a connection count 1-32")),
    }
}

/// The targets file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    version: u32,
    #[serde(default, rename = "target")]
    targets: Vec<Target>,
}

/// One thing to download.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    name: String,
    url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

const LAB: &str = "{lab}";

fn parse_targets(text: &str) -> Result<Vec<Target>> {
    let file: TargetsFile = toml::from_str(text).context("invalid targets file")?;
    if file.version != 1 {
        bail!(
            "targets file version {} is not supported (expected 1)",
            file.version
        );
    }
    let mut seen = std::collections::HashSet::new();
    for t in &file.targets {
        if t.name.is_empty()
            || !t
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            bail!(
                "target name {:?} must be letters, digits, '-', '_' or '.'",
                t.name
            );
        }
        if !seen.insert(t.name.as_str()) {
            bail!("target name {:?} appears twice", t.name);
        }
        if let Some(h) = &t.sha256
            && (h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            bail!("target {:?}: sha256 must be 64 hex digits", t.name);
        }
    }
    Ok(file.targets)
}

/// A target name for a URL given on the command line.
fn name_for_url(url: &str, index: usize) -> String {
    let last = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if clean.is_empty() {
        format!("url{}", index + 1)
    } else {
        format!("url{}-{clean}", index + 1)
    }
}

fn selected(name: &str, only: &[String]) -> bool {
    only.is_empty()
        || only.iter().any(|o| match o.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == o,
        })
}

/// The targets to run, with `{lab}` resolved; lab targets without `--lab` are
/// skipped (returned as the second value).
fn resolve(
    mut targets: Vec<Target>,
    only: &[String],
    lab: Option<&str>,
) -> Result<(Vec<Target>, Vec<String>)> {
    for o in only {
        if !targets
            .iter()
            .any(|t| selected(&t.name, std::slice::from_ref(o)))
        {
            bail!("no target matches --target {o:?}");
        }
    }
    targets.retain(|t| selected(&t.name, only));
    let mut skipped = Vec::new();
    let mut out = Vec::new();
    for mut t in targets {
        if let Some(rest) = t.url.strip_prefix(LAB) {
            let Some(base) = lab else {
                skipped.push(t.name);
                continue;
            };
            t.url = format!("{}{rest}", base.trim_end_matches('/'));
        } else if t.url.contains(LAB) {
            bail!("target {:?}: {LAB} must start the URL", t.name);
        }
        out.push(t);
    }
    Ok((out, skipped))
}

/// Statistics over a sample of durations, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Stats {
    median: f64,
    q1: f64,
    q3: f64,
    min: f64,
    max: f64,
}

/// The `p`-quantile (0..=1) of sorted values, by linear interpolation between
/// the closest ranks.
fn quantile(sorted: &[f64], p: f64) -> f64 {
    match sorted.len() {
        0 => f64::NAN,
        1 => sorted[0],
        n => {
            let pos = p.clamp(0.0, 1.0) * (n - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
        }
    }
}

fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    Some(Stats {
        median: quantile(&v, 0.5),
        q1: quantile(&v, 0.25),
        q3: quantile(&v, 0.75),
        min: v[0],
        max: v[v.len() - 1],
    })
}

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut v: Vec<f64> = values.collect();
    v.sort_by(f64::total_cmp);
    (!v.is_empty()).then(|| quantile(&v, 0.5))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Warmup,
    Measured,
}

/// One run, as printed with `--json` and written to the CSV file.
#[derive(Debug, Clone, Serialize)]
struct RunRecord {
    event: &'static str,
    target: String,
    setting: String,
    phase: Phase,
    /// 1-based round within the phase.
    round: u32,
    ok: bool,
    bytes: u64,
    wall_ms: f64,
    engine_ms: Option<u64>,
    ttfb_ms: Option<f64>,
    tail_ms: Option<f64>,
    mib_s: Option<f64>,
    max_connections: Option<u32>,
    segments: Option<usize>,
    sha256: Option<String>,
    error: Option<String>,
    #[serde(skip)]
    exit_code: u8,
}

#[derive(Debug, Serialize)]
struct Summary<'a> {
    event: &'static str,
    target: &'a str,
    url: String,
    setting: String,
    runs: usize,
    failed: usize,
    median_s: Option<f64>,
    q1_s: Option<f64>,
    q3_s: Option<f64>,
    min_s: Option<f64>,
    max_s: Option<f64>,
    median_mib_s: Option<f64>,
    median_ttfb_ms: Option<f64>,
    median_tail_ms: Option<f64>,
    max_connections: Option<u32>,
}

fn summarize<'a>(target: &'a Target, setting: Setting, runs: &[&RunRecord]) -> Summary<'a> {
    let ok: Vec<&&RunRecord> = runs.iter().filter(|r| r.ok).collect();
    let secs: Vec<f64> = ok.iter().map(|r| r.wall_ms / 1000.0).collect();
    let st = stats(&secs);
    let bytes = target.size.or_else(|| ok.first().map(|r| r.bytes));
    Summary {
        event: "summary",
        target: &target.name,
        url: shown_url(&target.url),
        setting: setting.to_string(),
        runs: ok.len(),
        failed: runs.len() - ok.len(),
        median_s: st.map(|s| s.median),
        q1_s: st.map(|s| s.q1),
        q3_s: st.map(|s| s.q3),
        min_s: st.map(|s| s.min),
        max_s: st.map(|s| s.max),
        median_mib_s: st
            .zip(bytes)
            .filter(|(s, _)| s.median > 0.0)
            .map(|(s, b)| b as f64 / MIB / s.median),
        median_ttfb_ms: median(ok.iter().filter_map(|r| r.ttfb_ms)),
        median_tail_ms: median(ok.iter().filter_map(|r| r.tail_ms)),
        max_connections: ok.iter().filter_map(|r| r.max_connections).max(),
    }
}

const MIB: f64 = 1024.0 * 1024.0;

/// A URL for display, without its query (it may be signed).
fn shown_url(url: &str) -> String {
    RequestContext::new(url)
        .map_or_else(|_| url.to_owned(), |c| boltget_engine::redact_url(c.url()))
}

/// What a run is measured against.
struct RunPlan<'a> {
    target: &'a Target,
    setting: Setting,
    phase: Phase,
    round: u32,
}

fn engine_config(setting: Setting, no_proxy: bool) -> EngineConfig {
    let mut cfg = EngineConfig::default();
    if !no_proxy {
        cfg.proxy = ProxyRules::from_env();
    }
    if let Setting::Fixed(n) = setting {
        cfg.initial_connections = n;
        cfg.max_connections = cfg.max_connections.max(n);
    }
    cfg
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let mut s = String::with_capacity(64);
    for b in h.finalize() {
        let _ = write!(s, "{b:02x}");
    }
    Ok(s)
}

/// Runs one download and measures it. Never returns an error: a failure is a
/// result too.
async fn run_once(plan: &RunPlan<'_>, dir: &Path, keep: bool, no_proxy: bool) -> RunRecord {
    let t = plan.target;
    let mut rec = RunRecord {
        event: "run",
        target: t.name.clone(),
        setting: plan.setting.to_string(),
        phase: plan.phase,
        round: plan.round,
        ok: false,
        bytes: 0,
        wall_ms: 0.0,
        engine_ms: None,
        ttfb_ms: None,
        tail_ms: None,
        mib_s: None,
        max_connections: None,
        segments: None,
        sha256: None,
        error: None,
        exit_code: 0,
    };
    let fail = |mut rec: RunRecord, e: &Error| {
        rec.exit_code = crate::exit_code_for(e);
        rec.error = Some(e.to_string());
        rec
    };
    let path = dir.join(format!("{}.c{}.bin", t.name, plan.setting));
    let _ = std::fs::remove_file(&path);
    let engine = match Engine::new(engine_config(plan.setting, no_proxy)) {
        Ok(e) => e,
        Err(e) => return fail(rec, &e),
    };
    let mut ctx = match RequestContext::new(&t.url) {
        Ok(c) => c,
        Err(e) => return fail(rec, &e),
    };
    if no_proxy {
        ctx.set_proxy(boltget_engine::ProxyChoice::Direct);
    }
    let mut req =
        AddRequest::new(ctx, Destination::File(path.clone())).conflict(ConflictPolicy::Overwrite);
    if let Setting::Fixed(n) = plan.setting {
        req = req.connections(n);
    }
    if let Some(c) = t
        .sha256
        .as_deref()
        .and_then(|h| Checksum::parse(&format!("sha256:{h}")).ok())
    {
        req = req.checksum(c);
    }

    let start = Instant::now();
    let handle = engine.add(req);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    let mut ttfb = None;
    let mut t95 = None;
    loop {
        tick.tick().await;
        let p = handle.progress();
        if ttfb.is_none() && p.downloaded > 0 {
            ttfb = Some(start.elapsed());
        }
        if t95.is_none()
            && let Some(total) = p.total.filter(|&n| n > 0)
            && u128::from(p.downloaded) * 20 >= u128::from(total) * 19
        {
            t95 = Some(start.elapsed());
        }
        if p.state.is_terminal() {
            break;
        }
    }
    let partial = handle.info().map(|i| i.partial_path.clone());
    let result = handle.wait().await;
    let wall = start.elapsed();
    engine.shutdown().await;
    rec.wall_ms = wall.as_secs_f64() * 1000.0;
    rec.ttfb_ms = ttfb.map(|d| d.as_secs_f64() * 1000.0);

    let done = match result {
        Ok(d) => d,
        Err(e) => {
            if let Some(p) = partial {
                let _ = std::fs::remove_file(p);
            }
            return fail(rec, &e);
        }
    };
    rec.bytes = done.bytes;
    rec.engine_ms = Some(done.elapsed.as_millis() as u64);
    rec.tail_ms = t95.map(|t| (wall.saturating_sub(t)).as_secs_f64() * 1000.0);
    rec.mib_s = Some(done.bytes as f64 / MIB / wall.as_secs_f64().max(1e-6));
    rec.max_connections = Some(done.max_connections);
    rec.segments = Some(done.segments);
    rec.ok = true;
    if let Some(size) = t.size
        && size != done.bytes
    {
        rec.ok = false;
        rec.exit_code = 9;
        rec.error = Some(format!("size mismatch: expected {size} bytes"));
    }
    // With a known hash the engine verified it while downloading; otherwise
    // hash the file (after the clock stopped) so runs can be compared.
    rec.sha256 = match &t.sha256 {
        Some(h) if done.verified.is_some() => Some(h.to_ascii_lowercase()),
        _ => match sha256_file(&done.path) {
            Ok(h) => Some(h),
            Err(e) => {
                rec.ok = false;
                rec.exit_code = 5;
                rec.error = Some(format!("cannot read the downloaded file: {e}"));
                None
            }
        },
    };
    if !keep {
        let _ = std::fs::remove_file(&done.path);
    }
    rec
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

const CSV_HEADER: &str = "target,setting,phase,round,ok,bytes,wall_ms,engine_ms,ttfb_ms,tail_ms,mib_s,max_connections,segments,sha256,error";

fn csv_row(r: &RunRecord) -> String {
    fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map(|v| v.to_string()).unwrap_or_default()
    }
    fn ms(v: Option<f64>) -> String {
        v.map(|v| format!("{v:.1}")).unwrap_or_default()
    }
    [
        csv_field(&r.target),
        csv_field(&r.setting),
        match r.phase {
            Phase::Warmup => "warmup".into(),
            Phase::Measured => "measured".into(),
        },
        r.round.to_string(),
        r.ok.to_string(),
        r.bytes.to_string(),
        format!("{:.1}", r.wall_ms),
        opt(r.engine_ms),
        ms(r.ttfb_ms),
        ms(r.tail_ms),
        r.mib_s.map(|v| format!("{v:.2}")).unwrap_or_default(),
        opt(r.max_connections),
        opt(r.segments),
        r.sha256.clone().unwrap_or_default(),
        csv_field(r.error.as_deref().unwrap_or("")),
    ]
    .join(",")
}

fn fmt_s(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), |v| format!("{v:.2}"))
}

fn table(summaries: &[Summary<'_>]) -> String {
    let header = [
        "target",
        "conns",
        "ok",
        "median s",
        "q1-q3 s",
        "min-max s",
        "MiB/s",
        "ttfb ms",
        "tail ms",
        "max conns",
    ];
    let rows: Vec<[String; 10]> = summaries
        .iter()
        .map(|s| {
            [
                s.target.to_owned(),
                s.setting.clone(),
                if s.failed == 0 {
                    s.runs.to_string()
                } else {
                    format!("{}/{}", s.runs, s.runs + s.failed)
                },
                fmt_s(s.median_s),
                match (s.q1_s, s.q3_s) {
                    (Some(a), Some(b)) => format!("{a:.2}-{b:.2}"),
                    _ => "-".into(),
                },
                match (s.min_s, s.max_s) {
                    (Some(a), Some(b)) => format!("{a:.2}-{b:.2}"),
                    _ => "-".into(),
                },
                s.median_mib_s
                    .map_or_else(|| "-".into(), |v| format!("{v:.1}")),
                s.median_ttfb_ms
                    .map_or_else(|| "-".into(), |v| format!("{v:.0}")),
                s.median_tail_ms
                    .map_or_else(|| "-".into(), |v| format!("{v:.0}")),
                s.max_connections
                    .map_or_else(|| "-".into(), |v| v.to_string()),
            ]
        })
        .collect();
    let mut widths = header.map(str::len);
    for r in &rows {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.len());
        }
    }
    let mut out = String::new();
    let line = |out: &mut String, cells: &[&str]| {
        for (i, (c, w)) in cells.iter().zip(widths).enumerate() {
            // Text columns left-aligned, numbers right-aligned.
            let _ = if i < 2 {
                write!(out, "{c:<w$}")
            } else {
                write!(out, "{c:>w$}")
            };
            out.push_str(if i + 1 < cells.len() { "  " } else { "\n" });
        }
    };
    line(&mut out, &header);
    for r in &rows {
        line(&mut out, &r.each_ref().map(String::as_str));
    }
    out
}

fn progress_line(r: &RunRecord, runs: u32) -> String {
    let phase = match r.phase {
        Phase::Warmup => "warm-up".to_owned(),
        Phase::Measured => format!("run {}/{runs}", r.round),
    };
    if r.ok {
        format!(
            "  {} c={} {phase}: {:.2} s, {:.1} MiB/s, {} conns",
            r.target,
            r.setting,
            r.wall_ms / 1000.0,
            r.mib_s.unwrap_or(0.0),
            r.max_connections.unwrap_or(0)
        )
    } else {
        format!(
            "  {} c={} {phase}: FAILED after {:.2} s: {}",
            r.target,
            r.setting,
            r.wall_ms / 1000.0,
            r.error.as_deref().unwrap_or("unknown error")
        )
    }
}

fn load_targets(args: &BenchArgs) -> Result<Vec<Target>> {
    let mut targets = match &args.targets {
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("cannot read {}", p.display()))?;
            parse_targets(&text).with_context(|| p.display().to_string())?
        }
        None => Vec::new(),
    };
    for (i, url) in args.urls.iter().enumerate() {
        targets.push(Target {
            name: name_for_url(url, i),
            url: url.clone(),
            size: None,
            sha256: None,
            notes: None,
        });
    }
    if targets.is_empty() {
        bail!("nothing to measure: give URLs or --targets <FILE>");
    }
    Ok(targets)
}

/// Runs `boltget bench`. Returns the exit code.
pub(crate) async fn run(args: BenchArgs) -> u8 {
    let usage = |e: &anyhow::Error| {
        eprintln!("boltget: {e:#}");
        2
    };
    let (targets, skipped) =
        match load_targets(&args).and_then(|t| resolve(t, &args.only, args.lab.as_deref())) {
            Ok(t) => t,
            Err(e) => return usage(&e),
        };
    for s in &skipped {
        eprintln!("boltget: skipping {s}: it needs --lab <URL of a boltget-testserver>");
    }
    if args.list {
        for t in &targets {
            if args.json {
                // Full URLs: scripts (bench/idm-compare.ps1) drive other tools with them.
                crate::json::emit(&serde_json::json!({
                    "event": "target",
                    "name": t.name,
                    "url": t.url,
                    "size": t.size,
                    "sha256": t.sha256,
                    "notes": t.notes,
                }));
            } else {
                let size = t.size.map(|s| format!(" ({s} bytes)")).unwrap_or_default();
                println!("{}  {}{size}", t.name, shown_url(&t.url));
            }
        }
        return 0;
    }
    if targets.is_empty() {
        return usage(&anyhow::anyhow!("no targets left to run"));
    }
    let mut settings = args.connections.clone();
    settings.dedup();

    let (dir, own_dir) = match &args.dir {
        Some(d) => (d.clone(), false),
        None => (
            std::env::temp_dir().join(format!("boltget-bench-{}", std::process::id())),
            true,
        ),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("boltget: cannot create {}: {e}", dir.display());
        return 5;
    }
    let mut csv = match &args.csv {
        Some(p) => match std::fs::File::create(p) {
            Ok(mut f) => {
                let _ = writeln!(f, "{CSV_HEADER}");
                Some(f)
            }
            Err(e) => {
                eprintln!("boltget: cannot create {}: {e}", p.display());
                return 5;
            }
        },
        None => None,
    };

    let mut records: Vec<RunRecord> = Vec::new();
    let mut first_failure = None;
    for target in &targets {
        if !args.json {
            eprintln!("{} ({})", target.name, shown_url(&target.url));
        }
        let rounds = (0..args.warmup)
            .map(|r| (Phase::Warmup, r))
            .chain((0..args.runs).map(|r| (Phase::Measured, r)));
        for (phase, round) in rounds {
            // Rotate the order each round: A B, B A, A B, ...
            let k = settings.len();
            for i in 0..k {
                let setting = settings[(i + round as usize) % k];
                let plan = RunPlan {
                    target,
                    setting,
                    phase,
                    round: round + 1,
                };
                let rec = run_once(&plan, &dir, args.keep, args.no_proxy).await;
                if args.json {
                    crate::json::emit(&rec);
                } else {
                    eprintln!("{}", progress_line(&rec, args.runs));
                }
                if let Some(f) = &mut csv {
                    let _ = writeln!(f, "{}", csv_row(&rec));
                }
                if !rec.ok && first_failure.is_none() {
                    first_failure = Some(rec.exit_code.max(1));
                }
                records.push(rec);
            }
        }
    }

    let mut summaries = Vec::new();
    for target in &targets {
        for &setting in &settings {
            let tag = setting.to_string();
            let runs: Vec<&RunRecord> = records
                .iter()
                .filter(|r| r.target == target.name && r.setting == tag)
                .filter(|r| r.phase == Phase::Measured)
                .collect();
            summaries.push(summarize(target, setting, &runs));
        }
    }
    if args.json {
        for s in &summaries {
            crate::json::emit(s);
        }
    } else {
        println!();
        print!("{}", table(&summaries));
    }
    if own_dir && !args.keep {
        let _ = std::fs::remove_dir(&dir);
    }
    first_failure.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((quantile(&v, 0.5) - 2.5).abs() < 1e-9);
        assert!((quantile(&v, 0.25) - 1.75).abs() < 1e-9);
        assert!((quantile(&v, 0.75) - 3.25).abs() < 1e-9);
        assert!((quantile(&[7.0], 0.9) - 7.0).abs() < 1e-9);
        assert!(quantile(&[], 0.5).is_nan());
        let s = stats(&[5.0, 1.0, 3.0]).unwrap();
        assert_eq!(
            s,
            Stats {
                median: 3.0,
                q1: 2.0,
                q3: 4.0,
                min: 1.0,
                max: 5.0
            }
        );
        assert_eq!(stats(&[]), None);
    }

    #[test]
    fn settings_parse() {
        assert_eq!(parse_setting("auto"), Ok(Setting::Auto));
        assert_eq!(parse_setting(" Adaptive "), Ok(Setting::Auto));
        assert_eq!(parse_setting("8"), Ok(Setting::Fixed(8)));
        assert!(parse_setting("0").is_err());
        assert!(parse_setting("33").is_err());
        assert!(parse_setting("x").is_err());
        assert_eq!(Setting::Fixed(4).to_string(), "4");
    }

    const FILE: &str = r#"
version = 1

[[target]]
name = "cdn-100mb"
url = "http://example.test/100mb"
size = 104857600
sha256 = "ab00000000000000000000000000000000000000000000000000000000000000"
notes = "a CDN"

[[target]]
name = "lab-throttled"
url = "{lab}/data?size=1024&throttle=10"
"#;

    #[test]
    fn targets_file_parses_and_resolves() {
        let t = parse_targets(FILE).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].size, Some(104_857_600));
        assert_eq!(t[1].sha256, None);

        let (run, skipped) = resolve(t.clone(), &[], None).unwrap();
        assert_eq!(run.len(), 1);
        assert_eq!(skipped, ["lab-throttled"]);

        let (run, skipped) = resolve(t.clone(), &[], Some("http://127.0.0.1:8090/")).unwrap();
        assert_eq!(skipped, Vec::<String>::new());
        assert_eq!(
            run[1].url,
            "http://127.0.0.1:8090/data?size=1024&throttle=10"
        );

        let (run, _) = resolve(t.clone(), &["lab-*".into()], Some("http://h")).unwrap();
        assert_eq!(run.len(), 1);
        assert_eq!(run[0].name, "lab-throttled");
        assert!(resolve(t, &["nope".into()], None).is_err());
    }

    #[test]
    fn targets_file_errors() {
        assert!(parse_targets("version = 2").is_err());
        assert!(
            parse_targets("version = 1\n[[target]]\nname = \"a\"\nurl = \"u\"\nsiz = 1").is_err()
        );
        let dup = "version = 1\n[[target]]\nname = \"a\"\nurl = \"u\"\n[[target]]\nname = \"a\"\nurl = \"v\"";
        assert!(parse_targets(dup).is_err());
        let bad_hash = "version = 1\n[[target]]\nname = \"a\"\nurl = \"u\"\nsha256 = \"xyz\"";
        assert!(parse_targets(bad_hash).is_err());
        let bad_name = "version = 1\n[[target]]\nname = \"a b\"\nurl = \"u\"";
        assert!(parse_targets(bad_name).is_err());
        let mid_lab =
            parse_targets("version = 1\n[[target]]\nname = \"a\"\nurl = \"http://x/{lab}\"")
                .unwrap();
        assert!(resolve(mid_lab, &[], Some("http://h")).is_err());
    }

    #[test]
    fn repository_targets_file_is_valid() {
        let t = parse_targets(include_str!("../../../bench/targets.toml")).unwrap();
        assert!(t.len() >= 5);
        assert!(t.iter().all(|t| t.size.is_some()));
        let (_, skipped) = resolve(t, &[], None).unwrap();
        assert!(skipped.iter().all(|s| s.starts_with("lab-")));
    }

    #[test]
    fn url_names() {
        assert_eq!(
            name_for_url("https://h.test/files/100MB.bin?x=1", 0),
            "url1-100MB.bin"
        );
        assert_eq!(name_for_url("https://h.test/", 1), "url2-h.test");
        assert_eq!(name_for_url("https://h.test/a b", 2), "url3-a_b");
    }

    #[test]
    fn csv_quotes_fields() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(CSV_HEADER.split(',').count(), 15);
    }
}
