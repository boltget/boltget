//! Kill-and-resume harness (`docs/ENGINE.md`, "Testing strategy"): starts
//! `boltget get` as a child process against the local test server, kills it
//! (`TerminateProcess` on Windows, `SIGKILL` on Unix) at random moments, resumes
//! it with `boltget resume`, kills that too, and so on; the last resume runs to
//! completion and the file must be byte-identical (SHA-256) to the source.
//!
//! After every kill the harness also checks the checkpoint invariant from the
//! outside: every byte the database claims is done must be in the partial file.
//!
//! - `BOLTGET_CRASH_CYCLES`: number of kills (default 16; CI nightly / manual
//!   runs use 1000).
//! - `BOLTGET_CRASH_JOBS`: downloads run in parallel (default 4).
//! - `BOLTGET_CRASH_SEED`: RNG seed, to reproduce a run (printed on start).
//! - `BOLTGET_CRASH_DEBUG`: print what the database holds after every kill.

// Helpers outside `#[test]` functions panic on failure just like the tests do.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use boltget_testserver::{TestServer, byte_at, content};
use sha2::{Digest, Sha256};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// What the database says about the one download in a state directory.
#[derive(Debug)]
struct Row {
    id: u64,
    state: String,
    temp_path: Option<String>,
    downloaded: u64,
    segments: Vec<(u64, u64)>, // (start, done)
}

fn read_row(state: &Path) -> Option<Row> {
    let db = state.join(boltget_engine::DB_FILE);
    if !db.exists() {
        return None;
    }
    // The child is dead, so nobody owns the database.
    let conn = rusqlite::Connection::open(&db).unwrap();
    let has_table: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'download'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if has_table == 0 {
        return None;
    }
    let row = conn
        .query_row(
            "SELECT id, state, temp_path, downloaded FROM download ORDER BY id LIMIT 1",
            [],
            |r| {
                Ok(Row {
                    id: r.get::<_, i64>(0)?.unsigned_abs(),
                    state: r.get(1)?,
                    temp_path: r.get(2)?,
                    downloaded: r.get::<_, i64>(3)?.unsigned_abs(),
                    segments: Vec::new(),
                })
            },
        )
        .ok()?;
    let mut stmt = conn
        .prepare("SELECT start, done FROM segment WHERE download_id = ?1 ORDER BY start")
        .unwrap();
    let segments = stmt
        .query_map([i64::try_from(row.id).unwrap()], |r| {
            Ok((
                r.get::<_, i64>(0)?.unsigned_abs(),
                r.get::<_, i64>(1)?.unsigned_abs(),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    Some(Row { segments, ..row })
}

/// Every checkpointed byte must be on disk with the right value.
fn check_invariant(row: &Row, seed: u64) -> u64 {
    let Some(temp) = &row.temp_path else {
        return 0;
    };
    let claimed: u64 = row.segments.iter().map(|(_, d)| d).sum();
    if claimed == 0 {
        return 0;
    }
    let Ok(data) = std::fs::read(temp) else {
        // Renamed already (killed after the final rename): checked at the end.
        return claimed;
    };
    for &(start, done) in &row.segments {
        for o in start..start + done {
            assert_eq!(
                data[o as usize],
                byte_at(seed, o),
                "checkpoint claims byte {o} but the partial file disagrees ({row:?})"
            );
        }
    }
    claimed
}

#[derive(Debug, Default)]
struct Stats {
    kills: AtomicUsize,
    before_record: AtomicUsize,
    without_progress: AtomicUsize,
    with_progress: AtomicUsize,
    after_rename: AtomicUsize,
    finished_first: AtomicUsize,
    resumed_with_progress: AtomicUsize,
    downloads: AtomicUsize,
}

struct Plan {
    seed: u64,
    size: u64,
    url: String,
    connections: u32,
    flush_ms: u64,
    kills: usize,
    /// Rough time a run needs for the whole file, for choosing kill times.
    full_ms: u64,
}

/// The finished file must be byte-identical, under its requested name, with
/// nothing left over, and recorded as completed.
fn verify(plan: &Plan, target: &Path, out: &Path, state: &Path, path: &Path) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len() as u64, plan.size);
    assert_eq!(
        Sha256::digest(&data),
        Sha256::digest(content(plan.seed, plan.size)),
        "content mismatch after {} kills",
        plan.kills
    );
    assert_eq!(path, target, "the download moved to another name");
    let row = read_row(state).unwrap();
    assert_eq!(row.state, "completed");
    assert!(row.temp_path.is_none());
    let leftovers: Vec<_> = std::fs::read_dir(out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "stray files: {leftovers:?}");
}

/// Runs one download through up to `plan.kills` kills and a final resume.
/// Returns the number of kills counted in `stats.kills` (fewer if it
/// finished first).
fn run_download(plan: &Plan, rng: &mut fastrand::Rng, stats: &Stats) -> usize {
    let out = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let target = out.path().join("file.bin");
    let mut id: Option<u64> = None;
    let mut saved = 0u64;
    let mut remaining_ms = plan.full_ms;
    let mut killed = 0;
    for attempt in 0..=plan.kills {
        let last = attempt == plan.kills;
        if id.is_some() && saved > 0 {
            stats.resumed_with_progress.fetch_add(1, Ordering::Relaxed);
        }
        let mut args: Vec<String> = match id {
            None => vec![
                "get".into(),
                plan.url.clone(),
                "-o".into(),
                target.display().to_string(),
                "-c".into(),
                plan.connections.to_string(),
            ],
            Some(id) => vec!["resume".into(), id.to_string()],
        };
        args.extend([
            "--json".into(),
            "--flush-interval-ms".into(),
            plan.flush_ms.to_string(),
            "--min-split-size".into(),
            (256 * KIB).to_string(),
        ]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_boltget"))
            .args(&args)
            .env("BOLTGET_DATA_DIR", state.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        });
        let stderr = child.stderr.take().unwrap();
        let err_reader = std::thread::spawn(move || {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        });
        if last {
            let started = Instant::now();
            let status = loop {
                if let Some(s) = child.try_wait().unwrap() {
                    break s;
                }
                if started.elapsed() > Duration::from_secs(180) {
                    child.kill().unwrap();
                    panic!("final resume of {args:?} did not finish");
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let lines = reader.join().unwrap();
            let errs = err_reader.join().unwrap();
            assert_eq!(
                status.code(),
                Some(0),
                "final run {args:?} failed: {errs:?} / last lines {:?}",
                lines.iter().rev().take(3).collect::<Vec<_>>()
            );
            let done: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
            assert_eq!(done["event"], "completed", "{done}");
            let path = PathBuf::from(done["path"].as_str().unwrap());
            verify(plan, &target, out.path(), state.path(), &path);
            stats.downloads.fetch_add(1, Ordering::Relaxed);
            return killed;
        }

        // Kill at a random moment: mostly while transferring, sometimes during
        // start-up (engine open, probe), sometimes right around the end.
        let delay = match rng.u8(0..10) {
            0 | 1 => rng.u64(0..150),
            2 => remaining_ms + rng.u64(0..200),
            _ => rng.u64(0..remaining_ms.max(1) + 150),
        };
        std::thread::sleep(Duration::from_millis(delay));
        let finished = child.try_wait().unwrap().is_some();
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        let errors = err_reader.join().unwrap_or_default();
        if !finished {
            stats.kills.fetch_add(1, Ordering::Relaxed);
            killed += 1;
        }
        if std::env::var_os("BOLTGET_CRASH_DEBUG").is_some() {
            eprintln!(
                "kill after {delay} ms ({args:?}): {:?} {errors:?}",
                read_row(state.path())
            );
        }

        match read_row(state.path()) {
            None => {
                stats.before_record.fetch_add(1, Ordering::Relaxed);
            }
            Some(row) if row.state == "completed" => {
                // It finished before the kill: nothing left to resume.
                stats.finished_first.fetch_add(1, Ordering::Relaxed);
                verify(plan, &target, out.path(), state.path(), &target);
                stats.downloads.fetch_add(1, Ordering::Relaxed);
                // The kill may have landed after the completion record.
                return killed;
            }
            Some(row) => {
                let partial_exists = row
                    .temp_path
                    .as_deref()
                    .is_some_and(|t| Path::new(t).exists());
                if !partial_exists && row.downloaded == plan.size {
                    // Killed between the final rename and the completion record.
                    stats.after_rename.fetch_add(1, Ordering::Relaxed);
                } else if check_invariant(&row, plan.seed) > 0 {
                    stats.with_progress.fetch_add(1, Ordering::Relaxed);
                } else {
                    stats.without_progress.fetch_add(1, Ordering::Relaxed);
                }
                saved = row.downloaded;
                let left = plan.size.saturating_sub(row.downloaded);
                remaining_ms = (plan.full_ms * left / plan.size).max(50);
                id = Some(row.id);
            }
        }
    }
    unreachable!("the last attempt always returns")
}

#[tokio::test(flavor = "multi_thread")]
async fn killed_downloads_resume_byte_identical() {
    let srv = Arc::new(TestServer::start().await);
    let cycles = env_num("BOLTGET_CRASH_CYCLES", 16) as usize;
    let jobs = env_num("BOLTGET_CRASH_JOBS", 4).max(1) as usize;
    let seed = env_num("BOLTGET_CRASH_SEED", fastrand::u64(..));
    eprintln!("crash harness: {cycles} kills, {jobs} jobs, BOLTGET_CRASH_SEED={seed}");

    let stats = Arc::new(Stats::default());
    let planned = Arc::new(AtomicUsize::new(0));
    let next_download = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let mut workers = Vec::new();
    for job in 0..jobs {
        let srv = srv.clone();
        let stats = stats.clone();
        let planned = planned.clone();
        let next_download = next_download.clone();
        workers.push(tokio::task::spawn_blocking(move || {
            let mut rng = fastrand::Rng::with_seed(seed.wrapping_add(job as u64 * 7919));
            loop {
                // Claim up to 5 kills for the next download.
                let want = rng.usize(1..=5);
                let got = loop {
                    let cur = planned.load(Ordering::SeqCst);
                    if cur >= cycles {
                        // Another job may still give back kills its download did
                        // not need; only stop once every kill has happened.
                        if stats.kills.load(Ordering::SeqCst) >= cycles {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    let n = want.min(cycles - cur);
                    if planned
                        .compare_exchange(cur, cur + n, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        break n;
                    }
                };
                let n = next_download.fetch_add(1, Ordering::SeqCst) as u64;
                let size = match rng.u8(0..4) {
                    0 => 300 * KIB + rng.u64(0..4096),
                    1 => MIB + rng.u64(0..65_536),
                    2 => 3 * MIB + rng.u64(0..65_536),
                    _ => 6 * MIB + rng.u64(0..65_536),
                };
                let connections = rng.u32(1..=8);
                // Each connection at a rate that makes a full run take ~1-2 s.
                let full_ms = rng.u64(1000..2000);
                let per_conn = (size * 1000 / full_ms / u64::from(connections)).max(32 * KIB);
                let file_seed = 1000 + n;
                let plan = Plan {
                    seed: file_seed,
                    size,
                    url: srv.url(&format!(
                        "/data?size={size}&seed={file_seed}&throttle={per_conn}&key=crash{n}"
                    )),
                    connections,
                    flush_ms: [20, 50, 100, 250, 2000][rng.usize(0..5)],
                    kills: got,
                    full_ms,
                };
                let done = run_download(&plan, &mut rng, &stats);
                // Kills a download did not need (it finished first) go back.
                planned.fetch_sub(got - done, Ordering::SeqCst);
            }
        }));
    }
    for w in workers {
        w.await.unwrap();
    }
    let s = &stats;
    let load = |a: &AtomicUsize| a.load(Ordering::Relaxed);
    eprintln!(
        "crash harness: {} kills over {} downloads in {:.1}s, all byte-identical \
         (killed before the record existed: {}, without checkpointed progress: {}, \
         with checkpointed progress: {}, between the final rename and the \
         completion record: {}; resumes that continued from a checkpoint: {}; \
         downloads that finished before their next kill: {})",
        load(&s.kills),
        load(&s.downloads),
        started.elapsed().as_secs_f64(),
        load(&s.before_record),
        load(&s.without_progress),
        load(&s.with_progress),
        load(&s.after_rename),
        load(&s.resumed_with_progress),
        load(&s.finished_first),
    );
    assert_eq!(load(&s.kills), cycles);
}
