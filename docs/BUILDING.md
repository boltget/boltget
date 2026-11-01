# Building from source

How to build the Boltget command-line downloader (`boltget`), including the exact build of a release. The desktop app (v0.2) will add its own steps here.

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

## Reproduce a release

Releases are built by [`.github/workflows/release.yml`](../.github/workflows/release.yml) from a `vX.Y.Z` tag on `main`. To build the same binary yourself:

```sh
git checkout vX.Y.Z
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
cargo build --locked --release -p boltget-cli --target x86_64-pc-windows-msvc
```

Use `x86_64-unknown-linux-gnu` for the Linux build. `--locked` uses the exact dependency versions in the committed `Cargo.lock`. The workflow uses the latest stable Rust at release time; the release notes do not record the version yet, so a byte-identical rebuild may need the same toolchain.

## Verify a download

Every release has a `SHA256SUMS` file and a GitHub build provenance attestation for each asset:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify boltget-X.Y.Z-windows-x64.zip --repo boltget/boltget
```

On Windows without `sha256sum`: `Get-FileHash boltget-X.Y.Z-windows-x64.zip -Algorithm SHA256` and compare with the line in `SHA256SUMS`.

The v0.x CLI builds are not code-signed yet; signing starts with the signed releases planned for v1.0 ([RELEASE.md](RELEASE.md#code-signing)). Windows SmartScreen may warn the first time you run `boltget.exe`.

## Checks

Before sending a change, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The website has its own steps in [WEBSITE.md](WEBSITE.md).
