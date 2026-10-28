//! `boltget-testserver`: serves the test server's routes on a fixed address, for
//! benchmarks that compare BoltGet with other download managers on a local,
//! reproducible server (see `bench/README.md`).
//!
//! ```text
//! boltget-testserver [--port 8090] [--bind 127.0.0.1]
//! ```
//!
//! It prints the base URL and runs until Ctrl-C.
// A command-line tool prints to stdout by design.
#![allow(clippy::print_stdout)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use boltget_testserver::TestServer;

const USAGE: &str = "usage: boltget-testserver [--port N] [--bind ADDR]";

fn parse_args() -> Result<SocketAddr, String> {
    let mut ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut port = 8090u16;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--port" | "-p" => {
                port = value()?.parse().map_err(|e| format!("--port: {e}"))?;
            }
            "--bind" => ip = value()?.parse().map_err(|e| format!("--bind: {e}"))?,
            "--help" | "-h" => return Err(USAGE.to_owned()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(SocketAddr::new(ip, port))
}

#[tokio::main]
async fn main() -> ExitCode {
    let addr = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let server = match TestServer::start_on(addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("boltget-testserver: cannot listen on {addr}: {e}");
            return ExitCode::from(1);
        }
    };
    println!("{}", server.url(""));
    let _ = tokio::signal::ctrl_c().await;
    ExitCode::SUCCESS
}
