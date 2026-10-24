# Vision

BoltGet exists to be the download manager we wish existed: as fast and reliable as Internet Download Manager, free and open source, honest about what it does, and pleasant to use.

This document explains the problem, the principles that guide every decision, what we deliberately will not do, who we build for, and how we will know we have succeeded.

## The problem

### IDM is the benchmark, and it shows its age

Internet Download Manager (IDM) earned its reputation. Its multi-connection engine with dynamic segmentation, its browser integration and its resume behaviour are the reason millions of people install a download manager at all. But:

- **It is paid.** After the trial, it requires a license. That is legitimate, but it is also why the internet is full of cracked copies.
- **It is Windows-only.** There is no macOS or Linux version.
- **The UI is dated.** It is dense in the wrong way: a classic Win32 list view, toolbar icons, and dialogs layered on dialogs. It does not scale well on high-DPI displays and has no real dark mode.
- **It nags.** Trial reminders and, notoriously, the "registered with a fake serial number" warning that appears even for some users who believe they bought a legitimate key from a reseller.
- **It is closed.** Nobody outside the vendor can audit what a program with deep browser integration and network access actually does.

### Free alternatives make you choose a compromise

- Some are **closed freeware** with a history of bundled offers.
- Some are **open source but thin GUIs over a general engine** (most commonly aria2) that is excellent but not tuned for interactive desktop use, browser handoff, or IDM-style segment rebalancing.
- Some are **abandoned** or update sporadically.
- Many are **Electron apps** that idle at hundreds of megabytes of RAM to show a list.
- The app stores are full of **repackages**: someone else's open-source code with ads added and the license ignored.

Nobody ships all of it at once: IDM-class speed, crash-proof resume, browser integration that respects store policy, a modern UI, and a license that keeps it open forever.

## Principles

1. **Speed is the product.** If BoltGet is not at least as fast as IDM on the same file, same network, same connection count, nothing else matters. Every engine change is benchmarked. See [ENGINE.md](ENGINE.md).
2. **Never lose a byte.** A download that was 90% done stays 90% done through crashes, power loss, reboots, sleep, network changes and expired links. Correctness of resume beats raw speed when the two conflict.
3. **Respect the user.** No ads, no bundled offers, no telemetry without opt-in, no dark patterns, no nag screens. Ever. This is a hard rule, not a business decision we might revisit.
4. **Simple, but makes you say wow.** Restraint first. Show what matters, hide what does not, and use motion only when it explains something (for example: watching a segment split in two). See [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md).
5. **Small and quiet.** Fast start, low idle memory, no background services beyond what is needed, a small installer. A download manager should feel like part of the OS, not a second browser.
6. **Open by default.** GPL-3.0-or-later, designed in public, decisions written down (this folder), benchmarks published with raw data and methodology so anyone can reproduce or refute them.
7. **Play by the rules.** The browser extension complies with every store's policies. Video downloading happens in the app through yt-dlp, not in the extension.
8. **Secure by construction.** No localhost HTTP server for the extension, no remote code, signed binaries, signed updates, verified sidecars. See [ARCHITECTURE.md](ARCHITECTURE.md#security).

## Non-goals

These are things we have consciously decided not to do, at least for v1. Some may come later; none will be added at the cost of the principles above.

- **Not a torrent client in v1.** BitTorrent and magnet support may come after 1.0 (see [ROADMAP.md](ROADMAP.md)). Excellent dedicated clients exist.
- **Not a "download anything from any site" scraper in the extension.** The extension detects and hands off. It never contains site-specific extraction logic.
- **Not a DRM circumvention tool.** We will not download DRM-protected streams.
- **Not a site grabber / offline website copier in v1.** IDM has one; it is a niche feature with high complexity.
- **Not a cloud service.** BoltGet does not route downloads through our servers. There is no account. (Optional settings sync, if it ever happens, would be end-to-end encrypted and opt-in.)
- **Not a platform for monetization.** No "Pro" tier that gates the engine. If the project takes donations, they fund signing certificates and infrastructure, not features.
- **Not an accelerator for already-saturated links.** Multi-connection downloads help when a server or path limits per-connection throughput. If your single connection already saturates your line, BoltGet cannot make your ISP faster, and we will say so plainly.

## Target users

1. **The IDM user who wants out.** Uses IDM daily, values its speed, dislikes paying or the nags, or has moved to a Mac. Needs: equal or better speed, browser catching that "just works", familiar concepts (categories, queues, scheduler).
2. **The heavy downloader on a mediocre connection.** Large ISOs, game assets, datasets, course videos, on a connection that drops or throttles. Needs: resume that never fails, link refresh, speed limits so the rest of the household can browse.
3. **The power user and developer.** Wants a CLI, scriptability, checksums, proxies, predictable behaviour, and the ability to read the source.

## What "beating IDM" means, measurably

"Faster than IDM" must be a claim we can defend with data. The detailed methodology is in [ENGINE.md, Benchmarks](ENGINE.md#benchmarks-plan-vs-idm). The criteria:

| Dimension | Measure | v1.0 target |
|---|---|---|
| Throughput | Median wall-clock time to download each file in the benchmark suite, same machine, same network, same connection count, alternating runs | BoltGet median is equal to or lower than IDM's on every scenario, and lower on the per-connection-throttled scenarios. "Equal" means within the measured run-to-run noise. |
| Throughput under adverse conditions | Same as above with injected latency, packet loss and per-connection throttling | Equal or better than IDM in every scenario |
| Resume reliability | Fraction of forced-kill / power-cut resume tests that complete with a byte-identical file | 100% across the automated suite (any failure is a release blocker) |
| Link refresh | Downloads with an expired signed URL resumed with a new URL without re-downloading completed bytes | Works for the test matrix in [ENGINE.md](ENGINE.md#link-refresh) |
| Browser capture | Fraction of downloads in the browser test matrix correctly captured with cookies and referrer intact | Design target: at least on par with IDM on the same matrix |
| Resource use | Idle RAM, RAM during a 10-connection download, CPU% at a given throughput | Design target: lower than IDM on the same machine. Exact numbers will be set after the first measurements of both, not invented now. |
| Startup | Cold start to interactive window | Design target: under 1 second on a mid-range machine with an SSD |
| Installer size | Size of the signed installer, excluding on-demand sidecars | Design target: under 15 MB |
| Price and honesty | Cost, ads, nags | Free, none, none |

Two rules apply to every number above:

- **Design targets are labelled as such.** Where we do not yet have data, the target is a direction, not a promise. Once v0.1 benchmarks exist, targets get replaced with measured baselines.
- **Results are published with raw data and the scripts to reproduce them**, including the scenarios where BoltGet loses. A benchmark that only shows wins is marketing, not engineering.

## What success looks like at 1.0

- A Windows user can install BoltGet in under a minute, install the extension, and never think about IDM again.
- The published benchmark shows BoltGet matching or beating IDM, and independent users reproduce it.
- A contributor can read [ARCHITECTURE.md](ARCHITECTURE.md) and [ENGINE.md](ENGINE.md), and make a meaningful engine change within a day.
- Nobody has ever lost a download to a BoltGet crash.
