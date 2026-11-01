# bench

Benchmark harness. Method and published results: [docs/BENCHMARK.md](../docs/BENCHMARK.md).

| File | What |
|---|---|
| `targets.toml` | Versioned target list: public speed-test files and local lab scenarios |
| `idm-compare.ps1` | Boltget vs Internet Download Manager, interleaved, on the same targets (Windows) |
| `results/<date>-<what>/` | Raw data of published runs: every run, the summary, the machine and tool settings |

## Boltget only

```sh
cargo build --release -p boltget-cli -p boltget-testserver

# Public targets (5 measured runs and 1 warm-up each, adaptive and 8 connections):
target/release/boltget bench --targets bench/targets.toml -t cachefly-100mb -c auto,8

# Lab targets need a local test server. --tls-port adds the HTTPS/HTTP/2
# listener the lab-h2-* targets reach through {lab}/tls/...; --ca-out writes
# its generated CA, which bench must trust with --ca-cert.
target/release/boltget-testserver --port 8090 --tls-port 8443 --ca-out lab-ca.pem
target/release/boltget bench --targets bench/targets.toml -t 'lab-*' --lab http://127.0.0.1:8090 --ca-cert lab-ca.pem --csv runs.csv
```

`boltget help bench` documents the options, the statistics and the `--json` format.

Lab shaping is done by the test server at the application layer (query
parameters, documented in `crates/testserver/src/lib.rs`): `throttle` per
connection, `latency_ms` per round trip, `conn_limit` connections per server.
Packet loss is not simulated; it needs the planned second-machine lab with
`tc netem` (docs/ENGINE.md, "Benchmarks plan"). The `lab-h2-*` targets do not
work in `idm-compare.ps1`: IDM does not trust the lab CA.

## Against IDM

With IDM installed and running (Windows PowerShell 5.1):

```powershell
cargo build --release -p boltget-cli -p boltget-testserver
powershell -ExecutionPolicy Bypass -File bench\idm-compare.ps1 -Dir $env:TEMP\boltget-bench -Out bench\results\<date>-idm
```

The script does not change IDM's settings; it records them. It hands links to
IDM through IDM's COM interface (what browser integrations use) and detects
completion from the final file, so IDM's own time between accepting a link and
closing the file counts. Each IDM run adds an entry to IDM's download list.

Public targets are other people's servers: keep runs few, and do not loop.
