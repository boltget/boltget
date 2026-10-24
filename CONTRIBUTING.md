# Contributing to BoltGet

Thanks for your interest in BoltGet. This document is the rulebook for contributing: how we talk, how we decide, how code gets in, and what "done" means. It is long because it is meant to be authoritative; the [quick version](#quick-version) fits on one screen.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). How decisions are made and who maintains what is described in [GOVERNANCE.md](GOVERNANCE.md). Where to ask questions is in [SUPPORT.md](SUPPORT.md).

> **Project status:** pre-alpha, design phase. There is no code yet; the most valuable contributions right now are reviews of the design documents in [docs/](docs/README.md), well-argued RFCs, and benchmarks/data that sharpen the [engine spec](docs/ENGINE.md). Sections below marked *once the code lands* describe the rules that apply as soon as the corresponding code exists.

## Contents

- [Quick version](#quick-version)
- [Ways to contribute](#ways-to-contribute)
- [Discuss before big changes](#discuss-before-big-changes)
- [RFCs](#rfcs)
- [Development setup](#development-setup) *(once the code lands)*
- [Branching and merging](#branching-and-merging)
- [Commit messages: Conventional Commits](#commit-messages-conventional-commits)
- [Developer Certificate of Origin (DCO)](#developer-certificate-of-origin-dco)
- [Pull requests](#pull-requests)
- [Code style](#code-style)
- [Tests](#tests)
- [User-facing strings](#user-facing-strings)
- [Documentation](#documentation)
- [Dependencies and licensing](#dependencies-and-licensing)
- [Changelog](#changelog)
- [Versioning](#versioning)
- [Issues and triage](#issues-and-triage)
- [Labels](#labels)
- [AI-assisted contributions](#ai-assisted-contributions)
- [Security issues](#security-issues)

## Quick version

1. Non-trivial change? Open or find an **issue** first. Architectural change? Write an **[RFC](#rfcs)**.
2. Branch from `main`: `feat/short-name` or `fix/short-name`.
3. Commit with **[Conventional Commits](#commit-messages-conventional-commits)** and **sign off** every commit: `git commit -s`.
4. Keep the PR to **one logical change**, with tests and docs updated, and a **Conventional Commit PR title** (it becomes the squash commit).
5. CI green + one maintainer approval → **squash-merge**.

## Ways to contribute

- **Review the design.** Read [docs/](docs/README.md). If something is wrong, unclear, or could be simpler, open a Discussion or a PR against the document.
- **Report bugs** (once there is something to break) using the bug report form.
- **Propose features** using the feature request form. Check [docs/FEATURES.md](docs/FEATURES.md) and [docs/ROADMAP.md](docs/ROADMAP.md) first, and the [non-goals](docs/VISION.md#non-goals).
- **Fix things.** Issues labelled [`good first issue`](https://github.com/boltget/boltget/labels/good%20first%20issue) and [`help wanted`](https://github.com/boltget/boltget/labels/help%20wanted) are the best entry points.
- **Benchmark.** Run BoltGet (and IDM, if you have it) on your network with the published harness and share results. Real-world data is gold.
- **Correct comparisons.** If [FEATURES.md](docs/FEATURES.md) says something wrong about another product, a PR with a source is very welcome.
- **Translate?** Not yet. BoltGet ships in English only; see [User-facing strings](#user-facing-strings).

## Discuss before big changes

To avoid wasted work on both sides:

| Change | What to do first |
|---|---|
| Typo, broken link, small doc fix, obvious bug fix | Just open a PR |
| Bug fix that changes behaviour, small feature within the roadmap | Open (or comment on) an issue; wait for a maintainer to confirm the approach |
| New feature not on the roadmap, UX change, new setting | Open a [Discussion](https://github.com/boltget/boltget/discussions) in *Ideas*; it becomes an issue once there is agreement |
| Architecture, protocols, data model, new dependency with large footprint, anything that contradicts a doc in `docs/` | Write an [RFC](#rfcs) |

PRs for significant changes without prior discussion may be closed with a pointer here. That is not a judgment of the work; it is how we keep the project coherent.

## RFCs

An RFC ("request for comments") is a short design document for changes that are hard to reverse. It is deliberately lightweight ("RFC-lite").

**When:** changes to architecture ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)), the engine's behaviour or invariants ([docs/ENGINE.md](docs/ENGINE.md)), the IPC/native messaging protocol, the database schema beyond additive changes, security model, tech stack decisions ([docs/TECH-STACK.md](docs/TECH-STACK.md)), licensing, or governance.

**How:**

1. Copy [docs/rfcs/0000-template.md](docs/rfcs/0000-template.md) to `docs/rfcs/0000-short-title.md`.
2. Fill it in. Be concrete; include alternatives you rejected and why.
3. Open a PR titled `docs(rfc): <title>`. The PR is where discussion happens.
4. A maintainer assigns the number (next free four-digit number) and renames the file before merging.
5. **Outcomes:** *Accepted* (merged with status Accepted; the affected docs in `docs/` are updated in the same PR or a follow-up), *Rejected* (merged with status Rejected and the reasons, so the idea and its counter-arguments are recorded), or *Withdrawn* (closed by the author).
6. Final decision follows [GOVERNANCE.md](GOVERNANCE.md#decision-making). Expect at least one week of comment time for anything non-trivial.

## Development setup

*Once the code lands.* The commands below are the intended interface; the exact scripts will be defined in v0.1 (engine/CLI) and v0.2 (app) and this section updated.

### Prerequisites (Windows)

- **Rust** stable via [rustup](https://rustup.rs) (the exact version is pinned by `rust-toolchain.toml`; rustup picks it up automatically), with the MSVC toolchain.
- **Visual Studio Build Tools** with the "Desktop development with C++" workload (needed by the MSVC Rust toolchain).
- **Node.js** LTS (pinned by `.node-version`) and **pnpm** via Corepack: `corepack enable`.
- **WebView2 Runtime** (preinstalled on Windows 11 and current Windows 10).

### Common commands

```sh
# Rust: engine and CLI
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

# Run the CLI from source
cargo run -p boltget-cli -- get https://example.com/file.iso

# JavaScript workspace
pnpm install
pnpm -r lint          # ESLint + Prettier check
pnpm -r check         # svelte-check / tsc
pnpm -r test          # Vitest

# Desktop app (Tauri dev mode with hot reload)
pnpm --filter @boltget/desktop tauri dev

# Browser extension (WXT dev mode; opens a browser with the extension loaded)
pnpm --filter @boltget/extension dev

# Website and docs
pnpm --filter @boltget/web dev
```

Never point development builds at your real BoltGet data: development builds use a separate data directory and a separate native messaging host name automatically.

## Branching and merging

- **Trunk-based development on `main`.** `main` is always releasable: CI green, no half-finished features exposed to users (use a setting or build flag to hide incomplete work).
- **Short-lived branches**, named by type:
  - `feat/<short-name>`, `fix/<short-name>`, `docs/<short-name>`, `refactor/<short-name>`, `perf/<short-name>`, `test/<short-name>`, `build/<short-name>`, `ci/<short-name>`, `chore/<short-name>`
  - Example: `feat/engine-link-refresh`, `fix/cli-json-sizes`.
  - Aim to merge within days, not weeks. Split large work into a series of PRs that each leave `main` working.
- **Squash-merge only.** Every PR becomes one commit on `main`. The repository is configured to allow only squash merges and to require a linear history.
- **The PR title becomes the commit subject**, so it must be a valid Conventional Commit (checked by CI).
- No force-pushes to `main`, ever. Force-pushing your own PR branch is fine.
- Release tags (`vX.Y.Z`) are created by maintainers on `main` (see [docs/RELEASE.md](docs/RELEASE.md)).

## Commit messages: Conventional Commits

We use [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).

```
<type>(<scope>): <subject>

<body: what and why, wrapped at 72 columns>

<footers>
Signed-off-by: Your Name <you@example.com>
```

**Types:**

| Type | Use for |
|---|---|
| `feat` | A new user-visible capability |
| `fix` | A bug fix |
| `docs` | Documentation only |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `perf` | Performance improvement (include numbers in the body) |
| `test` | Adding or fixing tests only |
| `build` | Build system, packaging, installer |
| `ci` | CI configuration and workflows |
| `chore` | Maintenance that fits nothing above (tooling config, repo housekeeping) |

**Scopes** (optional but encouraged; use the area the change is in):

`engine`, `cli`, `desktop`, `extension`, `web`, `ui`, `brand`, `docs`, `deps`, plus `rfc` for RFC pull requests (`docs(rfc): ...`)

**Rules:**

- Subject in the **imperative mood** ("add", not "added" or "adds"), lowercase first word after the colon, no trailing period, **at most 72 characters** for the whole first line.
- Body explains *why*; the diff shows *what*. Reference issues in footers: `Fixes #123`, `Refs #456`.
- Breaking changes: `!` after the type/scope (`feat(cli)!: rename --out to --output`) **and** a `BREAKING CHANGE:` footer describing the migration.
- Examples:
  - `feat(engine): split largest remaining segment when a worker frees up`
  - `fix(extension): resume browser download when native host times out`
  - `perf(engine): coalesce writes to 1 MiB chunks`
  - `docs(rfc): propose HTTP/3 workers`
  - `build(deps): bump hyper to 1.5`

Individual commits inside a PR should follow the format too (it makes review easier), but only the PR title is enforced, because it becomes the squash commit.

## Developer Certificate of Origin (DCO)

BoltGet uses the **[Developer Certificate of Origin 1.1](https://developercertificate.org/)** instead of a Contributor License Agreement (CLA).

**Why DCO, not a CLA:** BoltGet is GPL-3.0-or-later and intends to stay that way. A CLA typically grants the project owner extra rights (for example to relicense contributions), which a copyleft project does not need and which many contributors are rightly wary of. The DCO is a lightweight statement that you have the right to submit your contribution under the project's license. There is nothing to sign up for; you add one line per commit.

**How:** add a `Signed-off-by` trailer to every commit, matching the commit author:

```sh
git commit -s -m "fix(engine): handle 416 on empty files"
```

which produces:

```
Signed-off-by: Your Name <you@example.com>
```

- Use your real name (or the name you are publicly known by) and an email you control.
- Forgot? Fix the last commit with `git commit --amend -s --no-edit`, or all commits on your branch with `git rebase --signoff main`, then force-push your branch.
- The [DCO check](.github/workflows/dco.yml) runs on every PR and verifies that every commit is signed off by its author. Bot commits (Dependabot) are exempt.

By signing off, you certify the DCO 1.1 statements, in short: the contribution is yours (or you have the right to submit it), and you submit it under the project's open-source license.

## Pull requests

### Rules

- **One logical change per PR.** A bug fix and an unrelated refactor are two PRs. Reviewers can then approve, revert or bisect cleanly.
- **Linked issue** for anything non-trivial (`Fixes #123` in the description).
- **Tests** added or updated for every behaviour change (see [Tests](#tests)).
- **Docs** updated in the same PR when behaviour, settings, CLI, protocols or architecture change. If a PR contradicts a document in `docs/`, update the document (or the PR is not done).
- **Changelog** entry for user-visible changes (see [Changelog](#changelog)).
- **CI green.** Formatting, lint, type checks, tests, PR title, DCO and link checks all pass.
- **At least one maintainer approval.** Changes to `crates/engine` that touch segmentation, the write path, checkpointing or security-relevant code (native messaging, IPC, sidecars, updater) need approval from a maintainer who owns that area (see [CODEOWNERS](.github/CODEOWNERS) and [GOVERNANCE.md](GOVERNANCE.md)).
- **UI changes include screenshots** (dark and light theme) or a short recording.
- **Performance claims include numbers**: before/after, how measured, on what hardware and network.
- Fill in the [PR template](.github/PULL_REQUEST_TEMPLATE.md) checklist.

### Review

- Reviewers aim to respond within a few days. If a PR has had no response for a week, a polite ping is welcome.
- Review comments are about the code, never the person. Prefixing optional suggestions with `nit:` is encouraged.
- The author resolves conversations after addressing them; the reviewer may re-open.
- Draft PRs are welcome for early feedback; mark ready when it meets the rules above.
- Maintainers may push small fixes (typos, formatting) to your branch to get it merged faster, unless you opt out in the PR description.

## Code style

Formatting is never discussed in review: the tools decide.

### Rust (`crates/*`, `apps/desktop/src-tauri`)

- **Edition 2024**, stable toolchain pinned by `rust-toolchain.toml`.
- `cargo fmt` (rustfmt, default configuration unless `rustfmt.toml` says otherwise).
- `cargo clippy --all-targets -- -D warnings`: warnings are errors in CI. If a lint is wrong for a specific case, `#[allow(clippy::...)]` with a one-line comment explaining why.
- **Errors:** `thiserror` for typed errors in libraries; `anyhow` only in binaries' `main`/setup code. No `unwrap()`/`expect()` on data from the network, disk or IPC; `expect("reason")` is acceptable for true invariants, with the reason stating the invariant.
- **`unsafe`:** `#![forbid(unsafe_code)]` in the engine except in `platform/` modules (Win32 calls). Every `unsafe` block has a `// SAFETY:` comment.
- **Async:** no blocking calls on Tokio worker threads (disk I/O goes through the writer thread or `spawn_blocking`); no locks held across `.await`.
- **Logging:** `tracing` with structured fields; never log secrets (cookies, auth headers, full signed URLs). Use the redaction helpers.
- Public items in the engine have doc comments. Explain *why* in comments, not *what*.

### TypeScript and Svelte (`apps/desktop`, `apps/extension`, `apps/web`, `packages/ui`)

- **TypeScript strict mode** (`"strict": true`), no `any` without a comment explaining why.
- **Prettier** for formatting (including `prettier-plugin-svelte` and `prettier-plugin-astro`).
- **ESLint** (flat config) with `typescript-eslint` and `eslint-plugin-svelte`; zero warnings in CI.
- **`svelte-check`** passes with zero errors and warnings.
- **Svelte 5 runes** (`$state`, `$derived`, `$effect`, `$props`); no legacy `export let` / stores in new code unless there is a reason.
- Styles use design tokens from `packages/ui` (CSS custom properties). No hard-coded colors, font sizes or spacing values.
- Message types shared with Rust are **generated** from the Rust definitions; never hand-edit generated files.

### Everything else

- [`.editorconfig`](.editorconfig): UTF-8, LF line endings, final newline, 2-space indentation (4 for Rust).
- Markdown: one sentence per line is *not* required; wrap naturally. Use relative links between docs. Links are checked in CI.

## Tests

- **Every bug fix comes with a regression test** that fails without the fix, unless that is genuinely impractical (say why in the PR).
- **Engine:** unit tests for pure logic; integration tests against the local test server for network behaviour; property tests for segmentation and checkpoint invariants. Changes to segmentation, the write path or checkpointing must keep the crash-resume suite green. See [docs/ENGINE.md, Testing strategy](docs/ENGINE.md#testing-strategy).
- **Performance changes** to the engine include `boltget bench` results before and after.
- **UI:** Vitest for logic and components; Playwright for critical flows (add download, pause/resume, settings) once the app exists.
- **Extension:** unit tests for capture logic; Playwright end-to-end tests with a stub native host.
- **No network access to the public internet in tests.** Use the local test server or recorded fixtures. Real-world tests live in the benchmark harness, not in CI.
- Flaky tests are bugs. Quarantine (with an issue) rather than retry-until-green.

## User-facing strings

BoltGet is **English only** (app, extension and website). To keep wording consistent and leave the door open for community translations after 1.0:

- **All user-facing strings live in the central string catalog** and are referenced by key. No string literals for UI text in components, no user-facing messages built in Rust (the engine returns message keys and parameters; see [docs/ARCHITECTURE.md, Error handling](docs/ARCHITECTURE.md#error-handling)).
- Whole messages with named placeholders (`"{count} downloads paused"`), never sentences assembled by concatenation. Use plural-aware messages for counts.
- No text inside images or icons.
- Follow the tone and style rules in [docs/DESIGN-PRINCIPLES.md, Copy and strings](docs/DESIGN-PRINCIPLES.md#copy-and-strings): sentence case, verbs on buttons, calm and direct.

Localization is a possible post-1.0, community-driven effort, not a current goal. Please do not open PRs adding translations or i18n libraries until there is an accepted RFC for it.

## Documentation

- Design docs live in [docs/](docs/README.md) and are the source of truth for how BoltGet works. Code that deviates from a doc must update the doc in the same PR.
- User docs live in the website's Starlight docs (`apps/web`) once it exists.
- Write in plain, direct English. Prefer concrete examples and tables over prose. Mark design targets as such; do not present unmeasured numbers as facts.
- Repository Markdown is English only.

## Dependencies and licensing

- **License of contributions:** GPL-3.0-or-later, the license of the repository (see [LICENSE](LICENSE)), certified through the [DCO](#developer-certificate-of-origin-dco).
- **New dependencies** must be GPL-3.0-compatible (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, MPL-2.0, Unlicense, and similar). `cargo-deny` and a JS license check enforce this in CI once the code lands.
- Justify new dependencies in the PR: what it does, why not write it, maintenance status, size. Prefer well-maintained, widely used crates and packages. Large or security-sensitive dependencies need an RFC.
- Do not vendor or copy code from other projects without preserving its license notice and confirming compatibility in the PR.
- Dependency updates come from Dependabot (`build(deps): ...`) and are reviewed like any other PR.

## Changelog

- [CHANGELOG.md](CHANGELOG.md) follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
- **Every PR with a user-visible change adds a line** under `## [Unreleased]` in the matching section: `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.
- Write for users, not developers: "Downloads now resume after the link expires if you provide a new link", not "refactor link refresh state machine".
- Internal changes (refactors, CI, tests) do not need a changelog entry.
- Maintainers move entries into a version section at release time ([docs/RELEASE.md](docs/RELEASE.md#changelog)).

## Versioning

BoltGet follows [Semantic Versioning](https://semver.org). Before 1.0, minor versions may contain breaking changes. What counts as public API, how channels work and how the extension's version maps to the product version is defined in [docs/RELEASE.md](docs/RELEASE.md#versioning). Contributors do not bump versions in PRs; maintainers do at release time.

## Issues and triage

- **Use the issue forms** (bug report, feature request). Blank issues are disabled; questions go to [Discussions](https://github.com/boltget/boltget/discussions) (see [SUPPORT.md](SUPPORT.md)).
- **Search first.** Duplicates are closed with a link to the original; add your details there.
- **Triage** (done by maintainers and triagers, aiming for within a week):
  1. New issues get `status:needs-triage` automatically.
  2. A triager checks it is reproducible / well-defined, adds `type:*`, `area:*` and, when known, `priority:*`.
  3. Status moves to `status:needs-info` (waiting on the reporter), `status:confirmed` (ready to work on), or the issue is closed (duplicate, not planned, out of scope per [non-goals](docs/VISION.md#non-goals)).
  4. Issues in `status:needs-info` with no reply for 30 days may be closed; they can be reopened anytime with the requested information.
- **Claiming work:** comment "I'd like to work on this" and wait for a maintainer to assign you, so two people do not duplicate effort. If an assigned issue has no activity for 30 days, it may be unassigned.
- **Priorities:** `priority:critical` (data loss, security, crash on common path; release blocker), `priority:high` (major feature broken, no workaround), `priority:medium` (default), `priority:low` (nice to have).

## Labels

The full taxonomy, with colors and descriptions, is defined in [.github/labels.yml](.github/labels.yml) and synced to GitHub by a label-sync action. Summary:

| Group | Labels |
|---|---|
| `type:*` | `type:bug`, `type:feature`, `type:enhancement`, `type:docs`, `type:performance`, `type:security`, `type:refactor`, `type:question`, `type:rfc` |
| `area:*` | `area:engine`, `area:cli`, `area:desktop`, `area:extension`, `area:video`, `area:web`, `area:ui`, `area:brand`, `area:docs`, `area:build`, `area:ci`, `area:deps` |
| `priority:*` | `priority:critical`, `priority:high`, `priority:medium`, `priority:low` |
| `status:*` | `status:needs-triage`, `status:needs-info`, `status:confirmed`, `status:in-progress`, `status:blocked`, `status:wontfix`, `status:duplicate` |
| Community | `good first issue`, `help wanted` |
| Platform | `platform:windows`, `platform:macos`, `platform:linux` |

`good first issue` means: well-scoped, with pointers to the relevant code and docs, and a maintainer willing to mentor. `help wanted` means: we want this and would welcome someone taking it on.

## AI-assisted contributions

Using AI tools (code assistants, chat models, agents) to help write code, tests or docs is **allowed**. The rules are the same as for any other contribution, with emphasis:

- **You are fully responsible** for everything you submit, exactly as if you had typed every character. "The AI wrote it" is never an explanation for a bug, a license problem or a security issue.
- **You must understand the code** you submit and be able to explain and defend it in review. If a reviewer asks why something works, "I'm not sure" means the PR is not ready.
- **You must have tested it**: run it, run the tests, and check the edge cases yourself.
- **Licensing:** you must have the right to submit the output under GPL-3.0-or-later, and you certify that through your DCO sign-off. Do not submit output that reproduces code from other projects in a license-incompatible way.
- **Keep it focused.** AI tools make it easy to produce large, sprawling diffs. The [one logical change](#rules) rule applies; very large generated PRs will be asked to split.
- **No unattended agents opening PRs or issues** against this repository, and no AI-generated review comments posted as if they were your own analysis. Automated tools configured by maintainers (for example Dependabot) are the exception.
- Disclosure is appreciated but not required for small assists; for substantially generated contributions, a short note in the PR description helps reviewers calibrate.

## Security issues

**Do not report security vulnerabilities in public issues, discussions or pull requests.** Report them privately as described in [SECURITY.md](SECURITY.md).

---

Thank you for helping build BoltGet.
