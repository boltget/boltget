//! Link refresh: checking that a new link leads to the same file before any of
//! its bytes are written (`docs/ENGINE.md`, "Validating the new link").
//!
//! 1. Probe the new URL like a fresh download (`Range: bytes=0-`). Its body is
//!    never written; the connection is dropped.
//! 2. The size must equal the download's size exactly.
//! 3. Equal strong `ETag`s are accepted as proof.
//! 4. Otherwise (different or missing `ETag`s: another CDN edge or storage
//!    backend may tag the same bytes differently), up to three 64 KiB ranges
//!    from regions that are already on disk are fetched and compared byte for
//!    byte with the partial file: one at the end of the completed region and
//!    two at pseudo-random offsets.

use std::path::Path;

use http::StatusCode;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::{Error, Result};
use crate::headers::{ContentRange, Validators};
use crate::http::{Connector, Origin};
use crate::platform;
use crate::probe::{self, ProbeKind};
use crate::request::{RequestContext, redact_url};
use crate::segment::SegmentCheckpoint;

/// Size of each sample range.
const SAMPLE: u64 = 64 * 1024;
/// Number of sample ranges.
const SAMPLES: usize = 3;

/// Why a new link was accepted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefreshEvidence {
    /// Both links report the same strong `ETag` (and the same size).
    SameEtag,
    /// Ranges fetched from the new link equal the bytes already on disk.
    Samples {
        /// How many ranges were compared.
        ranges: usize,
        /// How many bytes were compared.
        bytes: u64,
    },
    /// Nothing was downloaded yet, so there is nothing to compare: the sizes match.
    NothingDownloaded,
}

/// What [`Engine::refresh_link`](crate::Engine::refresh_link) found.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct RefreshOutcome {
    /// The download.
    pub id: crate::DownloadId,
    /// Why the new link was accepted.
    pub evidence: RefreshEvidence,
    /// Where the new link leads after redirects (redacted).
    pub final_url: String,
}

/// What the probe of an accepted link established.
#[derive(Debug)]
pub(crate) struct Accepted {
    pub(crate) evidence: RefreshEvidence,
    pub(crate) final_url: Url,
    pub(crate) validators: Validators,
    pub(crate) content_type: Option<String>,
}

/// The download being refreshed, as far as validation is concerned.
#[derive(Debug)]
pub(crate) struct Target<'a> {
    pub(crate) total: Option<u64>,
    pub(crate) validators: &'a Validators,
    pub(crate) segments: &'a [SegmentCheckpoint],
    pub(crate) partial: Option<&'a Path>,
}

fn reject(why: impl Into<String>) -> Error {
    Error::RefreshRejected(why.into())
}

/// Completed byte ranges `[start, end)` of the checkpoint, merged and sorted.
fn completed(segments: &[SegmentCheckpoint]) -> Vec<(u64, u64)> {
    let mut v: Vec<(u64, u64)> = segments
        .iter()
        .filter(|s| s.done > 0)
        .map(|s| (s.start, s.start + s.done))
        .collect();
    v.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (s, e) in v {
        match out.last_mut() {
            Some(last) if last.1 >= s => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Picks the sample ranges: the end of the completed region, then two at
/// pseudo-random offsets weighted by region length.
fn pick_samples(regions: &[(u64, u64)], rng: &mut fastrand::Rng) -> Vec<(u64, u64)> {
    let Some(&(ls, le)) = regions.last() else {
        return Vec::new();
    };
    let mut picks = vec![(le.saturating_sub(SAMPLE).max(ls), le)];
    let total: u64 = regions.iter().map(|(s, e)| e - s).sum();
    for _ in 1..SAMPLES {
        let mut r = rng.u64(0..total);
        for &(s, e) in regions {
            let len = e - s;
            if r < len {
                let start = (s + r).min(e.saturating_sub(SAMPLE)).max(s);
                picks.push((start, (start + SAMPLE).min(e)));
                break;
            }
            r -= len;
        }
    }
    picks.sort_unstable();
    picks.dedup();
    picks
}

/// Probes `ctx` and checks it against `target`. Returns what the new link
/// looks like, or [`Error::RefreshRejected`] with the reason.
pub(crate) async fn validate(
    connector: &Connector,
    ctx: &RequestContext,
    target: &Target<'_>,
    cancel: &CancellationToken,
) -> Result<Accepted> {
    let probed = probe::probe(connector, ctx, cancel).await?;
    let new_total = match probed.kind {
        ProbeKind::Encoded { .. } => {
            return Err(reject(
                "the server behind the new link compresses the file on the fly, so the \
                 download cannot continue from it",
            ));
        }
        k => k.total(),
    };
    match (target.total, new_total) {
        (Some(want), Some(got)) if want != got => {
            return Err(reject(format!(
                "size differs: the new link has {got} bytes, the download {want}; it is a \
                 different file"
            )));
        }
        (Some(_), None) => {
            return Err(reject(
                "the new link does not say how large the file is, so it cannot be matched \
                 with the download",
            ));
        }
        _ => {}
    }
    let final_url = probed.final_url.clone();
    let validators = probed.validators.clone();
    let content_type = probed.content_type.clone();
    // Not a single byte of the probe is written.
    drop(probed);

    let regions = completed(target.segments);
    let accepted = |evidence| Accepted {
        evidence,
        final_url: final_url.clone(),
        validators: validators.clone(),
        content_type: content_type.clone(),
    };
    if regions.is_empty() {
        return Ok(accepted(RefreshEvidence::NothingDownloaded));
    }
    if let (Some(a), Some(b)) = (target.validators.strong_etag(), validators.strong_etag())
        && a == b
    {
        return Ok(accepted(RefreshEvidence::SameEtag));
    }
    let Some(total) = target.total else {
        return Err(reject(
            "the download's size is unknown, so the new link cannot be checked against it",
        ));
    };
    let partial = target
        .partial
        .ok_or_else(|| reject("the download has no partial file to compare the new link with"))?;
    let file = platform::open_partial(partial).map_err(|e| Error::disk(partial, e))?;
    let origin = Origin::of(ctx.url())?;
    let mut rng = fastrand::Rng::new();
    let picks = pick_samples(&regions, &mut rng);
    let mut compared = 0u64;
    for &(start, end) in &picks {
        let mut resp = tokio::select! {
            () = cancel.cancelled() => return Err(Error::Cancelled),
            r = connector.get(None, &final_url, &origin, ctx.headers(), Some((start, Some(end))), None) => r?,
        };
        if resp.status.as_u16() >= 400 {
            return Err(probe::status_error(
                resp.status,
                &resp.headers,
                &resp.url,
                false,
            ));
        }
        match (resp.status, ContentRange::from_headers(&resp.headers)) {
            (
                StatusCode::PARTIAL_CONTENT,
                Some(ContentRange::Bytes {
                    first, total: t, ..
                }),
            ) if first == start && t.is_none_or(|t| t == total) => {}
            (StatusCode::PARTIAL_CONTENT, other) => {
                return Err(Error::Protocol(format!(
                    "asked for bytes {start}-{}, got Content-Range {other:?}",
                    end - 1
                )));
            }
            _ => {
                return Err(reject(
                    "the server behind the new link does not support resuming (it ignored a \
                     byte range request)",
                ));
            }
        }
        let mut got = Vec::with_capacity((end - start) as usize);
        while (got.len() as u64) < end - start {
            let next = tokio::select! {
                () = cancel.cancelled() => return Err(Error::Cancelled),
                c = resp.chunk(connector.config().stall_timeout) => c?,
            };
            match next {
                Some(b) => got.extend_from_slice(&b),
                None => break,
            }
        }
        got.truncate((end - start) as usize);
        if got.len() as u64 != end - start {
            return Err(Error::Protocol(format!(
                "the new link sent {} of {} sample bytes",
                got.len(),
                end - start
            )));
        }
        let mut have = vec![0u8; got.len()];
        platform::read_exact_at(&file, &mut have, start).map_err(|e| Error::disk(partial, e))?;
        if have != got {
            let at = start + have.iter().zip(&got).take_while(|(a, b)| a == b).count() as u64;
            tracing::debug!(at, url = %redact_url(&final_url), "refresh sample differs");
            return Err(reject(format!(
                "its bytes differ from the ones already downloaded (at offset {at}); it is a \
                 different file"
            )));
        }
        compared += end - start;
    }
    Ok(accepted(RefreshEvidence::Samples {
        ranges: picks.len(),
        bytes: compared,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: u64, end: u64, done: u64) -> SegmentCheckpoint {
        SegmentCheckpoint {
            idx: 0,
            start,
            end,
            done,
        }
    }

    #[test]
    fn completed_regions_merge() {
        let segs = [
            seg(0, 100, 100),
            seg(100, 200, 50),
            seg(200, 300, 0),
            seg(300, 400, 10),
        ];
        assert_eq!(completed(&segs), vec![(0, 150), (300, 310)]);
        assert_eq!(completed(&[seg(0, 10, 0)]).len(), 0);
    }

    #[test]
    fn samples_stay_inside_completed_regions() {
        let regions = vec![
            (0, 1_000_000),
            (2_000_000, 2_010_000),
            (5_000_000, 5_000_100),
        ];
        let mut rng = fastrand::Rng::with_seed(7);
        for _ in 0..200 {
            let picks = pick_samples(&regions, &mut rng);
            assert!(!picks.is_empty() && picks.len() <= SAMPLES);
            // The end of the completed region is always checked.
            assert!(picks.contains(&(5_000_000, 5_000_100)), "{picks:?}");
            for (s, e) in &picks {
                assert!(e > s && e - s <= SAMPLE);
                assert!(
                    regions.iter().any(|(rs, re)| s >= rs && e <= re),
                    "{s}-{e} outside {regions:?}"
                );
            }
        }
        assert_eq!(pick_samples(&[], &mut rng).len(), 0);
    }
}
