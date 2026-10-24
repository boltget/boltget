# BoltGet documentation

Design documents for BoltGet. They define what we are building and how; code that deviates from them must update them in the same pull request (see [CONTRIBUTING.md](../CONTRIBUTING.md#documentation)). User-facing documentation will live on [boltget.app/docs](https://boltget.app/docs).

## Start here

| Document | Read it if you want to know... |
|---|---|
| [VISION.md](VISION.md) | Why BoltGet exists, its principles and non-goals, and how "beating IDM" is measured |
| [ROADMAP.md](ROADMAP.md) | What gets built in which milestone, and the exit criteria for each |
| [FEATURES.md](FEATURES.md) | Every planned feature, its milestone, and how it compares with IDM, FDM, Motrix and aria2 |

## Technical design

| Document | Covers |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Components, process model, IPC and events, data model, persistence, threading, error handling, security |
| [ENGINE.md](ENGINE.md) | The download engine spec: probing, dynamic segmentation, adaptive connections, HTTP/2, preallocation, write path, checkpointing, retries, link refresh, rate limiting, integrity, proxies, cookies, benchmarks |
| [BROWSER-INTEGRATION.md](BROWSER-INTEGRATION.md) | The WXT extension, capture rules, context menus, cookie/header forwarding, native messaging registration, media detection, store policies |
| [VIDEO.md](VIDEO.md) | yt-dlp and ffmpeg sidecars: install, verification, updates, the format picker, media flow |
| [TECH-STACK.md](TECH-STACK.md) | Every major technology decision as a short ADR, with rejected alternatives |

## Product and design

| Document | Covers |
|---|---|
| [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md) | Restraint, density, motion, themes, accessibility, keyboard, copy and strings |
| [WEBSITE.md](WEBSITE.md) | boltget.app structure, hero concept, performance budget, OG images, hosting |
| [../brand/BRAND.md](../brand/BRAND.md) | Visual identity: icon, colors, typography, tokens |

## Process

| Document | Covers |
|---|---|
| [RELEASE.md](RELEASE.md) | Versioning, channels, build pipeline, code signing, installer, auto-update, reproducibility, release checklist |
| [rfcs/](rfcs/) | Accepted and rejected RFCs; start a new one from [rfcs/0000-template.md](rfcs/0000-template.md) |

## Project policies (repository root)

| Document | Covers |
|---|---|
| [README.md](../README.md) | Project overview |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | The contribution rulebook: commits, branching, PRs, RFCs, code style, tests, labels, AI policy |
| [GOVERNANCE.md](../GOVERNANCE.md) | Roles, becoming a maintainer, decision making |
| [SUPPORT.md](../SUPPORT.md) | Where to ask questions and report problems |
| [SECURITY.md](../SECURITY.md) | How to report vulnerabilities privately |
| [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) | Contributor Covenant 2.1 |
| [CHANGELOG.md](../CHANGELOG.md) | Release notes (Keep a Changelog) |
| [LICENSE](../LICENSE) | GPL-3.0-or-later |

## Conventions in these documents

- **Design target** marks a number chosen as a starting point, not a measurement. Measured values replace targets as data arrives.
- `MUST` / `SHOULD` / `MAY` in the engine spec follow RFC 2119.
- Planned paths (`crates/engine`, `apps/desktop`, ...) refer to the [monorepo layout](../README.md#repository-layout); most do not exist yet.
- Diagrams use Mermaid, which GitHub renders inline.
