# Changelog

All notable changes to BoltGet will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `boltget get <url>`: a command-line downloader that splits a file across several connections (`-c`, default 8) and keeps re-splitting the largest remaining part, so connections finish together. It shows a live progress bar with speed, ETA and a segment map, prints line-delimited JSON events with `--json`, and documents its exit codes in `boltget get --help`.
- Downloads recover on their own: retries with backoff, `Retry-After` respected, stalled or very slow connections reconnected, redirects followed, and expiring signed download links re-resolved from the original URL.
- Downloads stop instead of producing a corrupt file when the file changes on the server mid-download (ETag / Last-Modified checks on every request).
- File names come from `Content-Disposition` (including international names), are made safe for Windows, and never overwrite an existing file unless you pass `--overwrite`.
- On Windows, partial files are created sparse, so large downloads do not stall on disk preallocation. Completed files get the Mark of the Web, like browser downloads.
- Design documents: vision, features, architecture, engine specification, browser integration, video, tech stack, design principles, website, roadmap and release process (`docs/`).
- Project policies: GPL-3.0-or-later license, contributing guide, governance, support, security policy and code of conduct.
- GitHub configuration: issue forms, pull request template, code owners, Dependabot, label taxonomy, and workflows for PR title and documentation link checks.

[Unreleased]: https://github.com/boltget/boltget/commits/main
