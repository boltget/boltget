<p align="center">
  <img src="brand/icon.svg" width="120" height="120" alt="BoltGet icon: a pixel down-arrow in a mint-to-emerald gradient on near-black">
</p>

<h1 align="center">BoltGet</h1>

<p align="center">
  <strong>Every byte, at full speed.</strong>
</p>

<p align="center">
  A download manager built for speed. Dynamic segmentation, crash-proof resume and seamless browser capture, for Windows.
</p>

<p align="center">
  <a href="https://boltget.app">boltget.app</a> ·
  <a href="docs/README.md">Docs</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a>
</p>

<p align="center">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-34D399">
  <img alt="Platform: Windows first" src="https://img.shields.io/badge/platform-Windows%20first-A7F3D0">
  <!-- Enable once the workflows exist:
  <a href="https://github.com/boltget/boltget/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/boltget/boltget/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/boltget/boltget/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/boltget/boltget"></a>
  -->
</p>

---

## What it does

BoltGet splits every file across several connections and keeps them all busy until the last byte. Its engine is written in Rust and built around three goals: speed, reliability and craft.

- **Dynamic segmentation.** A download starts with 4 connections and adds more only while they make it faster. When a connection finishes early, it takes over half of the largest remaining part, so all connections finish together. A slow or stalled connection is replaced within about a second. [Engine spec](docs/ENGINE.md)
- **Crash-safe resume.** Progress is recorded only after the bytes are on disk. Kill the process or lose power, and `boltget resume` continues where it stopped. Our test harness has run 1,000 kill-and-resume cycles; every file came out byte-identical.
- **Link refresh.** Signed link expired at 97%? Give BoltGet a new one. It checks that the file is the same (size, ETag or sample bytes) and continues instead of starting over.
- **Bandwidth limits.** Per download and global, changed without reconnecting.
- **Checksum verification.** SHA-256, SHA-512, SHA-1 or MD5, plus digests the server sends. The file is hashed while it downloads, so there is no wait at the end.
- **Proxies and cookies.** HTTP, HTTPS and SOCKS5 proxies; browser `cookies.txt` files matched per request; HTTP Basic auth.
- **A scriptable CLI.** Documented exit codes and line-delimited JSON events for every state change.

```console
$ boltget get https://example.com/ubuntu-24.04.iso --checksum sha256:<hex>
$ boltget list
$ boltget resume 7
$ boltget get https://example.com/big.zip --limit 5M --json
```

### Today

The Rust engine and the `boltget` command-line tool: `get`, `list`, `pause`, `resume`, `refresh`, `remove` and `config`. It is pre-alpha and moving quickly. See [Status](#status).

### Coming

- **Desktop app.** Tauri 2 and Svelte 5. Dark-first, keyboard-first, small and fast.
- **Browser extension.** Chrome, Edge and Firefox (Manifest V3). Catches downloads and forwards cookies, referrer and headers through native messaging, with no local web server. [Details](docs/BROWSER-INTEGRATION.md)
- **Video and audio.** yt-dlp and ffmpeg, installed on demand and kept up to date, with a clean format picker. [Details](docs/VIDEO.md)
- **Queues and scheduling.** Categories, queues and time windows.

The full feature matrix, with milestones and comparisons, is in [docs/FEATURES.md](docs/FEATURES.md). What we are building toward and how we measure it is in [docs/VISION.md](docs/VISION.md).

## Status

**Pre-alpha.** Work on the first milestone, v0.1 (the Rust engine and CLI, with a published benchmark), is under way. There is no installer or desktop app yet. See [docs/ROADMAP.md](docs/ROADMAP.md).

To try the CLI from source: `cargo run --release -p boltget-cli -- get <url>`.

## Install (planned)

Nothing to install yet. Once releases exist:

- **Windows (v1):** signed installer from [GitHub Releases](https://github.com/boltget/boltget/releases) and [boltget.app/download](https://boltget.app/download). We also plan to publish to winget.
- **Browser extension:** Chrome Web Store, Microsoft Edge Add-ons, Firefox Add-ons.
- **CLI:** bundled with the desktop app, and as a standalone binary.
- **macOS and Linux:** after 1.0.

## Repository layout

The repository is a monorepo (pnpm workspaces + a Cargo workspace). Of `apps/` and `packages/`, only `apps/web` exists so far; the rest is the planned layout.

```
boltget/
├── crates/
│   ├── engine/          Rust download engine (library): probing, segmentation, I/O, persistence
│   ├── cli/             `boltget` command-line tool on top of the engine
│   └── testserver/      Local HTTP server that simulates misbehaving servers for tests
├── apps/
│   ├── desktop/         Tauri 2 desktop app (Svelte 5 + TypeScript + Vite)
│   │   └── src-tauri/   Rust backend that embeds the engine
│   ├── extension/       Browser extension (WXT, Manifest V3, Chrome/Edge/Firefox)
│   └── web/             boltget.app (Astro + Svelte islands) and docs (Starlight)
├── packages/
│   └── ui/              Shared Svelte components and design tokens (app + site)
├── brand/               Icon, animated icon, brand guidelines (brand/BRAND.md)
└── docs/                Design documents
```

## Documentation

| Document | What it covers |
|---|---|
| [Vision](docs/VISION.md) | Problem, principles, non-goals, how we measure success |
| [Features](docs/FEATURES.md) | Feature matrix and comparisons, with milestones |
| [Architecture](docs/ARCHITECTURE.md) | Components, process model, IPC, data model, security |
| [Engine](docs/ENGINE.md) | The download engine spec: probing, segmentation, resume, benchmarks |
| [Browser integration](docs/BROWSER-INTEGRATION.md) | Extension, capture rules, native messaging |
| [Video](docs/VIDEO.md) | yt-dlp and ffmpeg sidecars |
| [Tech stack](docs/TECH-STACK.md) | Every major choice, with rejected alternatives |
| [Design principles](docs/DESIGN-PRINCIPLES.md) | Product and UI rules for app and site |
| [Website](docs/WEBSITE.md) | boltget.app structure, hero, performance budget |
| [Roadmap](docs/ROADMAP.md) | Milestones and exit criteria |
| [Release](docs/RELEASE.md) | Versioning, channels, signing, updater |
| [Brand](brand/BRAND.md) | Visual identity, tokens and voice |

The complete index is [docs/README.md](docs/README.md).

## Contributing

The best time to shape BoltGet is now, while most of it is still being designed. Start with [CONTRIBUTING.md](CONTRIBUTING.md). It is the rulebook for everything: Conventional Commits, branching, PR rules, the RFC process, code style, the string catalog and our policy on AI-assisted contributions.

- Questions and ideas: [GitHub Discussions](https://github.com/boltget/boltget/discussions) (see [SUPPORT.md](SUPPORT.md))
- Bugs and concrete feature requests: [Issues](https://github.com/boltget/boltget/issues)
- Security problems: **do not open an issue**. See [SECURITY.md](SECURITY.md).
- How decisions are made: [GOVERNANCE.md](GOVERNANCE.md)
- Everyone is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

BoltGet is free and open source under the **GNU General Public License v3.0 or later** ([LICENSE](LICENSE)). Copyleft keeps it that way: anyone can use, study, modify and share it, and any distributed fork stays open too.

"BoltGet" and the BoltGet icon identify this project. Please do not use them to brand a modified build in a way that suggests it is the official one.
