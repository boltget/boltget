# Benchmark: BoltGet and IDM

This page records how BoltGet's download times are measured and the results of each published run, including the comparison with Internet Download Manager (IDM). The plan behind it is in [ENGINE.md, Benchmarks plan](ENGINE.md#benchmarks-plan-vs-idm); the harness is in [`bench/`](../bench/README.md).

Every number here is a measurement from the run it belongs to, with the raw data in the repository. Nothing is estimated or rounded in BoltGet's favour. A difference is called a difference only when the run-to-run spread of the two tools does not overlap.

## Method

- **Targets:** the versioned list in [`bench/targets.toml`](../bench/targets.toml). Public targets are hosting providers' speed-test files; lab targets are served on loopback by `boltget-testserver` with generated, incompressible content.
- **Rounds:** for each target, 1 warm-up round (recorded, not counted) and 5 measured rounds. In each round every configuration downloads the file once, and the order rotates from round to round (IDM, BoltGet auto, BoltGet 8; then BoltGet auto, BoltGet 8, IDM; and so on), so slow drift in the network does not favour one tool. There is a 1.5 s pause between downloads.
- **Configurations:** IDM as installed (its default of 8 connections), BoltGet default (adaptive, starts at 4 connections, up to 16), and BoltGet fixed at 8 connections (`-c 8`), the same count as IDM.
- **Timing, the same for both tools, measured from outside:**
  - IDM: from handing the link to IDM until the final file exists with the expected size and IDM has closed it. Links are handed over through IDM's COM interface (`ICIDMLinkTransmitter::SendLinkToIDM`, flag 1: no dialogs), which is what browser integrations use. The documented command line (`IDMan.exe /d <url> /p <dir> /f <file> /n`) returned at once and never started a download on this machine.
  - BoltGet: from starting `boltget get <url> -o <file> --no-state --overwrite --json` until the process exits, with the file complete under its final name. BoltGet's own `elapsed_ms` is recorded too; it is about 25 to 50 ms shorter (process start-up and exit).
  - The completion check polls every 25 ms.
- **Correctness:** every downloaded file's SHA-256 was compared with the hash in `targets.toml` after the clock stopped. All 90 downloads matched. Files were deleted after each run.
- **Lab connection counts:** the test server's `/_stats` counters were reset before each lab run and read after it, giving the connections each tool really opened at the same time and the bytes the server sent.
- **Statistics:** median, interquartile range (q1 to q3, linear interpolation) and min to max of the 5 measured runs. MiB/s is the file size divided by the median time.

## Run 2026-10-28

Raw data: [`bench/results/2026-10-28-idm/`](../bench/results/2026-10-28-idm/) (`runs.csv` with every run including warm-ups, `summary.csv`, `environment.json`, and the `targets.toml` used).

### Setup

| | |
|---|---|
| Machine | AMD Ryzen 5 5500 (12 logical CPUs), 15.9 GiB RAM, MLD M300 NVMe SSD (500 GB), Windows 11 Home 10.0.26200 |
| Network | Realtek PCIe GbE, 1 Gbps link, wired; a home line in Turkey (the earlier smoke tests in ENGINE.md found about 100 MiB/s reachable from it) |
| IDM | 6.43 build 16, trial. No connection-count or speed-limit value set in `HKCU\Software\DownloadManager`, so IDM's built-in defaults applied; the lab server confirmed 8 simultaneous connections. Settings were not changed |
| BoltGet | 0.1.0-dev, engine at commit `2266755` (the uncommitted changes in that run were the benchmark harness, the CLI's `bench` command and the test server's `/_stats` route; the engine was unchanged), release build |
| Download folder | the same NVMe SSD for both tools (IDM's temporary folder, `%APPDATA%\IDM`, is on it too) |

### Results

Wall-clock seconds, 5 measured runs each.

| Target | Tool | Median s | q1 to q3 s | min to max s | MiB/s (median) | Connections |
|---|---|---:|---:|---:|---:|---:|
| cachefly-10mb (10 MiB, HTTP) | IDM | 3.22 | 3.19 to 3.22 | 3.18 to 4.20 | 3.1 | (8 default) |
| | BoltGet auto | 0.22 | 0.21 to 0.22 | 0.21 to 0.22 | 46.3 | 3 |
| | BoltGet 8 | 0.41 | 0.41 to 0.44 | 0.41 to 0.46 | 24.1 | 3 |
| cachefly-100mb (100 MiB, HTTP) | IDM | 4.06 | 4.05 to 4.09 | 4.04 to 4.10 | 24.6 | (8 default) |
| | BoltGet auto | 2.17 | 1.04 to 2.23 | 1.04 to 2.49 | 46.1 | 4 |
| | BoltGet 8 | 2.19 | 1.09 to 2.22 | 1.07 to 2.24 | 45.7 | 8 |
| nforce-100mb (100 MiB, HTTPS) | IDM | 4.15 | 3.42 to 4.28 | 3.32 to 4.68 | 24.1 | (8 default) |
| | BoltGet auto | 3.96 | 3.92 to 5.06 | 3.27 to 5.23 | 25.2 | 4 to 8 |
| | BoltGet 8 | 4.11 | 3.86 to 4.29 | 3.35 to 6.10 | 24.3 | 8 |
| lab-throttled-100mib (2 MiB/s per connection) | IDM | 7.13 | 7.12 to 7.45 | 7.09 to 7.47 | 14.0 | 8 |
| | BoltGet auto | 6.78 | 6.76 to 6.79 | 6.74 to 6.81 | 14.7 | 16 |
| | BoltGet 8 | 6.66 | 6.65 to 6.66 | 6.65 to 6.67 | 15.0 | 8 |
| lab-1gib (1 GiB, loopback, no limit) | IDM | 4.66 | 3.76 to 17.68 | 3.42 to 21.31 | 219.5 | 8 |
| | BoltGet auto | 2.47 | 1.92 to 2.49 | 1.78 to 2.54 | 413.9 | 4 |
| | BoltGet 8 | 6.34 | 1.86 to 9.37 | 1.79 to 17.27 | 161.6 | 8 |

Connections: for lab targets, the most simultaneous connections the server saw; for public targets, BoltGet's own count (IDM's is not visible from outside there). BoltGet at `-c 8` used 3 connections on the 10 MiB file because segments are not split below the 1 MiB minimum.

### Reading the results

- **cachefly-10mb and cachefly-100mb:** BoltGet was faster in every run, with either setting. IDM took 3.2 s for 10 MiB and 4.1 s for 100 MiB, only about 0.85 s more for ten times the data, and its times were very steady (3.18 to 3.22 s for 10 MiB in four of five runs). Most of IDM's time on this host is therefore a roughly fixed cost per download rather than transfer time; this method cannot say where inside IDM that time goes.
- **cachefly-100mb, BoltGet:** the times are bimodal: about 1.04 s in rounds 1 and 2 and about 2.2 s in rounds 3 to 5, for both settings alike. That is the line or the CDN changing during the run, not the setting; IDM stayed at about 4.05 s throughout.
- **nforce-100mb:** the three configurations overlap (medians 3.96, 4.11 and 4.15 s, with spreads of 1 to 3 s). No difference is claimed.
- **lab-throttled-100mib, at the same 8 connections:** BoltGet took 6.66 s and IDM 7.13 s; every BoltGet run was faster than every IDM run. The ideal at 8 x 2 MiB/s is 6.25 s. The server also sent IDM about 150 to 160 KiB more than the file size per run, BoltGet at most 16 KiB.
- **lab-throttled-100mib, BoltGet adaptive:** it reached 16 connections but finished at 6.78 s, slower than fixed 8. With 16 connections the ideal is 3.1 s, so the controller adds connections too late on a server that throttles each connection. See below.
- **lab-1gib:** every adaptive BoltGet run (1.78 to 2.54 s) was faster than every IDM run (3.42 to 21.31 s). IDM and BoltGet at 8 connections both had very slow runs in the same rounds: in round 1, 21.3 s for IDM right after 6.3 s for BoltGet; in round 5, 17.3 s for BoltGet right before 17.7 s for IDM (BoltGet also took 9.4 s in round 2). Outliers that hit neighbouring downloads of both tools point at the machine, for example writing back 1 GiB of cached file data, rather than at either tool. The medians of those two configurations are therefore not compared, and no difference is claimed between them.

### What this run does not show

- **One machine, one line, one afternoon.** Public results depend on the route and the time of day. They show these hosts from this line, not the general case.
- **Only IDM's defaults.** The plan also calls for IDM at 32 connections; that needs a manual change in IDM's settings and was not run.
- **No shaped network.** The lab is loopback with per-connection throttling only. Added latency, packet loss, connection limits at the server and HTTP/2 are not covered yet (see [ENGINE.md](ENGINE.md#benchmarks-plan-vs-idm)).
- **External timing for IDM.** The time includes whatever IDM does between accepting a link and closing the file. There is no throughput curve, time to first byte or tail time for IDM, and no CPU or memory figures for either tool.
- **Five runs.** Enough to see the large differences above; not enough to rank configurations whose spreads overlap.
- Each IDM run added an entry to IDM's download list; they were left there.

### Follow-up

- **Adaptive ramp on throttled servers.** On `lab-throttled-100mib` the adaptive default was slower than a fixed 8 connections although it ended with twice as many. The controller's step size or interval needs work for servers that limit each connection; to be tracked as an issue with this run as the reproduction.
  *Update:* fixed after this run (faster first baseline, full doubling once a step pays off in proportion, early verdicts on clear gains). On the same lab target adaptive now takes 5.12 s against 6.65 s for `-c 8`; numbers and method are in [ENGINE.md, open question 1](ENGINE.md#open-questions). The published numbers above are unchanged.
- **Default connection count.** The data does not support changing it: the adaptive start of 4 was the best or tied configuration on four of five targets, and fixed 8 was slower on the 10 MiB file. The default stays as specified in [ENGINE.md](ENGINE.md#open-questions).
