# Features

This document lists every planned BoltGet feature, the milestone that targets it, and how it compares with Internet Download Manager (IDM) and three well-known reference points: Free Download Manager (FDM), Motrix and aria2.

## How to read this

- **Milestones** refer to [ROADMAP.md](ROADMAP.md): `v0.1` engine + CLI, `v0.2` desktop app, `v0.3` browser extension, `v0.4` video, `v0.5` scheduler and queues, `v1.0` public launch, `post-1.0` later.
- **Comparison cells** describe each product as we understand it from its public documentation and our own use, as of 2026. They are meant to be fair, not flattering to us.
  - `Yes` / `No` are things we are confident about.
  - `?` means we are not sure. If you know the answer, open a PR with a source.
  - `Partial` is explained in the notes.
- The **BoltGet column describes plans**, not shipped features. Nothing is shipped yet.

## Core engine

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| Multi-connection (segmented) HTTP/HTTPS downloads | Yes | v0.1 | Yes | Yes | Yes (via aria2) | Yes |
| Dynamic segmentation (re-split the largest remaining segment when a connection frees up) | Yes | v0.1 | Yes | ? | Partial (aria2 model) | Partial |
| Adaptive connection count from measured throughput | Yes | v0.1 | ? | ? | No | No |
| Max connections per download | Up to 32 (configurable, per-host caps) | v0.1 | Up to 32 | ? | ? | Up to 16 per server |
| Resume after pause, crash, reboot | Yes, checkpointed to SQLite | v0.1 | Yes | Yes | Yes | Yes (control file) |
| Crash-safe checkpoint (progress recorded only after data is durable) | Yes | v0.1 | ? | ? | ? | ? |
| Link refresh (resume with a new URL after expiry) | Yes, with size/ETag/sample-byte validation | v0.1 (CLI), v0.3 (browser-assisted) | Yes | ? | ? | Manual (`-c` with new URI) |
| Validators (`If-Range`, ETag, Last-Modified) to detect changed files | Yes | v0.1 | ? | ? | ? | Partial |
| HTTP/2 | Yes (per-connection ALPN, see [ENGINE.md](ENGINE.md#http2-considerations)) | v0.1 | ? | ? | No (aria2 is HTTP/1.1) | No |
| HTTP/3 (QUIC) | Investigate | post-1.0 | ? | ? | No | No |
| FTP / FTPS | Yes | post-1.0 (target v1.x) | Yes | Yes | Yes | Yes |
| SFTP | Maybe | post-1.0 | No | ? | ? | Yes |
| BitTorrent / magnet | Maybe | post-1.0 | No | Yes | Yes | Yes |
| Metalink / multi-source mirrors | Yes | post-1.0 | No | ? | Partial | Yes |
| Sparse-file preallocation (avoids NTFS zero-fill stalls) | Yes | v0.1 | ? | ? | ? | Yes (`--file-allocation`) |
| Global and per-download speed limits | Yes (token bucket) | v0.1 (CLI), v0.2 (UI) | Yes | Yes | Yes | Yes |
| Checksum verification (SHA-256, SHA-1, MD5, SHA-512) | Yes | v0.1 | ? | ? | ? | Yes |
| Server digest headers (`Repr-Digest`, `x-goog-hash`, `x-amz-checksum-*`) | Yes, when present | v0.5 | ? | ? | ? | No |
| Proxies (HTTP, HTTPS CONNECT, SOCKS5) | Yes | v0.1 (CLI), v0.2 (UI) | Yes | Yes | Yes | Yes |
| System proxy detection (Windows) | Yes | v0.2 | Yes | ? | ? | No |
| PAC / WPAD | Investigate | post-1.0 | ? | ? | ? | No |
| HTTP auth (Basic, Digest) | Yes | v0.1 | Yes | ? | ? | Yes |
| Uses Windows certificate store (corporate CAs) | Yes (platform verifier) | v0.1 | ? | ? | ? | Depends on build |

## Desktop app

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| GUI | Yes (Tauri 2 + Svelte 5) | v0.2 | Yes (Win32) | Yes | Yes (Electron) | No (third-party GUIs) |
| Dark theme | Yes, dark-first, plus light | v0.2 | No | ? | Yes | n/a |
| Keyboard-first operation, command palette | Yes | v0.2 | Partial | ? | ? | n/a |
| Live segment visualization | Yes | v0.2 | Yes | ? | No | n/a |
| Categories (by file type, with destination folders) | Yes | v0.2 | Yes | Yes | No | n/a |
| Queues with concurrency limits | Yes | v0.2 (basic), v0.5 (polish) | Yes | Yes | Yes | Yes (`-j`) |
| Scheduler (time windows, start/stop, shutdown after) | Yes | v0.5 | Yes | ? | No | No |
| Clipboard URL monitoring | Opt-in | v0.2 | Yes | Yes | ? | n/a |
| Batch add (URL list, numbered patterns) | Yes | v0.5 | Yes | ? | ? | Yes (input file) |
| Tray icon with download animation | Yes | v0.2 | Yes | Yes | Yes | n/a |
| Post-download actions (open, scan with Defender, run nothing else) | Open/reveal: v0.2; Defender scan: v0.5 | | Yes (virus scan) | ? | ? | Hooks |
| Single portable build | Maybe | post-1.0 | No | ? | ? | Yes |
| UI languages | English (strings in a central catalog; community translations possible post-1.0) | v0.2 | Many languages | Many | Many | n/a |
| Import IDM queue / settings | Investigate | v1.0 | n/a | ? | ? | n/a |

## Browser integration

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| Chrome, Edge, Firefox extension | Yes (WXT, Manifest V3) | v0.3 | Yes | Yes | Third-party | Third-party |
| Automatic capture by file type and size | Yes | v0.3 | Yes | Yes | n/a | n/a |
| Modifier-key bypass (let the browser handle it) | Yes (`Alt`-click, configurable) | v0.3 | Yes | ? | n/a | n/a |
| Cookie, referrer and header forwarding | Yes | v0.3 | Yes | Yes | n/a | n/a |
| Context menu: download link / all links / page media | Yes | v0.3 | Yes | Yes | n/a | n/a |
| Transport | Native messaging (no localhost server) | v0.3 | ? | ? | n/a | RPC (third-party) |
| Media detection panel | Yes (detect and hand off only) | v0.4 | Yes | ? | No | No |

Details: [BROWSER-INTEGRATION.md](BROWSER-INTEGRATION.md).

## Video and audio

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| Download from video sites | Yes, via yt-dlp sidecar | v0.4 | Yes | ? | No | No |
| HLS / DASH stream download and mux | Yes, via yt-dlp + ffmpeg | v0.4 | Yes | ? | No | No |
| Format and quality picker | Yes | v0.4 | Yes | ? | No | No |
| Audio-only extraction | Yes | v0.4 | ? | ? | No | No |
| Subtitles | Yes | v0.4 | ? | ? | No | No |
| Sidecars installed on demand, verified, self-updating | Yes | v0.4 | n/a | n/a | n/a | n/a |
| DRM-protected streams | No (non-goal) | | No | No | No | No |

Details: [VIDEO.md](VIDEO.md).

## CLI and automation

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| Command-line tool | Yes (`boltget`) | v0.1 | Partial (`idman.exe` switches) | ? | No | Yes |
| JSON output for scripting | Yes (`--json`) | v0.1 | No | ? | No | Partial (RPC) |
| Built-in benchmark command | Yes (`boltget bench`) | v0.1 | No | No | No | No |
| Hand off to the running app's queue | Yes | v0.2 | Yes | ? | No | n/a |
| RPC / local API | Investigate (authenticated named pipe only) | post-1.0 | No | ? | Yes (aria2 RPC) | Yes (JSON-RPC, XML-RPC) |

## Platform, distribution, trust

| Feature | BoltGet | Milestone | IDM | FDM | Motrix | aria2 |
|---|---|---|---|---|---|---|
| Windows | Yes | v1.0 (previews from v0.2) | Yes | Yes | Yes | Yes |
| macOS | Yes | post-1.0 | No | Yes | Yes | Yes |
| Linux | Yes | post-1.0 | No | Yes | Yes | Yes |
| Price | Free | | Paid (trial) | Free | Free | Free |
| License | GPL-3.0-or-later | | Proprietary | Proprietary (current versions) | MIT | GPL-2.0-or-later |
| Ads / bundled offers | Never | | No ads, but license nags | ? | No | No |
| Telemetry | None by default; crash reports opt-in | | ? | ? | ? | No |
| Code-signed installer | Yes (Azure Trusted Signing) | v1.0 | Yes | Yes | ? | n/a |
| Signed auto-updates | Yes (Tauri updater) | v1.0 | Yes | ? | ? | n/a |

## Explicitly out of scope

- Website grabber / offline site copier (IDM has one). Not planned for v1. Reconsider after 1.0 if there is demand.
- DRM circumvention.
- Any form of advertising, bundling, or paid tier that gates the engine.

See [VISION.md, Non-goals](VISION.md#non-goals).

## Corrections

If any cell about another product is wrong, please open an issue or PR with a link to that product's documentation. We would rather lose a row than misrepresent someone else's work.
