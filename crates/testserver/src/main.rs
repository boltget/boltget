//! `boltget-testserver`: serves the test server's routes on a fixed address, for
//! benchmarks that compare Boltget with other download managers on a local,
//! reproducible server (see `bench/README.md`).
//!
//! ```text
//! boltget-testserver [--port 8090] [--bind 127.0.0.1] [--tls] [--tls-port N]
//!                    [--alpn both|h2|http1] [--ca-out FILE]
//! ```
//!
//! `--tls` makes the main port HTTPS. `--tls-port N` keeps it plain and adds an
//! HTTPS listener on N, reachable from plain URLs through `/tls/...` redirects.
//! Either way the certificate comes from a CA generated at start-up; `--ca-out`
//! writes that CA certificate (PEM) for `boltget --ca-cert`.
//!
//! It prints the base URL(s) and runs until Ctrl-C.
// A command-line tool prints to stdout by design.
#![allow(clippy::print_stdout)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;

use boltget_testserver::{Alpn, TestServer};

const USAGE: &str = "usage: boltget-testserver [--port N] [--bind ADDR] [--tls] [--tls-port N] \
                     [--alpn both|h2|http1] [--ca-out FILE]";

#[derive(Debug)]
struct Args {
    addr: SocketAddr,
    tls: bool,
    tls_port: Option<u16>,
    alpn: Alpn,
    ca_out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut port = 8090u16;
    let mut tls = false;
    let mut tls_port = None;
    let mut alpn = Alpn::Both;
    let mut ca_out = None;
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
            "--tls" => tls = true,
            "--tls-port" => {
                tls_port = Some(value()?.parse().map_err(|e| format!("--tls-port: {e}"))?);
            }
            "--alpn" => alpn = value()?.parse().map_err(|e| format!("--alpn: {e}"))?,
            "--ca-out" => ca_out = Some(PathBuf::from(value()?)),
            "--help" | "-h" => return Err(USAGE.to_owned()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if tls && tls_port.is_some() {
        return Err(format!("--tls and --tls-port exclude each other\n{USAGE}"));
    }
    if ca_out.is_some() && !tls && tls_port.is_none() {
        return Err(format!("--ca-out needs --tls or --tls-port\n{USAGE}"));
    }
    Ok(Args {
        addr: SocketAddr::new(ip, port),
        tls,
        tls_port,
        alpn,
        ca_out,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let addr = args.addr;
    let started = if args.tls {
        TestServer::start_tls_on(addr, args.alpn).await
    } else {
        TestServer::start_on(addr).await
    };
    let mut server = match started {
        Ok(s) => s,
        Err(e) => {
            eprintln!("boltget-testserver: cannot listen on {addr}: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(port) = args.tls_port {
        let tls_addr = SocketAddr::new(addr.ip(), port);
        if let Err(e) = server.listen_tls(tls_addr, args.alpn).await {
            eprintln!("boltget-testserver: cannot listen on {tls_addr}: {e}");
            return ExitCode::from(1);
        }
    }
    if let (Some(path), Some(pem)) = (&args.ca_out, server.ca_cert_pem())
        && let Err(e) = std::fs::write(path, pem)
    {
        eprintln!("boltget-testserver: cannot write {}: {e}", path.display());
        return ExitCode::from(1);
    }
    println!("{}", server.url(""));
    if args.tls_port.is_some()
        && let Some(url) = server.tls_url("")
    {
        println!("{url}");
    }
    let _ = tokio::signal::ctrl_c().await;
    ExitCode::SUCCESS
}
