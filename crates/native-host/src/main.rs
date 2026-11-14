//! `boltget-native-host`: started by the browser with the calling
//! extension's origin as its argument (Chromium also passes
//! `--parent-window=<id>` on Windows; Firefox the manifest's path and the
//! extension's id), it serves the extension until the browser closes stdin.
//!
//! For the installer and for support:
//!
//! - `--register`: writes the manifests next to this executable and the
//!   browsers' registry keys (development builds register the development
//!   host name, release builds the release one).
//! - `--unregister`: removes them.
//! - `--status`: prints each browser's registration.
//!
//! Exit codes: 0 when the browser closed the connection (or a command
//! succeeded), 1 when the stream broke or a command failed.

use std::io::Write;
use std::process::ExitCode;

use boltget_native_host::registration::{self, Flavor, Registration};
use boltget_native_host::{Session, Settings};

fn main() -> ExitCode {
    let first = std::env::args_os().nth(1);
    match first.as_ref().and_then(|a| a.to_str()) {
        Some("--register") => command(|r, reg| r.register(reg).map(|()| String::new())),
        Some("--unregister") => command(|r, reg| r.unregister(reg).map(|()| String::new())),
        Some("--status") => command(|r, reg| {
            Ok(r.status(reg).iter().fold(String::new(), |out, s| {
                out + &format!("{:?}: {:?}\n", s.browser, s.state)
            }))
        }),
        // The browser's own arguments, whatever they are.
        _ => host(),
    }
}

fn host() -> ExitCode {
    let mut session = match Session::new(Settings::from_env()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("boltget-native-host: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    match session.serve(&mut input, &mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // The browser logs a host's stderr; stdout is the protocol.
            eprintln!("boltget-native-host: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Runs a registration command for this executable, printing what it
/// returns.
fn command(
    f: impl FnOnce(
        &Registration,
        &dyn registration::Registry,
    ) -> Result<String, registration::RegistrationError>,
) -> ExitCode {
    let result = std::env::current_exe()
        .map_err(|e| registration::RegistrationError::NoHost(e.to_string().into()))
        .and_then(|exe| {
            let r = Registration::new(exe, Flavor::current());
            let reg = registration::system()?;
            f(&r, &reg)
        });
    match result {
        Ok(text) => {
            let _ = std::io::stdout().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("boltget-native-host: {e}");
            ExitCode::FAILURE
        }
    }
}
