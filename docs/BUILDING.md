# Building from source

How to build the Boltget command-line downloader (`boltget`) and the desktop app, including the exact build of a release.

## Requirements

- **Rust:** the stable toolchain. `rust-toolchain.toml` pins the channel, so [rustup](https://rustup.rs) installs the right one on first use. The minimum supported version is the `rust-version` in the root `Cargo.toml`.
- **Windows:** the MSVC build tools (the "Desktop development with C++" workload of Visual Studio or the Build Tools), as for any Rust project on Windows. No other tools are needed: TLS uses rustls with the `ring` provider and SQLite is bundled.
- **Linux:** a C compiler (`gcc` or `clang`) for `ring` and the bundled SQLite.

## Build

```sh
cargo build --release -p boltget-cli
```

The binary is `target/release/boltget.exe` on Windows and `target/release/boltget` on Linux. Run it in place, or copy it anywhere on your `PATH`; it needs no other files.

To run it without copying: `cargo run --release -p boltget-cli -- get <url>`.

## The desktop app

Besides Rust, the app needs [Node.js](https://nodejs.org) (the version in `.node-version`) and pnpm (`corepack enable`, which uses the version in the root `package.json`).

```sh
pnpm install
pnpm --filter @boltget/desktop tauri dev        # run it from source
pnpm --filter @boltget/desktop installer         # build the NSIS installer
```

The installer is `target/x86_64-pc-windows-msvc/release/bundle/nsis/Boltget_<version>_x64-setup.exe`; the first build downloads NSIS into `%LOCALAPPDATA%	auri`. It installs for the current user without administrator rights; `/S` installs silently and `/D=<folder>` chooses the folder. The release workflow renames it to `boltget-<version>-windows-x64-setup.exe`. `BOLTGET_DATA_DIR` gives the app (any build) another data directory, as it does for the CLI.

## Reproduce a release

Releases are built by [`.github/workflows/release.yml`](../.github/workflows/release.yml) from a `vX.Y.Z` tag on `main`. To build the same binary yourself:

```sh
git checkout vX.Y.Z
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
cargo build --locked --release -p boltget-cli --target x86_64-pc-windows-msvc
pnpm install --frozen-lockfile
pnpm --filter @boltget/desktop installer -- --locked
```

Use `x86_64-unknown-linux-gnu` for the Linux build. `--locked` uses the exact dependency versions in the committed `Cargo.lock`. The workflow uses the latest stable Rust at release time; the release notes do not record the version yet, so a byte-identical rebuild may need the same toolchain.

## Verify a download

Every release has a `SHA256SUMS` file and a GitHub build provenance attestation for each asset:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify boltget-X.Y.Z-windows-x64.zip --repo boltget/boltget
```

On Windows without `sha256sum`: `Get-FileHash boltget-X.Y.Z-windows-x64.zip -Algorithm SHA256` and compare with the line in `SHA256SUMS`.

The v0.x builds (the CLI and the app's installer) are not code-signed yet; signing starts with the signed releases planned for v1.0 ([RELEASE.md](RELEASE.md#code-signing)). Windows SmartScreen may warn the first time you run `boltget.exe` or the installer: choose "More info", then "Run anyway", after checking the file.

## Checks

Before sending a change, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The website has its own steps in [WEBSITE.md](WEBSITE.md).
