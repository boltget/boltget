//! Client mode (docs/ARCHITECTURE.md, "The CLI"): while the Boltget app runs,
//! it owns the engine and the database, so commands go to it over its named
//! pipe instead of failing with "locked". The app runs the download; this
//! process shows it, and Ctrl-C pauses it there.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use boltget_engine::{DownloadId, ErrorKind, RequestContext, redact_url};
use boltget_ipc::{
    AddSpec, Client, Endpoint, Finished, ProgressReport, RefreshSpec, Request, RequestSpec,
    Response, ResumeSpec, WireError,
};
use indicatif::{HumanBytes, HumanDuration, ProgressBar};
use serde_json::{Value, json};

use crate::display::{bar_style, percent, probing_style, stream_style};
use crate::json::emit;
use crate::settings::{ConfigAction, ConfigArgs, Key, show_limit};
use crate::{
    GetArgs, IdArgs, ListArgs, Rate, RefreshArgs, RemoveArgs, ResumeArgs, RunArgs, Speed,
    cookie_pairs, data_dir, exit_code_for_kind, hints, parse_headers, proxy_choice, user_auth,
};

/// Exit code when the running app cannot be talked to.
pub(crate) const EXIT_APP: u8 = 10;

fn client_name() -> String {
    format!("boltget-cli {}", env!("CARGO_PKG_VERSION"))
}

/// The running app, if one owns the database in `state` (an app that is
/// still opening it is waited for; one that waits for a standalone boltget
/// to finish sends us away). `Err` is an exit code: an app runs but cannot
/// be talked to (another protocol version, a broken pipe), which is
/// reported.
pub(crate) async fn connect(state: Option<&Path>) -> Result<Option<Connection>, u8> {
    let Ok(dir) = data_dir(state) else {
        return Ok(None);
    };
    let endpoint = match Endpoint::for_data_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(error = %e, "no pipe endpoint; running standalone");
            return Ok(None);
        }
    };
    match Client::connect(&endpoint, &client_name()).await {
        Ok(Some(client)) => Ok(Some(Connection { client, endpoint })),
        Ok(None) => Ok(None),
        Err(boltget_ipc::Error::Version(msg)) => {
            eprintln!("boltget: {msg}");
            Err(EXIT_APP)
        }
        Err(e) => {
            eprintln!("boltget: cannot talk to the running Boltget app: {e}");
            eprintln!("boltget: or run without it: quit the app, or use --state <DIR>");
            Err(EXIT_APP)
        }
    }
}

/// A connection to the app, and where to open another (Ctrl-C uses one).
pub(crate) struct Connection {
    client: Client,
    endpoint: Endpoint,
}

impl Connection {
    async fn ask(&mut self, req: &Request) -> Result<Response, u8> {
        match self.client.request(req).await {
            Ok(Response::Error { error }) => Err(refused(&error, false, None, &[])),
            Ok(resp) => Ok(resp),
            Err(e) => Err(lost(&e)),
        }
    }

    /// Like [`Connection::ask`], but reports a refusal in JSON if asked.
    async fn ask_json(
        &mut self,
        req: &Request,
        json: bool,
        id: Option<DownloadId>,
        command: &str,
    ) -> Result<Response, u8> {
        match self.client.request(req).await {
            Ok(Response::Error { error }) => {
                let hints = hints_for_wire(&error, id, command);
                Err(refused(&error, json, id, &hints))
            }
            Ok(resp) => Ok(resp),
            Err(e) => Err(lost(&e)),
        }
    }
}

fn lost(e: &boltget_ipc::Error) -> u8 {
    eprintln!("boltget: lost the connection to the Boltget app: {e}");
    EXIT_APP
}

fn unexpected(resp: &Response) -> u8 {
    eprintln!("boltget: the Boltget app gave an unexpected answer: {resp:?}");
    EXIT_APP
}

/// The exit code for an error the app reported.
fn code_for(e: &WireError) -> u8 {
    match e.error_kind() {
        Some(kind) => exit_code_for_kind(kind),
        None if e.key == boltget_ipc::proto::INVALID_REQUEST_KEY => 2,
        None if e.key == boltget_ipc::proto::VERSION_ERROR_KEY => EXIT_APP,
        None => 1,
    }
}

fn hints_for_wire(e: &WireError, id: Option<DownloadId>, command: &str) -> Vec<String> {
    match e.error_kind() {
        Some(kind) => hints(kind, &e.key, id, command),
        None => Vec::new(),
    }
}

/// Prints an error the app reported and returns its exit code.
fn refused(e: &WireError, json: bool, id: Option<DownloadId>, hints: &[String]) -> u8 {
    let code = code_for(e);
    if json {
        emit(&failed_line(e, id, code));
    } else {
        eprintln!("boltget: {}", e.message);
        for h in hints {
            eprintln!("boltget: {h}");
        }
    }
    code
}

fn failed_line(e: &WireError, id: Option<DownloadId>, code: u8) -> Value {
    let mut line = json!({
        "event": "failed",
        "error": {"kind": e.kind, "key": e.key, "message": e.message},
        "exit_code": code,
    });
    if let Some(id) = id {
        line["id"] = id.0.into();
    }
    line
}

fn usage(e: &anyhow::Error, json: bool, code: u8) -> u8 {
    if json {
        crate::json::failed_usage(e, code);
    } else {
        eprintln!("boltget: {e:#}");
    }
    code
}

/// Options that change the engine itself, which only a boltget that runs
/// its own engine can apply.
fn engine_only(run: &RunArgs) -> Option<&'static str> {
    if !run.ca_certs.is_empty() {
        Some("--ca-cert")
    } else if run.max_connections.is_some() {
        Some("--max-connections")
    } else if run.flush_interval_ms.is_some() {
        Some("--flush-interval-ms")
    } else if run.initial_connections.is_some() {
        Some("--initial-connections")
    } else if run.min_split_size.is_some() {
        Some("--min-split-size")
    } else {
        None
    }
}

fn check_engine_only(run: &RunArgs) -> Result<(), anyhow::Error> {
    match engine_only(run) {
        Some(flag) => Err(anyhow::anyhow!(
            "{flag} needs a boltget that runs the download itself, but the Boltget app is \
             running and owns your downloads; quit the app, or use --state <DIR> or --no-state"
        )),
        None => Ok(()),
    }
}

/// Headers, cookies, credentials and proxy from the command line, checked
/// as the standalone CLI checks them.
fn request_spec(run: &RunArgs, url: Option<&str>) -> Result<RequestSpec, anyhow::Error> {
    let mut spec = RequestSpec::default();
    for (name, value) in parse_headers(&run.headers)? {
        spec.headers.push((
            name.as_str().to_owned(),
            value.to_str().unwrap_or_default().to_owned(),
        ));
    }
    if let Some(v) = user_auth(run, url)? {
        spec.headers.push((
            boltget_engine::header::AUTHORIZATION.as_str().to_owned(),
            v.to_str().unwrap_or_default().to_owned(),
        ));
    }
    spec.cookies = cookie_pairs(run)?;
    spec.cookie_file = run.cookie_file.as_deref().map(absolute);
    // Validated here so a typo is a usage error, sent as typed.
    spec.proxy = match proxy_choice(run)? {
        Some(boltget_engine::ProxyChoice::Direct) => Some("direct".into()),
        Some(_) => run.proxy.clone(),
        None => None,
    };
    Ok(spec)
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn add_spec(args: &GetArgs) -> Result<AddSpec, anyhow::Error> {
    // The same checks, and the same messages, as a standalone `get`.
    crate::build_request(args)?;
    check_engine_only(&args.run)?;
    let (destination, to_file) = match &args.output {
        None => (std::env::current_dir()?, false),
        Some(p) if p.is_dir() => (absolute(p), false),
        Some(p) => (absolute(p), true),
    };
    let mut spec = AddSpec::new(args.url.clone(), destination);
    spec.to_file = to_file;
    spec.overwrite = args.overwrite;
    spec.connections = args.run.connections;
    spec.limit = args.run.limit.and_then(|Rate(r)| r);
    spec.checksum = args.run.checksum.as_ref().map(ToString::to_string);
    spec.server_digest = !args.no_server_digest;
    spec.queue.clone_from(&args.queue);
    spec.request = request_spec(&args.run, Some(&args.url))?;
    Ok(spec)
}

/// `boltget get` with the app running: the app downloads, this shows it.
pub(crate) async fn get(mut conn: Connection, args: GetArgs, verbose: u8) -> u8 {
    let json = args.run.json;
    let spec = match add_spec(&args) {
        Ok(s) => s,
        Err(e) => {
            let code = e
                .downcast_ref::<boltget_engine::Error>()
                .map_or(2, crate::exit_code_for);
            return usage(&e, json, code);
        }
    };
    let id = match conn.ask_json(&Request::Add(spec), json, None, "get").await {
        Ok(Response::Added { id }) => DownloadId(id),
        Ok(other) => return unexpected(&other),
        Err(code) => return code,
    };
    let url =
        RequestContext::new(&args.url).map_or_else(|_| args.url.clone(), |c| redact_url(c.url()));
    if json {
        emit(&json!({"event": "added", "id": id.0, "url": url}));
        if let Some(queue) = &args.queue {
            emit(&json!({"event": "queued", "id": id.0, "queue": queue}));
        }
    } else if let Some(queue) = args.queue.as_ref().filter(|_| !args.run.quiet) {
        eprintln!("Download {id} waits in queue {queue}; it starts when its turn comes");
    }
    if args.detach {
        if !json && !args.run.quiet {
            println!("Added download {id} to Boltget: {url}");
        }
        return 0;
    }
    follow(conn, id, &args.run, verbose, true).await
}

fn resume_spec(args: &ResumeArgs) -> Result<ResumeSpec, anyhow::Error> {
    check_engine_only(&args.run)?;
    let mut spec = ResumeSpec::new(args.id);
    spec.restart = args.restart;
    spec.connections = args.run.connections;
    spec.limit = args.run.limit.map(|Rate(r)| r.unwrap_or(0));
    spec.checksum = args.run.checksum.as_ref().map(ToString::to_string);
    spec.no_checksum = args.no_checksum;
    spec.request = request_spec(&args.run, None)?;
    Ok(spec)
}

/// `boltget resume` with the app running.
pub(crate) async fn resume(mut conn: Connection, args: ResumeArgs, verbose: u8) -> u8 {
    let json = args.run.json;
    let id = DownloadId(args.id);
    let spec = match resume_spec(&args) {
        Ok(s) => s,
        Err(e) => return usage(&e, json, 2),
    };
    match conn
        .ask_json(&Request::Resume(spec), json, Some(id), "resume")
        .await
    {
        Ok(Response::Ok) => {}
        Ok(other) => return unexpected(&other),
        Err(code) => return code,
    }
    if args.detach {
        if !json && !args.run.quiet {
            println!("Resumed download {id} in Boltget");
        }
        return 0;
    }
    follow(conn, id, &args.run, verbose, false).await
}

fn evidence(outcome: &Value) -> String {
    let e = &outcome["evidence"];
    match e["kind"].as_str() {
        Some("same_etag") => "same size and ETag".into(),
        Some("samples") => {
            let ranges = e["ranges"].as_u64().unwrap_or(0);
            format!(
                "same size, and {ranges} sample range{} ({}) equal the bytes already downloaded",
                if ranges == 1 { "" } else { "s" },
                HumanBytes(e["bytes"].as_u64().unwrap_or(0))
            )
        }
        Some("nothing_downloaded") => "same size (nothing was downloaded yet)".into(),
        _ => "accepted".into(),
    }
}

/// `boltget refresh` with the app running: a new link, then continue.
pub(crate) async fn refresh(mut conn: Connection, args: RefreshArgs, verbose: u8) -> u8 {
    let json = args.run.json;
    let id = DownloadId(args.id);
    if let Err(e) = RequestContext::new(&args.url) {
        return crate::report(&e, json, Some(id), &[]);
    }
    let request = match check_engine_only(&args.run).and_then(|()| request_spec(&args.run, None)) {
        Ok(r) => r,
        Err(e) => return usage(&e, json, 2),
    };
    let mut spec = RefreshSpec::new(args.id, args.url.clone());
    spec.request = request.clone();
    match conn
        .ask_json(&Request::Refresh(spec), json, Some(id), "refresh")
        .await
    {
        Ok(Response::Refreshed { outcome }) => {
            if json {
                let mut line = json!({"event": "refreshed"});
                if let (Some(line), Some(fields)) = (line.as_object_mut(), outcome.as_object()) {
                    line.extend(fields.clone());
                }
                emit(&line);
            } else if !args.run.quiet {
                println!("New link accepted: {}", evidence(&outcome));
            }
        }
        Ok(other) => return unexpected(&other),
        Err(code) => return code,
    }
    let mut resume = ResumeSpec::new(args.id);
    resume.connections = args.run.connections;
    resume.limit = args.run.limit.map(|Rate(r)| r.unwrap_or(0));
    resume.checksum = args.run.checksum.as_ref().map(ToString::to_string);
    // Cookies and credentials went with the new link; headers and the proxy
    // apply to this run, as in a standalone refresh.
    resume.request.headers = request.headers;
    resume.request.proxy = request.proxy;
    match conn
        .ask_json(&Request::Resume(resume), json, Some(id), "resume")
        .await
    {
        Ok(Response::Ok) => {}
        Ok(other) => return unexpected(&other),
        Err(code) => return code,
    }
    if args.detach {
        return 0;
    }
    follow(conn, id, &args.run, verbose, false).await
}

/// `boltget pause`: reaches a download running anywhere in the app.
pub(crate) async fn pause(mut conn: Connection, args: &IdArgs) -> u8 {
    let id = DownloadId(args.id);
    match conn.ask(&Request::Pause { id: args.id }).await {
        Ok(Response::Paused { state, .. }) if state == "paused" => {
            println!("Paused download {id}");
            0
        }
        Ok(Response::Paused { state, .. }) => {
            println!("Download {id} is not running ({state})");
            0
        }
        Ok(other) => unexpected(&other),
        Err(code) => code,
    }
}

/// `boltget remove` with the app running.
pub(crate) async fn remove(mut conn: Connection, args: &RemoveArgs) -> u8 {
    let req = Request::Remove {
        id: args.id,
        delete_file: args.delete_file,
    };
    match conn.ask(&req).await {
        Ok(Response::Removed { id, deleted }) => {
            println!("Removed download {id}");
            for p in &deleted {
                println!("Deleted {}", p.display());
            }
            0
        }
        Ok(other) => unexpected(&other),
        Err(code) => code,
    }
}

/// `boltget list` with the app running: its live view.
pub(crate) async fn list(mut conn: Connection, args: &ListArgs) -> u8 {
    match conn.ask(&Request::List).await {
        Ok(Response::Downloads { downloads }) => {
            if args.json {
                emit(&downloads);
            } else {
                let rows: Vec<crate::list::Row> =
                    downloads.iter().map(crate::list::Row::from_json).collect();
                crate::list::print_table(&rows);
            }
            0
        }
        Ok(other) => unexpected(&other),
        Err(code) => code,
    }
}

/// `boltget config` with the app running: limits change at once.
pub(crate) async fn config(mut conn: Connection, args: &ConfigArgs) -> u8 {
    let req = match &args.action {
        ConfigAction::List | ConfigAction::Get { .. } => Request::Settings,
        ConfigAction::Set { key, value } => {
            let Key::Limit = key;
            match crate::parse_limit(value) {
                Ok(Rate(r)) => Request::SetLimit { id: None, bps: r },
                Err(e) => {
                    eprintln!("boltget: {e}");
                    return 2;
                }
            }
        }
        ConfigAction::Unset { key } => {
            let Key::Limit = key;
            Request::SetLimit {
                id: None,
                bps: None,
            }
        }
    };
    match conn.ask(&req).await {
        Ok(Response::Settings { limit }) => {
            if matches!(args.action, ConfigAction::Get { .. }) {
                println!("{}", show_limit(limit));
            } else {
                println!("limit = {}", show_limit(limit));
            }
            0
        }
        Ok(other) => unexpected(&other),
        Err(code) => code,
    }
}

/// What the terminal shows of a download running in the app.
struct View {
    id: DownloadId,
    json: bool,
    bar: ProgressBar,
    styled: bool,
    speed: Speed,
    started: Instant,
    /// Bytes on disk when this run started (a checkpoint it continued from).
    resumed_from: Option<u64>,
    max_connections: u32,
    segments: usize,
    /// A new download: nothing was on disk before this run.
    fresh: bool,
}

impl View {
    fn new(id: DownloadId, run: &RunArgs, verbose: u8, fresh: bool) -> Self {
        let show = !run.json && !run.quiet && verbose == 0;
        let bar = if show {
            ProgressBar::new_spinner()
        } else {
            ProgressBar::hidden()
        };
        bar.set_style(probing_style());
        bar.set_message("connecting");
        bar.enable_steady_tick(Duration::from_millis(120));
        Self {
            id,
            json: run.json,
            bar,
            styled: false,
            speed: Speed::default(),
            started: Instant::now(),
            resumed_from: None,
            max_connections: 0,
            segments: 0,
            fresh,
        }
    }

    fn event(&mut self, event: &Value) {
        let kind = event["event"].as_str().unwrap_or_default();
        match kind {
            "probed" => {
                let name = event["info"]["probe"]["filename"].as_str().unwrap_or("");
                let dir = event["info"]["path"]
                    .as_str()
                    .map(Path::new)
                    .and_then(Path::parent)
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                self.bar.println(format!("  [{}] {name} -> {dir}", self.id));
            }
            "resumed" => {
                let done = event["downloaded"].as_u64().unwrap_or(0);
                self.resumed_from = Some(done);
                let at = match event["total"].as_u64() {
                    Some(t) => percent(done, t),
                    None => HumanBytes(done).to_string(),
                };
                self.bar.println(format!("  resuming at {at}"));
            }
            _ => {}
        }
        // `added` was printed when the download was added.
        if self.json && kind != "added" {
            emit(event);
        }
    }

    fn progress(&mut self, p: &ProgressReport) {
        let bps = self.speed.sample(p.downloaded);
        self.max_connections = self.max_connections.max(p.connections);
        self.segments = p.segments;
        if self.resumed_from.is_none() {
            // A resumed run whose `resumed` event came before we watched:
            // what it had at the first report is close enough.
            self.resumed_from = Some(if self.fresh { 0 } else { p.downloaded });
        }
        let eta_secs = p
            .total
            .filter(|_| bps > 0.0)
            .map(|t| (t.saturating_sub(p.downloaded) as f64 / bps) as u64);
        if self.json {
            emit(&json!({
                "event": "progress",
                "id": p.id,
                "downloaded": p.downloaded,
                "total": p.total,
                "speed": bps as u64,
                "eta_secs": eta_secs,
                "connections": p.connections,
                "segments": p.segments,
            }));
            return;
        }
        if !self.styled && (p.total.is_some() || p.downloaded > 0) {
            self.styled = true;
            match p.total {
                Some(t) => {
                    self.bar.set_length(t);
                    self.bar.set_style(bar_style());
                }
                None => self.bar.set_style(stream_style()),
            }
        }
        self.bar.set_position(p.downloaded);
        let eta = eta_secs.map_or_else(
            || "-".to_owned(),
            |s| HumanDuration(Duration::from_secs(s)).to_string(),
        );
        self.bar.set_prefix(format!(
            "{:>10}/s  ETA {eta:<11} {:>2} conn",
            HumanBytes(bps as u64).to_string(),
            p.connections
        ));
        self.bar.set_message(format!(
            "{} segment{} (running in the Boltget app)",
            p.segments,
            if p.segments == 1 { "" } else { "s" }
        ));
    }

    /// Reports how the download stopped; returns the exit code.
    fn finished(&self, f: &Finished) -> u8 {
        self.bar.finish_and_clear();
        match f.state.as_str() {
            "completed" => {
                self.completed(f);
                0
            }
            "paused" | "cancelled" => {
                if self.json {
                    emit(&json!({
                        "event": "paused",
                        "id": f.id,
                        "downloaded": f.downloaded,
                        "total": f.total,
                        "exit_code": 130,
                    }));
                } else {
                    let done = match f.total {
                        Some(t) => format!(
                            "{} ({} of {})",
                            percent(f.downloaded, t),
                            HumanBytes(f.downloaded),
                            HumanBytes(t)
                        ),
                        None => HumanBytes(f.downloaded).to_string(),
                    };
                    eprintln!("boltget: paused download {} at {done}", f.id);
                    eprintln!("boltget: continue with: boltget resume {}", f.id);
                }
                130
            }
            state => {
                let error = f.error.clone().unwrap_or_else(|| {
                    let (kind, key) = match state {
                        "corrupt" => (ErrorKind::Integrity, "error.checksum_mismatch"),
                        "needs_new_link" => (ErrorKind::LinkExpired, "error.link_expired"),
                        _ => (ErrorKind::Internal, "error.internal"),
                    };
                    WireError::new(kind, key, format!("the download stopped: {state}"))
                });
                let hints = hints_for_wire(&error, Some(self.id), "");
                let code = refused(&error, self.json, Some(self.id), &hints);
                if !self.json
                    && let Some(p) = f.partial_path.as_ref().filter(|p| p.exists())
                {
                    eprintln!("boltget: partial file kept at {}", p.display());
                }
                code
            }
        }
    }

    fn completed(&self, f: &Finished) {
        let bytes = f.total.unwrap_or(f.downloaded);
        let resumed_from = self.resumed_from.unwrap_or(0).min(bytes);
        let elapsed = self.started.elapsed();
        let secs = elapsed.as_secs_f64().max(1e-3);
        let fetched = bytes - resumed_from;
        let speed = (fetched as f64 / secs) as u64;
        let path = f.path.clone().unwrap_or_default();
        if self.json {
            emit(&json!({
                "event": "completed",
                "id": f.id,
                "path": path,
                "bytes": bytes,
                "resumed_from": resumed_from,
                "elapsed_ms": elapsed.as_millis() as u64,
                "speed": speed,
                "max_connections": self.max_connections,
                "segments": self.segments,
                "verified": f.verified,
            }));
            return;
        }
        let conns = format!(
            "{} connection{}, {} segment{}",
            self.max_connections,
            if self.max_connections == 1 { "" } else { "s" },
            self.segments,
            if self.segments == 1 { "" } else { "s" },
        );
        if resumed_from == 0 {
            println!(
                "Saved {} ({}) in {secs:.1}s, {}/s average, {conns}",
                path.display(),
                HumanBytes(bytes),
                HumanBytes(speed)
            );
        } else {
            println!(
                "Saved {} ({}): resumed at {}, fetched {} in {secs:.1}s, {}/s average, {conns}",
                path.display(),
                HumanBytes(bytes),
                percent(resumed_from, bytes),
                HumanBytes(fetched),
                HumanBytes(speed)
            );
        }
        if let Some(c) = &f.verified {
            println!("Checksum OK ({c})");
        }
    }
}

/// Shows a download running in the app until it stops. The first Ctrl-C
/// pauses it there (it writes its checkpoint); a second one stops watching.
async fn follow(
    mut conn: Connection,
    id: DownloadId,
    run: &RunArgs,
    verbose: u8,
    fresh: bool,
) -> u8 {
    let endpoint = conn.endpoint.clone();
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            match Client::connect(&endpoint, &client_name()).await {
                Ok(Some(mut c)) => {
                    let _ = c.request(&Request::Pause { id: id.0 }).await;
                }
                _ => eprintln!("boltget: could not reach the Boltget app to pause the download"),
            }
            if tokio::signal::ctrl_c().await.is_ok() {
                // The download keeps its last checkpoint in the app.
                std::process::exit(130);
            }
        }
    });
    let mut view = View::new(id, run, verbose, fresh);
    let mut msg = conn.client.request(&Request::Watch { id: id.0 }).await;
    let code = loop {
        match msg {
            Ok(Response::Event { event }) => view.event(&event),
            Ok(Response::Progress(p)) => view.progress(&p),
            Ok(Response::Finished(f)) => break view.finished(&f),
            Ok(Response::Error { error }) => {
                view.bar.finish_and_clear();
                break refused(&error, run.json, Some(id), &[]);
            }
            Ok(other) => {
                view.bar.finish_and_clear();
                break unexpected(&other);
            }
            Err(e) => {
                view.bar.finish_and_clear();
                eprintln!("boltget: lost the connection to the Boltget app: {e}");
                eprintln!("boltget: download {id} goes on there; see it with: boltget list");
                break EXIT_APP;
            }
        }
        msg = conn.client.next().await;
    };
    ctrl_c.abort();
    code
}
