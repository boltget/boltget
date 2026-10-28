# Changelog

All notable changes to BoltGet will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Faster on servers that limit the speed of each connection: the adaptive connection count reaches 16 connections in about 2.5 s instead of about 6 s. On a local server limited to 2 MiB/s per connection, a 100 MiB download now takes 5.1 s instead of 6.8 s (8 fixed connections: 6.6 s). Servers that do not reward more connections, and those that answer `429`/`503`, are handled as before.

### Added

- `boltget bench`: measures download times on your own network. It runs each URL, or the targets in a TOML file such as `bench/targets.toml`, several times (`--runs`, `--warmup`), compares connection settings in rotating order (`-c auto,8`), checks size and SHA-256, and reports the median, interquartile range, min-max, MiB/s, time to first byte and tail time as a table, line-delimited JSON (`--json`) or CSV (`--csv`). Downloaded files are deleted after each run.
- A first benchmark against Internet Download Manager, with method, raw data and limitations: `docs/BENCHMARK.md`. The harness (`bench/`) includes a versioned target list, a PowerShell script that drives IDM and BoltGet on the same files, and `boltget-testserver`, a local server for reproducible lab scenarios.

- The number of connections adapts: a download starts with 4 and adds connections only while they make it faster, reverts ones that do not, and halves when the server answers `429`/`503`, remembering that host's limit for 24 hours. `-c N` still fixes the count; `--max-connections` caps the adaptive one.
- Slow connections no longer hold up the end of a download: a slow connection's remaining bytes are split with an idle connection down to 64 KiB, and a stalled or very slow last connection is replaced within about a second.
- Speed limits: `--limit 5M` (also `500K`, `1048576`; `off` to remove) on `get`, `resume` and `refresh` limits one download and is saved with it; `boltget config set limit 5M` sets a global limit for all downloads, saved in the settings (`config get`, `config unset`, `config list`). Limits change without reconnecting.
- Checksums: `--checksum sha256:<hex>` (also `md5`, `sha1`, `sha512`, or just the hex) checks the finished file. Digests the server sends (`Repr-Digest`, `Digest`, `Content-MD5`, `Content-Digest`, `x-goog-hash`, `x-amz-checksum-sha256`) are checked too unless you pass `--no-server-digest`. The file is hashed while it downloads, so there is no long wait at the end. On a mismatch the file is not renamed, the download is marked `corrupt` and `boltget` exits with code 9; `boltget resume <id> --restart` downloads it again and `boltget resume <id> --no-checksum` keeps it.
- Expired links: a download whose link stops working (401/403/410, a login page instead of the file, or a redirect to an error elsewhere) waits as `needs_new_link` with its progress kept. `boltget refresh <id> <new-url>` continues it from a new link after checking that it is the same file (same size, and the same ETag or matching sample bytes); a link to a different file is refused with the reason.
- Proxies: `--proxy http://host:port`, `https://...` or `socks5://...` (with `user:password@` if needed), or the `http_proxy`, `https_proxy`, `all_proxy` and `no_proxy` environment variables; `--no-proxy` connects directly. Proxy passwords are never logged or saved.
- Cookies and logins: `--cookie-file cookies.txt` (Netscape format, as exported from browsers) sends each request only the cookies that match it, also across redirects, and cookies set by the server are kept for the download; `--cookie name=value` and `--user user:password` (HTTP Basic) are also available. Only the cookie file's path is saved; it is read again on `resume`.
- Downloads can be paused and resumed, and survive crashes. `boltget get` records every download under a numeric id; Ctrl-C pauses it (exit code 130) and `boltget resume <id>` continues where it stopped, even after the process was killed or the machine lost power. Progress is saved only once the bytes are on disk, so a resumed file is always byte-identical to the original. If the file changed on the server in the meantime, `resume` stops with a clear error instead of mixing two versions; `boltget resume <id> --restart` downloads it again.
- `boltget list [--json]` shows recorded downloads with their state and progress (it also works while another `boltget` is downloading), `boltget pause <id>` marks a download that is not running as paused, and `boltget remove <id> [--delete-file]` forgets a download and optionally deletes its file.
- The download database lives in `%LOCALAPPDATA%\BoltGet` (Linux: `$XDG_DATA_HOME/boltget`); `--state <dir>` or `BOLTGET_DATA_DIR` choose another directory, and `boltget get --no-state` downloads without recording anything. `Authorization` headers are never saved: pass `-H` again to `resume`. Only one `boltget` at a time can use the database; a second one says so and exits with code 7. Exit code 8 means no such download, or the command does not apply to it (for example resuming a completed download).
- `--json` output: every line now carries the download `id`, a `resumed` event reports the progress a resume continues from, and a `paused` line ends a download stopped with Ctrl-C.
- `boltget get <url>`: a command-line downloader that splits a file across several connections (adaptive, or a fixed `-c N`) and keeps re-splitting the largest remaining part, so connections finish together. It shows a live progress bar with speed, ETA and a segment map, prints line-delimited JSON events with `--json`, and documents its exit codes in `boltget get --help`.
- Downloads recover on their own: retries with backoff, `Retry-After` respected, stalled or very slow connections reconnected, redirects followed, and expiring signed download links re-resolved from the original URL.
- Downloads stop instead of producing a corrupt file when the file changes on the server mid-download (ETag / Last-Modified checks on every request).
- File names come from `Content-Disposition` (including international names), are made safe for Windows, and never overwrite an existing file unless you pass `--overwrite`.
- On Windows, partial files are created sparse, so large downloads do not stall on disk preallocation. Completed files get the Mark of the Web, like browser downloads.
- Design documents: vision, features, architecture, engine specification, browser integration, video, tech stack, design principles, website, roadmap and release process (`docs/`).
- Project policies: GPL-3.0-or-later license, contributing guide, governance, support, security policy and code of conduct.
- GitHub configuration: issue forms, pull request template, code owners, Dependabot, label taxonomy, and workflows for PR title and documentation link checks.

[Unreleased]: https://github.com/boltget/boltget/commits/main
