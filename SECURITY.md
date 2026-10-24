# Security policy

BoltGet parses untrusted network data, writes files to disk, talks to a browser extension, runs helper binaries and updates itself. We take security reports seriously and appreciate responsible disclosure.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions or pull requests.**

Report privately through **GitHub Security Advisories**:

1. Go to [github.com/boltget/boltget/security/advisories/new](https://github.com/boltget/boltget/security/advisories/new) (repository → Security → Report a vulnerability).
2. Describe the issue, affected version or commit, and steps to reproduce. A proof of concept helps a lot.
3. Tell us how you would like to be credited (or if you prefer to stay anonymous).

If you cannot use GitHub, email [security@boltget.app](mailto:security@boltget.app) with the same information.

## What to expect

- **Acknowledgement** within 3 business days (target).
- **Initial assessment** (severity, whether we can reproduce) within 10 business days (target).
- **Updates** at least every two weeks until resolved.
- **Fix and disclosure:** we aim to release a fix within 90 days of the report, sooner for critical issues, and coordinate the disclosure date with you. We publish a GitHub Security Advisory (with a CVE where appropriate) and credit you unless you ask us not to.

These are targets for a volunteer project; we will tell you if we expect to miss them.

## Scope

In scope (once the code exists):

- The download engine and CLI (`crates/`)
- The desktop app, including its IPC (named pipe), native messaging host and updater (`apps/desktop`)
- The browser extension (`apps/extension`)
- How BoltGet downloads, verifies and runs yt-dlp and ffmpeg
- The release and update infrastructure (signing, update manifests)
- boltget.app

Examples of what we especially want to hear about: a web page or remote server causing BoltGet to write outside the chosen folder, execute code, or leak cookies/credentials to another site; another local user or a website talking to the native messaging host or IPC pipe; bypassing update or sidecar signature verification; Mark-of-the-Web not being applied.

Out of scope:

- Vulnerabilities in yt-dlp, ffmpeg, browsers or the OS themselves (report to those projects), unless BoltGet's use of them makes things worse.
- Downloading a malicious file that the user explicitly chose to download (BoltGet applies Mark of the Web so the OS can protect the user, but cannot judge file contents).
- Attacks that require an already-compromised user account or administrator access.
- Reports from automated scanners without a demonstrated impact.

## Supported versions

BoltGet has no releases yet. After the first release, security fixes are made for the **latest stable release** only (and the current beta). Users are expected to stay updated; the app auto-updates by default.

## Safe harbor

We will not pursue legal action against researchers who act in good faith: who report privately, avoid privacy violations and data destruction, do not degrade services for others, and give us reasonable time to fix the issue before public disclosure.

## Security design

The security model (native messaging allowlist, no localhost server, no remote code, sidecar verification, signed updates, secret handling) is documented in [docs/ARCHITECTURE.md, Security](docs/ARCHITECTURE.md#security).
