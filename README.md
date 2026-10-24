<p align="center">
  <img src="brand/icon.svg" width="120" height="120" alt="BoltGet icon: a pixel down-arrow in a mint-to-emerald gradient on near-black">
</p>

<h1 align="center">BoltGet</h1>

<p align="center">
  <strong>The free, open-source download manager that is faster than the one you paid for.</strong>
</p>

<p align="center">
  <a href="https://boltget.app">boltget.app</a> ·
  <a href="docs/README.md">Docs</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a>
</p>

<p align="center">
  <img alt="License: GPL-3.0-or-later" src="https://img.shields.io/badge/license-GPL--3.0--or--later-059669">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-34D399">
  <img alt="Platform: Windows first" src="https://img.shields.io/badge/platform-Windows%20first-A7F3D0">
  <!-- Enable once the workflows exist:
  <a href="https://github.com/boltget/boltget/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/boltget/boltget/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/boltget/boltget/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/boltget/boltget"></a>
  -->
</p>

---

## What is BoltGet?

BoltGet is a download manager for people who download big files and care how long it takes. It splits every download across multiple connections, keeps re-splitting the work so no connection sits idle, survives crashes and expired links, and does all of this behind an interface that stays out of your way.

It is built to match and then beat **Internet Download Manager (IDM)** on the things that matter (raw speed, resume reliability, browser integration), while being free, open source (GPL-3.0-or-later), free of ads and nag screens, and eventually cross-platform.

## Why

- **IDM is excellent at its core job, but** it is paid, Windows-only, has a dated UI, and nags users with license and fake-serial warnings.
- **Free alternatives** tend to be bloated, abandoned, built on a general-purpose engine that is not tuned for desktop use, or repackaged with adware.
- **Nobody has shipped all of it in one place:** IDM-class speed, a modern UI, honest open-source licensing, and a browser extension that complies with store policies.

The full argument, the principles we hold, and how we will *measure* "beating IDM" are in [docs/VISION.md](docs/VISION.md).

## Feature highlights (planned)

| | |
|---|---|
| **Dynamic segmentation** | Starts N connections; whenever one finishes, it splits the largest remaining segment and opens a new connection. Connection count adapts to measured throughput. [Engine spec](docs/ENGINE.md) |
| **Crash-safe resume** | Segment progress is checkpointed to SQLite only after the bytes are durable on disk. Pull the power cord and BoltGet resumes where it left off. |
| **Link refresh** | Signed URL expired at 97%? Give BoltGet a fresh link. It checks size, ETag and sample bytes, then resumes instead of starting over. |
| **Browser integration** | Chrome, Edge and Firefox extension (Manifest V3) catches downloads and forwards cookies, referrer and headers through native messaging. No localhost web server. [Details](docs/BROWSER-INTEGRATION.md) |
| **Video and audio** | yt-dlp and ffmpeg, installed on demand and kept up to date, with a clean format and quality picker. [Details](docs/VIDEO.md) |
| **Queues and scheduler** | Categories, queues, per-download and global speed limits, time windows. |
| **Integrity** | SHA-256 (and other) checksum verification, plus server-provided digests when available. |
| **CLI** | `boltget` runs on the same engine, for scripts and servers. |
| **Modern UI** | Tauri 2 + Svelte 5. Dark-first, keyboard-first, small and fast. |

The full matrix, with a comparison to IDM, Free Download Manager, Motrix and aria2 and the target milestone for each feature, is in [docs/FEATURES.md](docs/FEATURES.md).

## Status

**Pre-alpha, under active design.** There is no code yet. This repository currently holds the design documents that define what we are building. The first milestone (v0.1) is the Rust engine plus CLI, with a published benchmark against IDM. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Install (planned)

Nothing to install yet. Once releases exist:

- **Windows (v1):** signed installer from [GitHub Releases](https://github.com/boltget/boltget/releases) and [boltget.app/download](https://boltget.app/download). We also plan to publish to winget.
- **Browser extension:** Chrome Web Store, Microsoft Edge Add-ons, Firefox Add-ons.
- **CLI:** bundled with the desktop app, and as a standalone binary.
- **macOS and Linux:** after 1.0.

## Repository layout

The repository is a monorepo (pnpm workspaces + a Cargo workspace). Most of these directories do not exist yet; this is the planned layout.

```
boltget/
├── crates/
│   ├── engine/          Rust download engine (library): probing, segmentation, I/O, persistence
│   └── cli/             `boltget` command-line tool on top of the engine
├── apps/
│   ├── desktop/         Tauri 2 desktop app (Svelte 5 + TypeScript + Vite)
│   │   └── src-tauri/   Rust backend that embeds the engine
│   ├── extension/       Browser extension (WXT, Manifest V3, Chrome/Edge/Firefox)
│   └── web/             boltget.app (Astro + Svelte islands) and docs (Starlight)
├── packages/
│   └── ui/              Shared Svelte components and design tokens (app + site)
├── brand/               Icon, animated icon, brand guidelines (brand/BRAND.md)
└── docs/                Design documents (you are here)
```

## Documentation

| Document | What it covers |
|---|---|
| [Vision](docs/VISION.md) | Problem, principles, non-goals, what "beating IDM" means |
| [Features](docs/FEATURES.md) | Feature matrix vs IDM and others, with milestones |
| [Architecture](docs/ARCHITECTURE.md) | Components, process model, IPC, data model, security |
| [Engine](docs/ENGINE.md) | The download engine spec: probing, segmentation, resume, benchmarks |
| [Browser integration](docs/BROWSER-INTEGRATION.md) | Extension, capture rules, native messaging |
| [Video](docs/VIDEO.md) | yt-dlp and ffmpeg sidecars |
| [Tech stack](docs/TECH-STACK.md) | Every major choice, with rejected alternatives |
| [Design principles](docs/DESIGN-PRINCIPLES.md) | Product and UI rules for app and site |
| [Website](docs/WEBSITE.md) | boltget.app structure, hero, performance budget |
| [Roadmap](docs/ROADMAP.md) | Milestones and exit criteria |
| [Release](docs/RELEASE.md) | Versioning, channels, signing, updater |
| [Brand](brand/BRAND.md) | Visual identity and tokens |

The complete index is [docs/README.md](docs/README.md).

## Contributing

BoltGet is designed in the open, and the best time to shape it is now, while the design is still on paper. Start with [CONTRIBUTING.md](CONTRIBUTING.md). It is the rulebook for everything: Conventional Commits, DCO sign-off, branching, PR rules, the RFC process, code style, the string catalog and our policy on AI-assisted contributions.

- Questions and ideas: [GitHub Discussions](https://github.com/boltget/boltget/discussions) (see [SUPPORT.md](SUPPORT.md))
- Bugs and concrete feature requests: [Issues](https://github.com/boltget/boltget/issues)
- Security problems: **do not open an issue**. See [SECURITY.md](SECURITY.md).
- How decisions are made: [GOVERNANCE.md](GOVERNANCE.md)
- Everyone is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

BoltGet is licensed under the **GNU General Public License v3.0 or later** ([LICENSE](LICENSE)).

Why copyleft: the download-manager space is full of closed, ad-stuffed repackages of other people's work. The GPL means anyone can use, study, modify and redistribute BoltGet, and any distributed fork has to stay open too.

"BoltGet" and the BoltGet icon identify this project. Please do not use them to brand a modified build in a way that suggests it is the official one.
