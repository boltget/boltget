# Changelog

All notable changes to BoltGet will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Downloads can be paused and resumed, and survive crashes. `boltget get` records every download under a numeric id; Ctrl-C pauses it (exit code 130) and `boltget resume <id>` continues where it stopped, even after the process was killed or the machine lost power. Progress is saved only once the bytes are on disk, so a resumed file is always byte-identical to the original. If the file changed on the server in the meantime, `resume` stops with a clear error instead of mixing two versions; `boltget resume <id> --restart` downloads it again.
- `boltget list [--json]` shows recorded downloads with their state and progress (it also works while another `boltget` is downloading), `boltget pause <id>` marks a download that is not running as paused, and `boltget remove <id> [--delete-file]` forgets a download and optionally deletes its file.
- The download database lives in `%LOCALAPPDATA%\BoltGet` (Linux: `$XDG_DATA_HOME/boltget`); `--state <dir>` or `BOLTGET_DATA_DIR` choose another directory, and `boltget get --no-state` downloads without recording anything. `Authorization` headers are never saved: pass `-H` again to `resume`. Only one `boltget` at a time can use the database; a second one says so and exits with code 7. Exit code 8 means no such download, or the command does not apply to it (for example resuming a completed download).
- `--json` output: every line now carries the download `id`, a `resumed` event reports the progress a resume continues from, and a `paused` line ends a download stopped with Ctrl-C.
- `boltget get <url>`: a command-line downloader that splits a file across several connections (`-c`, default 8) and keeps re-splitting the largest remaining part, so connections finish together. It shows a live progress bar with speed, ETA and a segment map, prints line-delimited JSON events with `--json`, and documents its exit codes in `boltget get --help`.
- Downloads recover on their own: retries with backoff, `Retry-After` respected, stalled or very slow connections reconnected, redirects followed, and expiring signed download links re-resolved from the original URL.
- Downloads stop instead of producing a corrupt file when the file changes on the server mid-download (ETag / Last-Modified checks on every request).
- File names come from `Content-Disposition` (including international names), are made safe for Windows, and never overwrite an existing file unless you pass `--overwrite`.
- On Windows, partial files are created sparse, so large downloads do not stall on disk preallocation. Completed files get the Mark of the Web, like browser downloads.
- Design documents: vision, features, architecture, engine specification, browser integration, video, tech stack, design principles, website, roadmap and release process (`docs/`).
- Project policies: GPL-3.0-or-later license, contributing guide, governance, support, security policy and code of conduct.
- GitHub configuration: issue forms, pull request template, code owners, Dependabot, label taxonomy, and workflows for PR title and documentation link checks.

[Unreleased]: https://github.com/boltget/boltget/commits/main
