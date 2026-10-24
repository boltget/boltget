# Engine specification

The engine (`crates/engine`, crate name `boltget-engine`) is the heart of BoltGet. It is a Rust library used by the desktop app and by the CLI. This document specifies its behaviour in enough detail to implement it and to review implementations against it.

Conventions used here:

- **MUST / SHOULD / MAY** are used in the RFC 2119 sense.
- **Design target** marks a number chosen as a starting point. Every design target is a tunable setting and will be revisited with benchmark data from v0.1. None of them is a measured result.
- Byte ranges are written half-open, `[start, end)`. HTTP `Range` headers are inclusive, so `[start, end)` is sent as `bytes=start-(end-1)`.

Related documents: [ARCHITECTURE.md](ARCHITECTURE.md) (process model, data model, threading), [BROWSER-INTEGRATION.md](BROWSER-INTEGRATION.md) (where cookies and headers come from), [VIDEO.md](VIDEO.md) (media downloads, which bypass most of this engine).

## Contents

1. [Goals](#goals)
2. [Overview of a download](#overview-of-a-download)
3. [Request context](#request-context)
4. [Probing](#probing)
5. [Filename resolution (Content-Disposition)](#filename-resolution-content-disposition)
6. [Redirects](#redirects)
7. [Dynamic segmentation](#dynamic-segmentation)
8. [Connection management and adaptive concurrency](#connection-management-and-adaptive-concurrency)
9. [HTTP/2 considerations](#http2-considerations)
10. [Preallocation and sparse files](#preallocation-and-sparse-files)
11. [Write path](#write-path)
12. [Checkpointing and crash-safe resume](#checkpointing-and-crash-safe-resume)
13. [Retries, backoff and stall detection](#retries-backoff-and-stall-detection)
14. [Link refresh](#link-refresh)
15. [Bandwidth limiting](#bandwidth-limiting)
16. [Integrity](#integrity)
17. [Proxies](#proxies)
18. [Authentication and cookies](#authentication-and-cookies)
19. [Mirrors and multi-source (later)](#mirrors-and-multi-source-later)
20. [Public API sketch](#public-api-sketch)
21. [Testing strategy](#testing-strategy)
22. [Benchmarks plan vs IDM](#benchmarks-plan-vs-idm)
23. [Open questions](#open-questions)

## Goals

In priority order:

1. **Never corrupt or lose completed bytes.** The file on disk plus the checkpoint in SQLite are always consistent after a crash at any instruction.
2. **Saturate the available path.** Use as many connections as help, and no more.
3. **Keep every connection busy until the end.** The last few percent of a download must not run on one slow connection while others sit idle.
4. **Be a good citizen.** Respect per-host limits, `Retry-After`, and server signals that we are opening too many connections.
5. **Bounded resources.** Memory use per download is bounded by configuration, independent of file size.

## Overview of a download

```mermaid
sequenceDiagram
  participant M as Manager
  participant D as Download actor
  participant W as Workers
  participant F as Writer
  participant S as SQLite (DB actor)

  M->>D: start(download)
  D->>W: worker 0: probe GET Range: bytes=0-
  W-->>D: 206, Content-Range: bytes 0-N/size, ETag, Last-Modified
  D->>F: create .boltget file, mark sparse, set length
  D->>S: persist metadata + initial segment table
  D->>W: spawn workers 1..k on split segments
  loop until all segments complete
    W->>F: (offset, bytes)
    F-->>D: durable up to (segment, offset) after flush
    D->>S: checkpoint batch (every interval)
    W-->>D: segment finished
    D->>D: split largest remaining segment
    D->>W: assign new half to a worker
  end
  D->>F: final flush, close
  D->>D: verify checksum (if any)
  D->>F: rename .boltget -> final name, write Zone.Identifier
  D->>S: state = Completed
```

## Request context

Every download carries a `RequestContext`, captured when the download is created and persisted (encrypted, see [ARCHITECTURE.md](ARCHITECTURE.md#persistence)):

| Field | Source | Notes |
|---|---|---|
| `url` | user, browser | The original URL. Kept forever: re-resolving redirects starts here. |
| `method` | browser | Almost always `GET`. `POST`-initiated downloads (form submissions) are supported only if the browser gives us the body; otherwise we report that the download must be handled by the browser. |
| `headers` | browser | Allowlist: `User-Agent`, `Referer`, `Accept`, `Accept-Language`, `Authorization`, `Origin`, plus site-specific headers explicitly allowlisted later. Hop-by-hop headers (`Connection`, `Keep-Alive`, `Transfer-Encoding`, `TE`, `Upgrade`, `Proxy-*`) and `Range`, `If-*`, `Accept-Encoding`, `Host`, `Content-Length` are never copied. |
| `cookies` | browser | Stored as cookie records (name, value, domain, path, secure, httpOnly, expiry), not as one header string. Loaded into a per-download cookie jar. See [Authentication and cookies](#authentication-and-cookies). |
| `proxy` | settings | Resolved per download. |
| `auth` | user | Basic/Digest credentials, stored in the OS credential store. |

Every request the engine sends adds:

- `Accept-Encoding: identity`. Byte ranges apply to the *encoded* representation. If a server compresses on the fly, ranges and sizes become meaningless. We always request the raw bytes.
- `User-Agent`: the browser's if provided, otherwise `BoltGet/<version>`. (Some servers block non-browser agents; the browser-provided value is preferred for that reason, not to impersonate.)

## Probing

The probe determines: total size, whether byte ranges work, validators, filename, MIME type and the final URL. It MUST NOT waste a round trip: **the probe connection becomes worker 0.**

### Why not HEAD

`HEAD` looks like the obvious probe, but it is unreliable in practice:

- Pre-signed URLs (for example S3 and S3-compatible storage) are signed for a specific method. A URL signed for `GET` returns 403 to `HEAD`.
- Some servers and CDNs return different headers (or no `Content-Length`) for `HEAD`, or do not implement it at all.
- Some download endpoints perform side effects (counting a download, consuming a one-time token) on any request, so an extra request can break the real one.

So the engine probes with the request it needs anyway.

### Probe algorithm

1. Send `GET <url>` with `Range: bytes=0-`. (An open-ended range starting at 0 is valid for any non-empty resource and asks for the whole thing.)
2. Interpret the response:

| Response | Meaning | Action |
|---|---|---|
| `206 Partial Content` with `Content-Range: bytes 0-(n-1)/n` | Ranges supported, size `n` known | `resumable = true`. Keep reading on this connection as worker 0 for segment `[0, n)`; split immediately (see [Initial split](#initial-split)). |
| `206` with `Content-Range: bytes 0-x/*` | Ranges supported, total size unknown | Single connection; resumable from the current offset, cannot split. |
| `200 OK` with `Content-Length: n` and `Accept-Ranges: bytes` | Server ignored the range on this request but advertises support (some servers treat `bytes=0-` as "everything" and answer 200) | Keep reading worker 0. Issue one **verification request** `Range: bytes=m-` with `m = n/2` on a second connection. If it returns `206` with a matching `Content-Range`, `resumable = true` and segmentation proceeds with worker 0's segment shrunk to `[0, m)`. If not, `resumable = false`. |
| `200 OK` with `Content-Length: n`, no `Accept-Ranges` or `Accept-Ranges: none` | No range support (probably) | Single connection. Still try the verification request once, because many servers support ranges without advertising them. |
| `200 OK` without `Content-Length` (chunked or close-delimited) | Unknown size, no ranges | Single connection, not resumable. Progress shows bytes only. |
| `416 Range Not Satisfiable` with `Content-Range: bytes */0` | Empty resource | Create an empty file, complete. |
| `3xx` | Redirect | See [Redirects](#redirects). |
| `401`, `403`, `407` | Auth needed or link expired | `407` → proxy auth. `401`/`403` on a fresh download → permanent failure with a clear message (often missing cookies). |
| `4xx` / `5xx` other | | Classified per [Retries](#retries-backoff-and-stall-detection). |

3. Record validators from the response: `ETag` (verbatim, including any `W/` prefix), `Last-Modified`, `Content-Type`, `Content-Disposition`, and the `Date` header (needed to judge whether `Last-Modified` is usable for `If-Range`, see below).
4. Check `Content-Encoding`. If it is present and not `identity` despite our `Accept-Encoding: identity`, the server is compressing anyway: ranges are unsafe. Fall back to a single connection and mark not resumable.
5. Resolve the filename ([below](#filename-resolution-content-disposition)) and destination, then check free disk space for `n` bytes plus a safety margin. If insufficient, fail with `Disk` before writing anything.

### Validators on every subsequent range request

Every range request after the probe MUST carry a validator so a changed resource is detected instead of silently mixing two versions of a file:

- If the probe returned a **strong ETag** (no `W/` prefix): send `If-Range: "<etag>"`.
- Else if it returned `Last-Modified` that is **at least one second earlier than the response `Date`** (the condition RFC 9110 sets for treating a date as a strong validator): send `If-Range: <Last-Modified>`.
- Else: send no `If-Range`, and instead check every `206` response's `Content-Range` total against the known size, and the response `ETag`/`Last-Modified` (if present) against the recorded ones.

Weak ETags MUST NOT be used in `If-Range` (RFC 9110 forbids it).

If a range request with `If-Range` returns `200` instead of `206`, the resource changed. The worker MUST NOT write those bytes at the requested offset. The download moves to `ResourceChanged`.

### Initial split

After a successful probe with known size `n` and range support:

- Compute the initial worker count `k = min(initial_connections, per_host_available, floor(n / min_split_size))`, at least 1.
  - `initial_connections`: design target 8 (IDM's default connection count is also 8, which makes comparison straightforward).
  - `min_split_size`: design target 1 MiB. Below this, a new connection's setup cost (TCP + TLS handshake, slow start) outweighs the parallelism.
- Worker 0 keeps its open-ended request but its segment is set to `[0, n/k)`. It stops reading when it reaches its segment end (see [Ending a segment early](#ending-a-segment-early)).
- Segments `[i·n/k, (i+1)·n/k)` for `i = 1..k-1` are created, with boundaries aligned down to 64 KiB (alignment keeps writes page- and cluster-friendly and makes logs readable). The last segment ends at `n`.
- Workers 1..k-1 are started. They are not all started at once: they are started with a short stagger (design target: 50 ms) to avoid a synchronized burst of TLS handshakes that some servers rate-limit.

## Filename resolution (Content-Disposition)

Order of precedence:

1. A name the user typed in the add dialog.
2. `Content-Disposition` header (RFC 6266):
   - Prefer `filename*` (RFC 8187 extended value, for example `filename*=UTF-8''na%C3%AFve%20file.txt`). Decode the percent-encoding using the declared charset; support `UTF-8` and `ISO-8859-1`.
   - Else `filename` (quoted-string). Many servers put raw UTF-8 bytes or percent-encoded UTF-8 here, against the spec. Heuristic: if the bytes are valid UTF-8, use them; if the value contains `%XX` sequences that decode to valid UTF-8, decode them.
   - `attachment` vs `inline` does not matter for naming.
3. The browser's suggested filename (passed in the request context), because the browser may have already applied its own logic.
4. The last non-empty path segment of the **final** URL, percent-decoded, query string removed.
5. If the result has no extension and `Content-Type` maps to a well-known one (`application/pdf` → `.pdf`, `application/zip` → `.zip`, and so on, from a small built-in table), append it.
6. Fallback: `download` + extension.

Sanitization (Windows rules applied on all platforms, so a file name that works on Windows works everywhere):

- Strip any directory components (`/`, `\`), so a server cannot write outside the destination.
- Replace `< > : " / \ | ? *` and control characters `0x00–0x1F` with `_`.
- Remove trailing dots and spaces.
- If the stem (case-insensitive, before the first dot) is a reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, also with superscript digits `COM¹`–`COM³`, `LPT¹`–`LPT³`), prefix `_`.
- Truncate to 255 UTF-16 code units per component, preserving the extension.
- Long paths: the engine uses `\\?\`-prefixed paths internally (or relies on the process being long-path aware via the application manifest), so destinations deeper than 260 characters work.

Collision handling: if the target exists (or another queued download targets the same path), append ` (1)`, ` (2)`, ... before the extension. The add dialog shows the final name before starting.

## Redirects

- Follow up to 10 redirects (`301`, `302`, `303`, `307`, `308`). `303` switches to `GET`; the engine only downloads with `GET` anyway.
- Record the chain. `final_url` is stored in the DB and used for segment requests.
- On a redirect to a different origin:
  - Drop `Authorization` (unless the user configured credentials for that origin).
  - Cookies are not a concern because they come from a cookie jar that matches by domain (see [Authentication and cookies](#authentication-and-cookies)), never from a blanket `Cookie` header.
  - Keep `Referer` as provided by the browser.
- Redirect from `https` to `http` is allowed only if the user opted in (setting off by default); otherwise fail with a clear error.
- **Expiring redirect targets.** Many hosts redirect a stable URL to a short-lived signed URL (release assets on code-hosting sites, cloud storage, CDNs). When a segment request to `final_url` fails with `LinkExpired`, the engine MUST first re-resolve from the original `url` (fresh redirect chain), validate the new `final_url` like a [link refresh](#link-refresh), and continue. Only if that fails does the download move to `NeedsNewLink`.

## Dynamic segmentation

This is the core of what makes a segmented downloader fast at the end of a download, not just at the start.

### Segment table

The download actor owns a table of segments. Each segment has:

```rust
struct Segment {
    idx: u32,          // stable id
    start: u64,        // inclusive
    end: u64,          // exclusive; may shrink when split
    received: u64,     // bytes received from network (start + received = receive cursor)
    durable: u64,      // bytes known durable on disk (durable <= received)
    worker: Option<WorkerId>,
}
```

Invariants (checked in debug builds and property tests):

1. Segments are disjoint and their union is `[0, n)`.
2. `start + durable <= start + received <= end`.
3. A byte is written to disk by exactly one worker.
4. The persisted checkpoint for a segment never exceeds its `durable` value.

### Steady state

Each worker runs a loop on its assigned segment: read chunks from the response body and forward them to the writer until `start + received == end`. Then it reports `SegmentFinished` and asks the actor for more work.

### Split rule

When a worker becomes free (its segment finished) or a new connection slot opens (adaptive concurrency raised the target, or a per-host slot freed up):

1. Pick the segment with the **largest remaining bytes**, where `remaining = end - (start + received)`, among segments that currently have a worker.
2. Estimate how many bytes the owning worker will receive before it learns about the split: `in_flight = owner_throughput × (rtt_estimate + reaction_slack)`. (Design target: `reaction_slack` = 100 ms.)
3. Let `effective_remaining = remaining - in_flight`. If `effective_remaining < 2 × min_split_size`, **do not split**; see [Endgame](#endgame) instead.
4. Split point `mid = align_down(start + received + in_flight + effective_remaining / 2, 64 KiB)`.
5. Under the actor's state (single-threaded by construction, so no lock), set the owner's `end = mid` and create a new segment `[mid, old_end)`.
6. Assign the new segment to the free worker, which issues `Range: bytes=mid-(old_end-1)` with the validator.
7. Tell the owner its new end. If the owner has already received bytes past `mid` by the time it sees the message (the in-flight estimate was too low), it MUST discard the bytes beyond `mid` rather than write them, since the new worker owns them now. A discard is a small waste, never a correctness problem.

Unassigned segments (from a resumed download, or from a worker that failed) are always picked before splitting an active one.

Splitting the *largest remaining* segment in half is what keeps all connections finishing at roughly the same time: the work is continuously rebalanced toward wherever the most bytes remain. This is the same idea IDM describes as dynamic file segmentation.

### Ending a segment early

When a segment's `end` shrinks, the owner's HTTP request may still be open for bytes beyond the new end:

- **HTTP/1.1:** the worker stops reading at `end`. The rest of the response is still in flight, so the connection cannot be reused for another request; it is closed. This costs one new connection when the worker gets its next segment. To reduce this cost, workers SHOULD request closed ranges (`bytes=a-b`) for the segments they are assigned, so a worker that completes its segment *without* having been split has fully consumed its response and can reuse the keep-alive connection for its next range.
- **HTTP/2:** the worker cancels its stream (`RST_STREAM` with `CANCEL`). The connection stays usable.

Worker 0 (the probe connection) is the exception: it starts with an open-ended range because the size was unknown at the time. It always ends early.

### Endgame

When no segment has enough remaining bytes to split, but some connections are idle, a slow connection can still hold up completion (for example, one connection hit a slow CDN edge). Endgame mode:

- Trigger: all remaining work is in segments smaller than `2 × min_split_size`, and at least one connection is idle, and the slowest active segment's estimated time to finish exceeds a threshold (design target: 2 seconds) and is more than 2x the median of the others.
- Action: an idle worker requests the **tail half** of the slowest segment as a duplicate. Whichever worker reaches a byte first owns it; the actor shrinks the slow worker's `end` as soon as the fast one starts receiving. Bytes are still written exactly once (invariant 3), because ownership is transferred before writing.
- Limited to one duplicate per segment, so endgame never more than doubles the traffic for the last few MiB.

Endgame is marked **experimental** for v0.1 and is only enabled by default if benchmarks show a benefit.

### Single-connection downloads

If `resumable = false`, the download runs on worker 0 only, with no splits. If interrupted, it restarts from zero (the UI says so up front: "This server does not support resuming").

If the size is unknown but ranges work (`Content-Range: bytes 0-x/*`), the download is resumable from its cursor but cannot be split.

## Connection management and adaptive concurrency

### Limits

Three caps apply simultaneously; the effective number of workers for a download is the minimum:

| Limit | Default (design target) | Range | Notes |
|---|---|---|---|
| Per download (`max_connections`) | 16 | 1–32 | User-visible setting, overridable per download |
| Per host (`max_per_host`) | 16 | 1–32 | Shared by all downloads to the same host (scheme + host + port). Some servers ban clients above a threshold; the per-host cap is what protects the user's IP. |
| Global (`max_total`) | 64 | 1–256 | Protects the local machine and router (NAT tables on cheap routers) |

### Adaptive connection count

Fixed connection counts are wrong for most servers: too few leaves throughput on the table when the server throttles per connection; too many triggers throttling, wastes handshakes, and can be slower over a congested last mile. The engine adjusts the target worker count per download with a simple hill-climbing controller:

1. Start at `k0 = initial_connections` (or fewer for small files, per the initial split).
2. Every measurement window (design target: 2 s, using an exponentially weighted moving average of aggregate throughput), compare with the previous window:
   - If the last change was an **increase** and throughput rose by more than `gain_threshold` (design target: 5%), increase again by 1–2 connections, up to the caps.
   - If throughput did **not** rise meaningfully, revert the last increase and hold for a cool-down period (design target: 10 s) before probing again.
   - If throughput **dropped**, or the server returned `429`/`503`, or connections are being reset or refused at connection setup, decrease by half (multiplicative decrease) and record a lower ceiling for that host for the session.
3. Never go below 1 or above the caps.
4. Near the end of the download (fewer than `target` splittable segments), the controller stops increasing.

Per-host learned ceilings are kept in memory for the session and persisted (design target: for 24 hours) so the next download from a strict server starts at a sensible count instead of rediscovering the limit.

The controller is deliberately simple. If benchmarks show it oscillating, alternatives to evaluate are a BBR-style probe-and-drain cycle or a fixed schedule based on file size. Any change MUST come with benchmark data.

### Connection reuse and DNS

- One `hyper` connection per worker, created explicitly (not drawn from a shared pool), so the engine controls how many TCP connections exist. See [HTTP/2 considerations](#http2-considerations).
- DNS is resolved once per download and the resulting addresses are shared by workers. If a host resolves to several addresses, workers MAY be spread across them (opt-in, off by default: some CDNs require session affinity).
- TCP: `TCP_NODELAY` is irrelevant for bulk download; receive buffer sizes are left to the OS autotuning by default.
- TLS: rustls, session resumption enabled (tickets), so additional workers to the same host skip the full handshake.
- Happy Eyeballs (RFC 8305) for dual-stack hosts.

## HTTP/2 considerations

Multiple HTTP/2 **streams** on one TCP connection share a single congestion window and a single TCP receive path. For a download manager this means:

- Opening 16 streams on one HTTP/2 connection does **not** give the speedup of 16 TCP connections when the bottleneck is per-connection throttling or a long fat network. It may be no faster than one stream.
- HTTP clients that pool connections (including `reqwest` by default) multiplex requests to the same origin onto one HTTP/2 connection. Using such a pool for workers would silently defeat segmentation.

Therefore:

1. Workers use `hyper`'s low-level client connection API (`hyper::client::conn::{http1, http2}`) with **one connection per worker**, each with its own TCP + TLS session. Each HTTP/2 connection carries one active stream at a time.
2. ALPN offers `h2` and `http/1.1`. Which one is better for bulk transfer is server-dependent; the default (design target) is to accept what the server picks, and the benchmark suite includes HTTP/1.1-only and HTTP/2 scenarios to decide whether to prefer `http/1.1` for workers.
3. HTTP/2 flow control: set a large initial stream and connection window (design target: 8 MiB) so the window is not the bottleneck on high bandwidth-delay paths; hyper's adaptive window option is evaluated as an alternative.
4. Some servers count HTTP/2 connections from one client more strictly than HTTP/1.1 connections. Adaptive concurrency handles this the same way (connection refusal or `GOAWAY` → decrease).
5. `GOAWAY` handling: finish the current range if the stream is below the last-stream-id, otherwise reconnect and continue from the cursor without counting it as an error.

HTTP/3 (QUIC) is post-1.0. QUIC's per-connection congestion control has the same multiplexing caveat; the per-worker-connection model carries over.

## Preallocation and sparse files

The engine creates the output file as `<final name>.boltget` in the destination directory (same volume as the final file, so the final rename is atomic) and sets its length to the full size before workers start writing at scattered offsets.

### Why NTFS needs care

On NTFS, extending a file with `SetEndOfFile` (Rust `File::set_len`) is cheap: it does not write zeros. But NTFS tracks a **valid data length (VDL)** per file. A write at an offset beyond the VDL forces NTFS to zero-fill everything between the VDL and the write offset first, synchronously, inside the write call. With segmented downloading, the first write by the last segment's worker lands near the end of the file, so a 50 GB download can trigger a write that blocks until tens of gigabytes of zeros are written. That stall looks exactly like a hung download.

`SetFileValidData` avoids this but requires the `SE_MANAGE_VOLUME_NAME` privilege (administrator) and exposes stale disk contents; BoltGet will not use it.

### Strategy on Windows

1. Create the file and mark it **sparse** with `DeviceIoControl(FSCTL_SET_SPARSE)`. In a sparse file, unwritten ranges are not allocated and read as zeros, and writes at any offset do not trigger zero-fill of the gap.
2. `set_len(n)`.
3. Workers write at their offsets.
4. On completion, the file is fully written, so every range is allocated. The sparse attribute is harmless at that point. Whether to clear it (`FSCTL_SET_SPARSE` with `SetSparse = FALSE`) is an [open question](#open-questions): it is cosmetic for a fully written file, and we will measure whether it matters for any tool.

Trade-off: a sparse file allocates clusters as writes arrive, so a heavily segmented download can end up more fragmented than a fully preallocated file. On SSDs this does not matter; on HDDs, larger segments (and the 64 KiB alignment) limit fragmentation. A "full preallocation" mode for HDDs is a possible setting, but it means writing `n` zero bytes up front, which costs time and disk wear. Benchmark first.

Other file systems:

- **exFAT** also has a valid data length concept and **no sparse files**, so writes far beyond VDL zero-fill. (This affects USB drives and SD cards.) Strategy: start segments in order and limit how far ahead of the lowest incomplete offset new segments may begin (a "write window"), so zero-fill happens incrementally in the background rather than as one giant stall.
- **FAT32:** no sparse files; also a 4 GiB per-file limit. The engine MUST check the destination file system and fail early with a clear message if `n ≥ 4 GiB` on FAT32.
- **ReFS:** supports sparse files; same strategy as NTFS.
- **Network shares (SMB):** behaviour depends on the server. Use the sparse attempt and fall back to plain `set_len` if `FSCTL_SET_SPARSE` fails.

Unix (post-1.0): `ftruncate` creates sparse files by default on ext4, XFS, Btrfs and APFS. `fallocate(FALLOC_FL_KEEP_SIZE)` / `F_PREALLOCATE` is an optional full-preallocation mode.

## Write path

### Positional writes

All writes are positional; there is no shared file cursor:

- Windows: `std::os::windows::fs::FileExt::seek_write`. (It also moves the handle's file pointer, which is irrelevant because nothing relies on it.) Short writes are possible in principle; the writer loops until the whole buffer is written.
- Unix: `std::os::unix::fs::FileExt::write_all_at` (`pwrite`).

### Writer design

- One writer per download, running on a dedicated thread (writes are blocking syscalls; doing them on Tokio worker threads would stall networking).
- Workers send `WriteChunk { segment, offset, bytes: Bytes }` over a **bounded** channel. Channel capacity times chunk size bounds memory per download.
  - Design target: chunks are coalesced per worker to 1 MiB before sending (network reads are typically 16–64 KiB), channel capacity 16 chunks, so roughly 16 MiB per download in flight plus per-worker coalescing buffers.
- When the channel is full, workers await, stop reading the socket, and TCP flow control slows the sender. This is the intended backpressure, not an error.
- The writer tracks, per segment, the highest contiguous offset written (`received` on disk but not yet flushed).
- Unbuffered (`FILE_FLAG_NO_BUFFERING`) or overlapped I/O is not used in v0.1. The OS page cache plus 1 MiB writes is expected to be adequate; if benchmarks show the writer is the bottleneck at multi-gigabit speeds, evaluate overlapped I/O or io_uring (Linux) then.

### Flush (fsync) policy

Data must be durable before the checkpoint that claims it is written:

- The writer flushes the file (`FlushFileBuffers` on Windows, `fdatasync` on Unix; `File::sync_data` in Rust) at the **checkpoint interval** (design target: every 2 seconds, or every 64 MiB written, whichever comes first), not after every write. Flushing on every write would cost far more than the network transfer.
- After a successful flush, the writer reports `Durable { segment → offset }` for all segments to the download actor, which updates each segment's `durable` field.
- On pause, cancel, or app shutdown, the writer drains its channel, flushes, and reports final durable offsets.
- On completion, a final flush happens before verification and rename.

## Checkpointing and crash-safe resume

### The invariant

> For every segment, the progress stored in SQLite is less than or equal to the number of bytes that are durable in the file.

If this holds, resume is always correct: after any crash, the engine re-downloads at most the bytes between the last checkpoint and the crash, and never trusts bytes that might not be on disk.

### Order of operations

1. Writer: `write` chunks (page cache).
2. Writer: `sync_data()` returns successfully.
3. Writer → actor: durable offsets.
4. Actor → DB actor: one transaction updating `segment.done` for all changed segments and `download.downloaded`.
5. DB commit (WAL).

A crash between 2 and 5 leaves the DB behind the file (safe). With `synchronous=NORMAL`, a power loss can roll back the most recent WAL commits, again leaving the DB behind (safe). There is no ordering in which the DB can claim bytes that are not durable.

Segment splits are also persisted (new segment rows) in the same checkpoint transaction. A split that happened after the last checkpoint is simply lost on crash; on resume, the old larger segment is re-split as needed.

### Resume procedure

On startup (or when a paused download is resumed):

1. Load the download and its segments.
2. Open `<name>.boltget`. If it is missing, or its length is not `total_size`, the partial data is gone: offer to restart from zero.
3. Re-probe: `GET` with `Range: bytes=<first incomplete offset>-` and `If-Range` (validator from the original probe). This is the first worker's request, not an extra round trip.
4. Validate: `206` with consistent `Content-Range` total, matching ETag / Last-Modified. Any mismatch → `ResourceChanged` (or [link refresh](#link-refresh) flow if the URL is dead).
5. Rebuild the segment table from `start + done` cursors, merge completed segments, assign workers to incomplete segments, and continue normally.

### Shutdown

On app exit (including Windows logoff/shutdown, `WM_QUERYENDSESSION` / `WM_ENDSESSION`), the engine pauses all downloads, which flushes and checkpoints. Design target: complete within 2 seconds; if the OS kills the process sooner, the invariant still guarantees a correct resume.

## Retries, backoff and stall detection

### Classification

See the error table in [ARCHITECTURE.md, Error handling](ARCHITECTURE.md#error-handling). Retries happen **per worker connection**, not per download: one failing connection does not stop the others.

### Backoff

Exponential backoff with full jitter:

```
delay = random_uniform(0, min(max_delay, base × 2^attempt))
```

- Design targets: `base` = 1 s, `max_delay` = 60 s, `max_attempts_per_segment` = 10 consecutive failures without progress. Any received byte resets the attempt counter for that segment.
- `Retry-After` (seconds or HTTP-date) on `429`/`503` overrides the computed delay, capped at a maximum (design target: 15 minutes) beyond which the download is paused with a message.
- If all workers of a download are failing at the same time, the download enters `Retrying` and the UI shows the next retry time. If the network is down (OS reports no connectivity), retries pause until connectivity returns instead of burning attempts.

### Stall and slow-connection detection

- **Stall:** no bytes received on a connection for `stall_timeout` (design target: 30 s) → abort and reconnect from the cursor.
- **Connect timeout:** design target 15 s. **TLS handshake timeout:** design target 15 s. **First byte after request:** design target 30 s.
- **Slow connection:** a worker whose throughput stays below a fraction of the per-worker median (design target: 10% for 20 s), while others are healthy, is closed and its segment reassigned (or split). This handles a single connection landing on a bad CDN edge.

## Link refresh

Download links often expire: pre-signed cloud storage URLs, file-host session links, tokens in the query string. IDM's "refresh download address" lets the user supply a new link and continue; BoltGet does the same, with stricter validation.

### Detecting expiry

A download moves to `NeedsNewLink` when:

- A request to a URL that previously worked returns `401`, `403` or `410`, or
- The response is `200`/`206` with `Content-Type: text/html` where the file was not HTML (a login or error page served with a success status), or
- The redirect chain now ends somewhere else entirely (different host and a 4xx), and
- Re-resolving from the original URL (see [Redirects](#redirects)) did not fix it.

`404` is not automatically treated as expiry (it often means the file is gone), but the user can still choose "refresh link".

### Providing a new link

- **Manually:** "Refresh link" in the app, paste a URL.
- **Browser-assisted (v0.3):** the user starts the download again in the browser (for example by clicking the download button on the site, which generates a fresh signed link). The extension captures it as usual; the app notices that a download waiting for a new link has the **same filename and size** and offers: "Use this link to resume *file.iso* (73% done)?" This is the flow most users need, because they cannot copy a signed URL by hand.
- The new request context (cookies, headers) replaces the old one.

### Validating the new link

Before any byte from the new URL is written:

1. Probe the new URL with `Range: bytes=0-` (as a normal probe).
2. **Size MUST match** `total_size` exactly. If not: reject.
3. Compare validators:
   - Both strong ETags present and equal → accept.
   - Both present and different → do not reject outright (different CDN edges or a different storage backend can produce different ETags for the same bytes), go to step 4.
   - `Last-Modified` equal → supporting evidence; still do step 4 unless ETags matched.
4. **Sample verification:** fetch a few small ranges from regions that are **already completed** (design target: 3 ranges of 64 KiB at pseudo-random offsets, including one near the end of the completed region) and compare them byte-for-byte with the file on disk. All must match.
5. If accepted, add the URL to `download_source` with role `refreshed`, update `final_url` and validators, and resume. If rejected, explain why ("Size differs: this is a different file").

Sample verification costs a few hundred KiB and turns "probably the same file" into "very likely the same file" without trusting server metadata alone. It does not prove identity; a final checksum, if known, does.

## Bandwidth limiting

### Model

Hierarchical token buckets:

- **Global bucket**: rate `R_global`, burst `B_global`.
- **Per-queue bucket** (optional, v0.5).
- **Per-download bucket**: rate `R_dl`, burst `B_dl`.

A worker that wants to read `c` bytes MUST acquire `c` tokens from every bucket on its path (download → queue → global). Acquisition is async: if tokens are insufficient, the worker sleeps until the refill time, then retries.

- Token granularity: workers acquire in units of the read size (design target: 16 KiB); larger acquisitions increase burstiness.
- Burst: design target 250 ms worth of rate (or at least one read unit). A bucket with too small a burst cannot sustain its rate at high speed because of timer resolution; too large and the limit is visibly bursty.
- Fairness: within a download, workers compete fairly for tokens through a FIFO waiter queue (a `tokio::sync::Semaphore`-like structure refilled by a timer, or a lock-free GCRA implementation; to be chosen in v0.1). Across downloads, per-download buckets plus the FIFO global queue prevent one download from starving others.
- Changing a limit takes effect within one refill interval, without reconnecting.

### Where throttling happens

Throttling is applied to **reads from the socket**. When a worker stops reading, the kernel receive buffer fills, the TCP receive window closes, and the sender slows down. This means:

- Short bursts above the limit (up to the size of the socket receive buffer) are unavoidable and expected.
- The limit applies to payload bytes. TLS and HTTP overhead is not counted.
- Very low limits on very fast connections can cause servers to time out an idle-looking connection; the stall timer accounts for intentional throttling (time spent waiting for tokens does not count as a stall).

### Scheduler integration

Queues and the scheduler (v0.5) can change limits by time of day ("unlimited from 01:00 to 07:00, 2 MB/s otherwise"). The engine exposes `set_limit(scope, rate)`; the schedule logic lives in the manager.

## Integrity

### What can go wrong

- **Network corruption:** for HTTPS, TLS authenticates every record, so corrupted bytes cause a connection error rather than bad data. For plain HTTP, TCP checksums are weak; corruption is rare but possible.
- **Logic bugs:** writing bytes at the wrong offset, double-writing a range with data from two different versions of a file, losing a chunk. This is the most likely source of corruption in any segmented downloader, which is why the invariants above are enforced and property-tested.
- **Resource changed mid-download** without validators to detect it.
- **Disk errors** after writing.

### Checksum verification

- The user can attach an expected checksum when adding a download (SHA-256, SHA-512, SHA-1, MD5; algorithm inferred from the hex length or chosen explicitly), or paste one later.
- **Automatic discovery** (v0.5), all opt-out:
  - `Repr-Digest` / `Content-Digest` headers (RFC 9530) and the older `Digest` header (RFC 3230), when they describe the whole representation.
  - Cloud storage headers such as `x-goog-hash` (MD5 / CRC32C) and `x-amz-checksum-sha256` when present for the full object. (S3 multipart ETags are not MD5 of the file and MUST NOT be treated as one.)
  - A sibling checksum file next to the URL (`<file>.sha256`, `SHA256SUMS`) when the browser captured it or the user points to it. Never fetched speculatively by default.
- Verification runs after all bytes are durable: a sequential read of the file computing the hash (streaming, constant memory, on a blocking thread, with progress). SHA-256 must be computed in file order, so it cannot be done incrementally while segments arrive out of order.
- Mismatch → `Corrupt` state. The UI offers "Re-download" (from zero) and "Keep anyway".

### Re-fetching corrupted ranges

A whole-file hash says *that* the file is wrong, not *where*. Targeted repair needs piece-level hashes:

- **Metalink** (RFC 5854) `<pieces>` hashes, and other formats that provide per-piece hashes, allow verifying each piece independently and re-fetching only bad pieces. Supported when Metalink support lands (post-1.0).
- **Without piece hashes,** BoltGet can offer "Repair": re-download the file in segments while comparing each range with the existing bytes, rewriting only ranges that differ. This re-uses bandwidth but avoids discarding the file, and it reports which ranges were wrong, which is valuable for bug reports. Repair is a v0.5 feature.

### Internal checks

- The engine counts bytes per segment and asserts the totals equal `total_size` at completion.
- In debug builds and in the test suite, an optional per-chunk CRC32C log records what was written where, so tests can prove no range was written twice with different data.

## Proxies

- Supported: HTTP proxy (plain HTTP via absolute-form requests; HTTPS via `CONNECT`), HTTPS proxy (TLS to the proxy), SOCKS5 with remote DNS resolution (the equivalent of `socks5h`), SOCKS5 username/password auth. SOCKS4 is not planned.
- Proxy auth: Basic and Digest for HTTP proxies. NTLM/Negotiate (common in corporate environments) is post-1.0 and needs investigation.
- Configuration precedence: per-download override → per-category → global setting → system proxy (desktop app) / environment variables `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` (CLI).
- **System proxy on Windows (v0.2):** read the current user's WinINet proxy settings (static proxy and bypass list). PAC/WPAD support is post-1.0; when a PAC URL is configured and unsupported, BoltGet shows that it is ignoring it rather than failing silently.
- The extension can also tell the app which proxy the browser used, but browsers do not expose this reliably; v1 relies on system and app settings.

## Authentication and cookies

- **Cookies from the browser:** the extension sends cookie records for the download URL (and the final URL, if the browser saw the redirect). The engine builds a per-download cookie jar (domain/path/secure matching as in RFC 6265) and sends only the cookies that match each request's URL. This handles redirects across domains correctly and never leaks cookies to a third party. `Set-Cookie` from responses is applied to the jar (some hosts set a session cookie on the redirect hop).
- Cookie expiry: expired cookies are dropped. If a download is resumed days later with expired session cookies, the server will likely reject it and the [link refresh](#link-refresh) flow takes over.
- **HTTP auth:** Basic and Digest (RFC 7616) via `401` + `WWW-Authenticate`. Credentials are requested from the user, optionally saved per origin in the Windows Credential Manager. Basic auth over plain HTTP prompts a warning.
- **Bearer tokens / custom headers:** passed through from the browser's request headers (allowlisted) or added manually in the add dialog's advanced section.
- All of the above are secrets: encrypted at rest, redacted in logs, deleted after completion by default (see [ARCHITECTURE.md, Security](ARCHITECTURE.md#security)).

## Mirrors and multi-source (later)

Post-1.0. The segment model already supports it: a segment is just a byte range; *which URL* serves it is a worker property.

- Sources: user-added mirror URLs, Metalink files (RFC 5854) and `Link: <...>; rel=duplicate` headers (RFC 6249).
- Every mirror is validated like a [link refresh](#link-refresh) (size, validators, sample bytes) before it serves a segment.
- Per-source throughput is tracked; adaptive concurrency allocates more workers to faster mirrors, and a failing mirror is benched with backoff.
- Piece hashes from Metalink enable per-piece verification and targeted repair.

## Public API sketch

The shape of the API the app and CLI program against. Names are indicative.

```rust
pub struct Engine { /* ... */ }

impl Engine {
    pub async fn open(config: EngineConfig) -> Result<Engine, Error>;   // takes the owner lock, runs migrations
    pub async fn add(&self, req: AddRequest) -> Result<DownloadId, Error>;
    pub async fn probe(&self, req: &RequestContext) -> Result<ProbeResult, Error>; // for the add dialog
    pub async fn pause(&self, id: DownloadId) -> Result<(), Error>;
    pub async fn resume(&self, id: DownloadId) -> Result<(), Error>;
    pub async fn remove(&self, id: DownloadId, delete_file: bool) -> Result<(), Error>;
    pub async fn refresh_link(&self, id: DownloadId, ctx: RequestContext) -> Result<RefreshOutcome, Error>;
    pub async fn set_limit(&self, scope: LimitScope, bytes_per_sec: Option<u64>) -> Result<(), Error>;
    pub async fn list(&self, filter: ListFilter) -> Result<Vec<DownloadSummary>, Error>;
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<EngineEvent>;
    pub fn progress(&self, ids: &[DownloadId]) -> Vec<Progress>;          // lock-free snapshot from atomics
    pub async fn shutdown(self) -> Result<(), Error>;                      // pause all, flush, checkpoint
}

pub enum EngineEvent {
    Added(DownloadSummary),
    StateChanged { id: DownloadId, state: State, error: Option<ErrorInfo> },
    SegmentsChanged { id: DownloadId },     // split, merge, reassign (UI pulls details)
    Completed { id: DownloadId, path: PathBuf },
    NeedsNewLink { id: DownloadId },
}
```

The CLI maps directly onto this API:

```
boltget get <url> [-o <path>] [-c <max-connections>] [--limit 5M] [--sha256 <hex>]
               [--header 'Name: value'] [--cookie-file cookies.txt] [--proxy socks5://...]
boltget list [--json]
boltget pause|resume|remove <id>
boltget refresh <id> <new-url>
boltget bench <url> [--runs 5] [--connections 8,16,32] [--json]
```

`--cookie-file` accepts the Netscape `cookies.txt` format, which browsers' export extensions and other tools produce.

## Testing strategy

- **Unit tests:** probe interpretation table, Content-Disposition parsing (a corpus of real-world headers, including broken ones), filename sanitization, split rule arithmetic, token bucket timing (with Tokio's paused clock), backoff.
- **Property tests (`proptest`):** random sequences of chunk arrivals, splits, worker failures, pauses and crashes against an in-memory model; assert the four [segment invariants](#segment-table) and that the final file equals the source.
- **Integration tests against a local test server** (a small Rust server in `crates/engine/tests/support` built on hyper) that can: disable ranges, ignore `bytes=0-`, change content mid-download, return 200 to `If-Range`, throttle per connection, reset connections at random, expire URLs after N requests, redirect through signed URLs, serve wrong `Content-Length`, and speak HTTP/1.1 or HTTP/2.
- **Crash tests:** a harness that starts a download in a child process, kills it (`TerminateProcess`) at random times, resumes, repeats, and compares the final SHA-256. Run thousands of iterations nightly.
- **Fuzzing (`cargo-fuzz`):** header parsers (Content-Disposition, Content-Range, Retry-After, cookies) and the native messaging decoder.
- **File system matrix (Windows CI and manual):** NTFS, exFAT USB drive, FAT32 (4 GiB check), ReFS where available, an SMB share.

## Benchmarks plan vs IDM

The v0.1 milestone ships with a published benchmark. The goal is a fair, reproducible comparison, not a marketing number.

### Principles

- Same machine, same network, same time window, same file, same connection count.
- Alternate the order of runs (IDM, BoltGet, IDM, BoltGet, ...) to cancel out drift in network conditions.
- Publish everything: scripts, raw timings, system info, and the scenarios BoltGet loses.
- The person running the benchmark records the IDM version and settings. (We should check IDM's license terms regarding publishing benchmark results before publishing; see [open questions](#open-questions).)

### Environments

1. **Controlled (lab):** a local test server on a second machine (or VM) on a wired gigabit LAN, with network conditions shaped by `tc netem` / `tc tbf` on the Linux server side:
   - Per-connection throttling (the scenario where segmentation matters most), for example each connection capped at a fixed rate.
   - Added latency (design targets for scenarios: 20 ms, 100 ms, 250 ms RTT) and loss (0%, 0.5%, 2%).
   - A connection-count limit at the server (refuses connections above a threshold), to test adaptive concurrency.
   - HTTP/1.1 only, and HTTP/2.
2. **Real world:** public, freely redistributable large files from well-provisioned hosts, such as Linux distribution ISO images from official mirrors and hosting providers' public speed-test files. Use them sparingly and in line with each host's terms. The list of URLs is versioned in `bench/targets.toml`.

### Test files

- Sizes: 10 MiB, 100 MiB, 1 GiB, 10 GiB (the last in the lab only).
- Content: random data (incompressible; also ensures no transparent compression skews results).
- A known SHA-256 for each, verified after every run for both tools.

### Metrics

| Metric | How |
|---|---|
| Wall-clock time to complete | From the moment the download is started to the file being complete on disk. For IDM, measured externally: start via `idman.exe` command-line switches, detect completion by the final file appearing with the expected size and hash. For BoltGet, the same external measurement, plus internal timestamps. |
| Average throughput | size / time |
| Throughput over time | BoltGet internal 100 ms samples; for IDM, sample the partial file's growth (IDM writes to temporary files, so this may not be possible; see open questions) or NIC counters on a dedicated machine |
| Time to first byte | BoltGet internal; IDM n/a |
| Tail time | Time from 95% to 100%, which is where dynamic segmentation pays off |
| Connections used | Server-side connection log in the lab |
| CPU and RAM | Process counters sampled at 1 Hz (Windows performance counters) |
| Bytes on wire vs file size | Server-side byte counts; measures waste from discarded bytes and endgame duplicates |
| Resume correctness | Kill-and-resume cycles; SHA-256 of the result |

### Procedure

- Warm-up run discarded for each tool and scenario.
- At least 5 measured runs per tool per scenario (more on the real-world targets, where variance is higher).
- Report median and interquartile range; show every run in the raw data.
- Configurations: IDM default settings (8 connections), IDM at maximum connections (32), BoltGet default (adaptive), and BoltGet fixed at the same counts as IDM.
- A difference is only claimed if it is larger than the run-to-run spread.

### Deliverables

- `bench/` directory with the harness (Rust or PowerShell scripts), scenario definitions and the server-side shaping scripts.
- `boltget bench` command for users to run BoltGet-only measurements on their own network.
- A results page on the website, dated, with the exact versions tested.

## Open questions

These need a decision, ideally backed by measurement, during v0.1:

1. **Default connection count and `min_split_size`.** 8 and 1 MiB are starting points chosen for comparability with IDM, not measured optima.
2. **HTTP/1.1 vs HTTP/2 for workers.** Prefer whatever the server negotiates, or prefer HTTP/1.1 for bulk ranges? Decide from benchmark data.
3. **Clear the sparse flag on completion?** Measure whether any tool or file system behaviour depends on it.
4. **exFAT write window size.** How far ahead of the lowest incomplete offset to allow segments on non-sparse file systems.
5. **rustls crypto provider** (`aws-lc-rs` vs `ring`): performance at multi-gigabit TLS throughput vs build friction on Windows CI.
6. **Benchmark publication and IDM's EULA.** Confirm that publishing comparative benchmark results does not conflict with IDM's license terms.
7. **Measuring IDM's throughput curve.** IDM's partial-file layout is not documented; NIC counters on a dedicated benchmark machine may be the only reliable source.
