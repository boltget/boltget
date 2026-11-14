//! One browser connection: read a message, check it, relay it to the app,
//! write the reply; until the browser closes stdin.
//!
//! Every message gets exactly one reply, and anything but
//! [`Reply::Accepted`] tells the extension to leave the download to the
//! browser. The host answers within [`Budget::connect`] plus
//! [`Budget::reply`] (14 s at most, about 2 s when it has to start the app,
//! a few milliseconds when the app runs), so the extension's own timeout
//! must be longer (docs/BROWSER-INTEGRATION.md, "Native messaging").

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use boltget_ipc::browser::{
    self, ErrorReason, HelloReply, Message, Reply, ReplyEnvelope, Unavailable, UnavailableReason,
};
use boltget_ipc::{Client, Endpoint, Error, Reach, Request, Response, Role};

use crate::launch;
use crate::stdio::{self, Frame};

/// How long the host waits for the app.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// To reach the app, starting it if needed (its engine included).
    pub connect: Duration,
    /// For the app's reply to one message, once connected.
    pub reply: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(8),
            reply: Duration::from_secs(6),
        }
    }
}

/// Where the app is and how to start it.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The app's pipe; `None` when there is no data directory.
    pub endpoint: Option<Endpoint>,
    /// The app to start when it is not running; `None` when none was found.
    pub app: Option<PathBuf>,
    /// Waiting times.
    pub budget: Budget,
}

/// How the host names itself to the app and the extension.
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One session with the browser.
#[derive(Debug)]
pub struct Session {
    settings: Settings,
    runtime: tokio::runtime::Runtime,
    /// The app, while a connection is open.
    app: Option<Client>,
}

impl Session {
    /// A session that reaches the app as `settings` say.
    pub fn new(settings: Settings) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            settings,
            runtime,
            app: None,
        })
    }

    /// Serves the browser until it closes `input`. An error means the
    /// stream itself broke (a frame cut short, over 64 MiB, or stdout
    /// gone); every message problem is answered instead.
    pub fn serve(&mut self, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
        while let Some(frame) = stdio::read_frame(input)? {
            let reply = match frame {
                Frame::Message(bytes) => self.handle(&bytes),
                Frame::Skipped(len) => ReplyEnvelope::new(
                    browser::VERSION,
                    None,
                    Reply::error(
                        ErrorReason::TooLarge,
                        format!("a message of {len} bytes exceeds the 1 MiB limit"),
                    ),
                ),
            };
            stdio::write_frame(output, &reply.to_json())?;
        }
        Ok(())
    }

    /// The reply to one message from the browser.
    pub fn handle(&mut self, bytes: &[u8]) -> ReplyEnvelope {
        let envelope = match browser::decode(bytes) {
            Ok(e) => e,
            Err(rejection) => return rejection.into_envelope(),
        };
        let (v, id) = (envelope.v, envelope.id);
        let starts = envelope.message.starts_app();
        let is_hello = matches!(envelope.message, Message::Hello(_));
        let reply = self.runtime.block_on(relay(
            &self.settings,
            &mut self.app,
            v,
            envelope.message,
            starts,
        ));
        let reply = match reply {
            // The extension asks whether Boltget is there; it is installed
            // and will start for the first download.
            Reply::Unavailable(u) if is_hello && u.reason == UnavailableReason::NotRunning => {
                Reply::Hello(hello(None))
            }
            Reply::Hello(mut h) => {
                HOST_VERSION.clone_into(&mut h.host);
                Reply::Hello(h)
            }
            other => other,
        };
        ReplyEnvelope::new(v, id, reply)
    }
}

fn hello(app: Option<String>) -> HelloReply {
    let mut h = HelloReply::new(app, None);
    HOST_VERSION.clone_into(&mut h.host);
    h
}

/// Sends one message to the app and returns its reply, or why there is
/// none. A message that may start a download always gets a fresh
/// connection, so a reply lost on a stale one can never mean the app added
/// it twice; the others reuse the open one and try once more on a new
/// connection if it broke.
async fn relay(
    settings: &Settings,
    app: &mut Option<Client>,
    v: u32,
    message: Message,
    starts: bool,
) -> Reply {
    if starts {
        *app = None;
    }
    let request = Request::Browser { v, message };
    for attempt in 0..2 {
        let reused = app.is_some();
        let client = match app {
            Some(c) => c,
            None => match connect(settings, starts).await {
                Ok(c) => app.insert(c),
                Err(u) => return Reply::Unavailable(u),
            },
        };
        match tokio::time::timeout(settings.budget.reply, client.request(&request)).await {
            Ok(Ok(Response::Browser { reply })) => return reply,
            Ok(Ok(Response::Error { error })) => {
                *app = None;
                return Reply::unavailable(
                    UnavailableReason::Mismatch,
                    format!("Boltget refused the message: {}", error.message),
                );
            }
            Ok(Ok(other)) => {
                *app = None;
                return Reply::unavailable(
                    UnavailableReason::Mismatch,
                    format!("Boltget answered {other:?}"),
                );
            }
            Ok(Err(e)) => {
                *app = None;
                if reused && !starts && attempt == 0 {
                    continue;
                }
                return Reply::unavailable(
                    UnavailableReason::Timeout,
                    format!("the connection to Boltget broke: {e}"),
                );
            }
            Err(_) => {
                *app = None;
                return Reply::unavailable(
                    UnavailableReason::Timeout,
                    "Boltget did not answer in time",
                );
            }
        }
    }
    Reply::unavailable(
        UnavailableReason::Timeout,
        "the connection to Boltget broke",
    )
}

/// Reaches the app, starting it if `start` and it is not running.
async fn connect(settings: &Settings, start: bool) -> Result<Client, Unavailable> {
    let Some(endpoint) = &settings.endpoint else {
        return Err(gone(
            UnavailableReason::NotInstalled,
            "no data directory for Boltget",
        ));
    };
    let deadline = Instant::now() + settings.budget.connect;
    if let Some(client) = reach(endpoint, deadline).await? {
        return Ok(client);
    }
    if !start {
        return Err(gone(
            UnavailableReason::NotRunning,
            "Boltget is not running",
        ));
    }
    let Some(app) = &settings.app else {
        return Err(gone(
            UnavailableReason::NotInstalled,
            "no Boltget app next to the native host",
        ));
    };
    if let Err(e) = launch::start(app) {
        return Err(gone(
            UnavailableReason::NotInstalled,
            format!("could not start {}: {e}", app.display()),
        ));
    }
    let mut pause = Duration::from_millis(50);
    loop {
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(Duration::from_millis(400));
        if let Some(client) = reach(endpoint, deadline).await? {
            return Ok(client);
        }
        if Instant::now() >= deadline {
            return Err(gone(
                UnavailableReason::Timeout,
                "Boltget did not start in time",
            ));
        }
    }
}

/// One attempt to say hello: the client, `None` when nobody serves the
/// pipe yet, or the reply that ends the attempt.
async fn reach(endpoint: &Endpoint, deadline: Instant) -> Result<Option<Client>, Unavailable> {
    let left = deadline.saturating_duration_since(Instant::now());
    let client = format!("boltget-native-host {HOST_VERSION}");
    match tokio::time::timeout(left, Client::reach(endpoint, &client, Some(Role::Browser))).await {
        Ok(Ok(Reach::Connected(c))) => Ok(Some(c)),
        Ok(Ok(Reach::Nobody)) => Ok(None),
        // `Reach::NotOwner`, and anything a later version may add.
        Ok(Ok(_)) => Err(gone(
            UnavailableReason::Busy,
            "another Boltget process owns the downloads",
        )),
        Ok(Err(Error::Version(why) | Error::Protocol(why))) => {
            Err(gone(UnavailableReason::Mismatch, why))
        }
        Ok(Err(e)) => Err(gone(
            UnavailableReason::Timeout,
            format!("could not talk to Boltget: {e}"),
        )),
        Err(_) => Err(gone(
            UnavailableReason::Timeout,
            "Boltget did not answer in time",
        )),
    }
}

fn gone(reason: UnavailableReason, message: impl Into<String>) -> Unavailable {
    Unavailable::new(reason, message)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn session(app: Option<PathBuf>) -> (Session, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            endpoint: Some(Endpoint::for_data_dir(dir.path()).unwrap()),
            app,
            budget: Budget {
                connect: Duration::from_millis(500),
                reply: Duration::from_secs(1),
            },
        };
        (Session::new(settings).unwrap(), dir)
    }

    fn frame(v: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        stdio::write_frame(&mut out, &serde_json::to_vec(v).unwrap()).unwrap();
        out
    }

    fn replies(output: &[u8]) -> Vec<Value> {
        let mut r = output;
        let mut all = Vec::new();
        while let Some(Frame::Message(m)) = stdio::read_frame(&mut r).unwrap() {
            all.push(serde_json::from_slice(&m).unwrap());
        }
        all
    }

    #[test]
    fn without_an_app_every_message_still_gets_its_reply() {
        let (mut s, dir) = session(Some(dir_path("no-such-app")));
        let mut input = [
            frame(&json!({"v": 1, "id": 1, "type": "hello", "extension": "0.3.0", "browser": "chrome"})),
            frame(&json!({"v": 1, "id": 2, "type": "status"})),
            frame(&json!({"v": 1, "id": 3, "type": "capture", "url": "https://x.test/a.iso"})),
            frame(&json!({"v": 1, "id": 4, "type": "capture", "url": "ftp://x.test/a.iso"})),
            frame(&json!({"v": 7, "id": 5, "type": "status"})),
        ]
        .concat();
        let big = browser::MAX_MESSAGE as u32 + 1;
        input.extend(big.to_ne_bytes());
        input.resize(input.len() + big as usize, b' ');
        input.extend(frame(&json!({"v": 1, "id": 6, "type": "status"})));
        let mut out = Vec::new();
        s.serve(&mut &input[..], &mut out).unwrap();
        let r = replies(&out);
        assert_eq!(r.len(), 7, "{r:#?}");
        assert_eq!(r[0]["type"], "hello");
        assert_eq!(r[0]["app"], Value::Null);
        assert_eq!(r[0]["host"], HOST_VERSION);
        assert_eq!(r[0]["id"], 1);
        assert_eq!(
            (r[1]["type"].as_str(), r[1]["reason"].as_str()),
            (Some("unavailable"), Some("not_running"))
        );
        assert_eq!(
            (r[2]["type"].as_str(), r[2]["reason"].as_str()),
            (Some("unavailable"), Some("not_installed"))
        );
        assert_eq!(
            (r[3]["type"].as_str(), r[3]["reason"].as_str()),
            (Some("error"), Some("invalid"))
        );
        assert_eq!(r[3]["id"], 4);
        assert_eq!(r[4]["reason"], "app_too_old");
        assert_eq!(r[5]["reason"], "too_large");
        assert_eq!(
            r[6]["id"], 6,
            "the stream stays in step after a skipped frame"
        );
        drop(dir);
    }

    #[test]
    fn no_app_to_start() {
        let (mut s, _dir) = session(None);
        let reply = s.handle(br#"{"v":1,"type":"add_links","links":[{"url":"https://x.test/a"}]}"#);
        assert!(matches!(
            reply.reply,
            Reply::Unavailable(ref u) if u.reason == UnavailableReason::NotInstalled
        ));
    }

    #[test]
    fn a_broken_stream_ends_the_session() {
        let (mut s, _dir) = session(None);
        let mut cut = frame(&json!({"v": 1, "type": "status"}));
        cut.truncate(cut.len() - 1);
        assert!(s.serve(&mut &cut[..], &mut Vec::new()).is_err());
        let huge = (browser::MAX_FRAME + 1).to_ne_bytes();
        assert!(s.serve(&mut &huge[..], &mut Vec::new()).is_err());
        assert!(
            s.serve(&mut &b""[..], &mut Vec::new()).is_ok(),
            "a clean end"
        );
    }

    fn dir_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("boltget-native-host-test-{name}"))
    }
}
