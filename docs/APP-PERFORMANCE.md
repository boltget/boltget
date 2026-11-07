# Desktop app performance and footprint

Measurements of the v0.2 desktop app against the targets in [VISION.md, "What beating IDM means, measurably"](VISION.md#what-beating-idm-means-measurably), with the method and scripts to reproduce them. Download speed is measured separately, for the engine, in [BENCHMARK.md](BENCHMARK.md).

## Summary (2026-11-07, 0.2.0-preview.1)

| Measure | Result | Target | Verdict |
|---|---|---|---|
| Start to the first painted window, warm | 518 ms (empty list), 604 ms (500 downloads), median | Under 1 s on a mid-range machine with an SSD (design target) | Met |
| Start with a fresh WebView2 profile (first start after install) | 611 ms (empty), 723 ms (500 downloads), median; slowest run 1.38 s | Under 1 s | Met at the median; the slowest first start missed |
| First frame that shows the list (500 downloads) | About 565 ms after the process starts; the list is drawn 20 to 30 ms after the window's first frame | – | – |
| 500 downloads, 20 running, progress updates | 0 long tasks, no frame over 13 ms at 165 Hz; with the CPU slowed 4×, still no frame over 12.2 ms | No dropped frames on a mid-range laptop (design target) | Met (see "Smoothness" for what was dropped) |
| Installer | 4.96 MiB (5,205,086 bytes), unsigned | Under 15 MB (design target) | Met |
| Installed | 18.5 MiB (`Boltget.exe` 18.3 MiB, `uninstall.exe` 0.2 MiB) | – | – |
| Idle memory, window shown (30 s idle) | Private bytes 172 MiB (empty list) to 189 MiB (500 downloads) over 7 processes; `Boltget.exe` itself 6 to 7 MiB | Lower than IDM on the same machine (design target) | **Missed**: IDM idles at 9.4 MiB private, 20 MiB working set |
| Idle memory, hidden in the tray (30 s) | Working set 54 MiB (from 402 MiB shown), private bytes unchanged at 179 MiB | Lower than IDM | **Missed** |
| CPU with 20 downloads running, window shown | 2.3% of one core for all processes, 0.2% of the machine | – | – |

## The machine

AMD Ryzen 5 5500 (6 cores, 12 threads), 16 GB RAM, NVMe SSD (MLD M300), NVIDIA GeForce RTX 3050, a 1920 × 1080 display at 165 Hz and 100% scaling, Windows 11 Home 25H2 (build 26200), Microsoft Edge WebView2 Runtime 156.0.4314.58. A desktop rather than a laptop; the 4× CPU slowdown stands in for a mid-range laptop, as in [APP-DESIGN.md](APP-DESIGN.md#the-list-and-the-details-pane-as-built).

## Method

All runs use the release build that the installer contains (`pnpm --filter @boltget/desktop installer`, `target/x86_64-pc-windows-msvc/release/Boltget.exe`, version 0.2.0-preview.1), started directly with scratch directories: `BOLTGET_DATA_DIR` for the downloads list (honoured by release builds since this step) and `WEBVIEW2_USER_DATA_FOLDER` for the WebView's profile. Downloads come from `boltget-testserver` on `127.0.0.1:8090`.

**The 500 downloads.** Added through the app's own commands: 200 files of 64 KiB that complete at once (the history) and 300 files of 2 GiB served at 128 KiB/s per connection with 2 connections each, in the Main queue set to run 20 at once, so 20 run and 280 wait.

**Start-up.** A PowerShell script starts `Boltget.exe` and polls every 5 ms until the process has a visible top-level window titled "Boltget"; the app shows its window only after the page has applied the theme, loaded the fonts and drawn one frame, so this is the first painted window. Five runs with the WebView2 profile deleted before each (the first start after installing) and seven warm runs, 3 s apart, each after the previous instance was ended. The page also records two `performance` marks, `boltget:shown` (the first frame) and `boltget:list` (the first frame after the engine's snapshot arrived); with the DevTools protocol enabled they are read after start and added to `performance.timeOrigin` minus the process start time. A cold start after a reboot, with nothing in the file cache, was not measured.

**Smoothness.** With the window at 1184 × 771 and 20 downloads running, a script in the page records every `requestAnimationFrame` interval and every long task (`PerformanceObserver`, over 50 ms) for 10 s, once with the list still (progress updates only) and once flinging it up and down at 3,600 px/s, each at normal speed and with the CPU slowed 4× (`Emulation.setCPUThrottlingRate`). It runs in the real WebView2 through its DevTools protocol (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`).

| Run (10 s) | Frames | Median interval | Slowest frame | Frames over 25 ms | Dropped at 165 Hz | Long tasks |
|---|---|---|---|---|---|---|
| Progress only | 1,651 | 6.1 ms | 6.2 ms | 0 | 0 | 0 |
| Fling | 1,650 | 6.1 ms | 12.1 ms | 0 | 1 | 0 |
| Progress only, CPU 4× slower | 1,649 | 6.1 ms | 12.2 ms | 0 | 2 | 0 |
| Fling, CPU 4× slower | 1,591 | 6.1 ms | 12.2 ms | 0 | 60 | 0 |

"Dropped" counts missed 165 Hz refreshes. Even the worst case kept every frame under 12.3 ms, so on a 60 Hz display (16.7 ms per frame) no frame would have been dropped. The list keeps 23 to 30 rows in the page whatever its length.

**Memory.** 30 s after start with the window shown and nothing running (the queue stopped), then 30 s after closing the window to the tray, summed over `Boltget.exe` and every process under it: the WebView2 browser, renderer, GPU, two utility processes and crashpad. Working set double-counts the pages these processes share, so private bytes is the fairer total. IDM was running idle in the tray on the same machine and was only read, not started.

| Process | Working set, shown | Private, shown | Working set, hidden | Private, hidden |
|---|---|---|---|---|
| `Boltget.exe` | 31.5 MiB | 6.9 MiB | 31.5 MiB | 7.3 MiB |
| WebView2 browser | 119 MiB | 34.6 MiB | 3.1 MiB | 33.5 MiB |
| GPU process | 75.6 MiB | 84.6 MiB | 0 | 77.7 MiB |
| Renderer (the page) | 88.9 MiB | 39.7 MiB | 3.0 MiB | 38.7 MiB |
| Utility × 2, crashpad | 86.8 MiB | 22.6 MiB | 16.1 MiB | 22.1 MiB |
| **Total (500 downloads)** | **402 MiB** | **188 MiB** | **54 MiB** | **179 MiB** |
| Total (empty list) | 386 MiB | 172 MiB | – | – |

The hidden column is with this step's change: while the window is in the tray, the app asks WebView2 for its low memory target (`ICoreWebView2_19::MemoryUsageTargetLevel`), which trims the WebView's working set from about 370 MiB to about 20 MiB; showing the window sets it back to normal. Before the change the hidden window kept 404 MiB working set. Private bytes, memory committed to the processes, do not move: the trimmed pages are kept for when the window comes back.

**To reproduce:** the scripts are in [`bench/app/`](../bench/app/README.md): `startup.ps1` (start-up polling), `mem.ps1` (the memory sum), `seed.mjs` (the 500 downloads), `smooth.js` with `frames.mjs` (the frame recorder) and `cdp.mjs` (a small DevTools protocol client). Use scratch directories, never your own downloads.

## The gap to IDM's memory, and the plan

IDM is a single native Win32 process; Boltget is a Rust process plus Microsoft's WebView2, a Chromium with its GPU, renderer and utility processes. `Boltget.exe` alone (the engine, the tray, the pipe) idles at 7 MiB private, below IDM's 9.4 MiB, but the WebView adds about 165 MiB. That is the cost of [ADR-003](TECH-STACK.md#adr-003-tauri-2-for-the-desktop-app) and it is far less than an Electron app's, but it does not meet "lower than IDM". The plan, in order of payoff:

1. **Close the WebView after some minutes in the tray** (already the plan in [APP-DESIGN.md](APP-DESIGN.md#the-tray-the-taskbar-and-notifications-as-built)) and recreate it when the window is shown. Hidden, Boltget would then be `Boltget.exe` alone, about 7 MiB private, under IDM. The cost is a start-up-like delay (about 0.5 s) when the window comes back from the tray; the tray, toasts, the pipe and downloads never need the page. v0.5 performance pass.
2. **Start with Windows into the tray without creating the WebView at all** (`--background`), for the same reason.
3. Revisit the GPU process (about 80 MiB private) once the WebView can be closed: software compositing would save it but cost smoothness while the window is shown.

The VISION target stays a design target until the IDM comparison is part of the v0.5 benchmark re-run, where both are measured the same way, including during a 10-connection download.
