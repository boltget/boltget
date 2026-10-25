//! Probing: one `GET` with `Range: bytes=0-` whose connection becomes worker 0.
//!
//! See `docs/ENGINE.md`, "Probing". `HEAD` is never used.

use std::time::Duration;

use http::StatusCode;
use http::header::{self, HeaderMap};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::backoff;
use crate::error::{Error, Result};
use crate::headers::{self, ContentRange, Validators};
use crate::http::{Connector, Origin, Response};
use crate::request::{RequestContext, redact_url};

/// Whether the server honours byte ranges for this resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeSupport {
    /// Confirmed by a `206` response.
    Yes,
    /// The server ignored ranges (or encodes the body): one connection, restart on failure.
    No,
}

/// What the probe found out about a resource.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct ProbeResult {
    /// The URL after redirects (query string kept; redact before logging).
    #[serde(serialize_with = "ser_redacted")]
    pub final_url: Url,
    /// Number of redirects followed.
    pub redirects: usize,
    /// Total size, if known.
    pub size: Option<u64>,
    /// Range support.
    pub ranges: RangeSupport,
    /// Validators recorded for later range requests.
    pub validators: Validators,
    /// `Content-Type`, if any.
    pub content_type: Option<String>,
    /// The resolved, sanitized file name.
    pub filename: String,
}

fn ser_redacted<S: serde::Serializer>(u: &Url, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&redact_url(u))
}

impl ProbeResult {
    /// Whether an interrupted download can continue from where it stopped.
    pub fn resumable(&self) -> bool {
        self.ranges == RangeSupport::Yes
    }
}

/// The probe interpretation table from `docs/ENGINE.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeKind {
    /// `206` with `Content-Range: bytes 0-(n-1)/n`.
    Ranged { total: u64 },
    /// `206` with `Content-Range: bytes 0-x/*`: resumable, cannot split.
    RangedUnknownSize,
    /// `200` with `Content-Length: n`. Needs a verification request.
    Full { total: u64 },
    /// `200` without `Content-Length` (chunked or close-delimited).
    Stream,
    /// `Content-Encoding` other than identity: ranges are unsafe.
    Encoded { total: Option<u64> },
    /// `416` with `Content-Range: bytes */0`.
    Empty,
}

impl ProbeKind {
    pub(crate) fn total(self) -> Option<u64> {
        match self {
            ProbeKind::Ranged { total } | ProbeKind::Full { total } => Some(total),
            ProbeKind::Encoded { total } => total,
            ProbeKind::Empty => Some(0),
            ProbeKind::RangedUnknownSize | ProbeKind::Stream => None,
        }
    }
}

/// Interprets the probe response (after redirects).
pub(crate) fn interpret(status: StatusCode, headers: &HeaderMap, url: &Url) -> Result<ProbeKind> {
    let encoded = headers::is_encoded(headers);
    match status.as_u16() {
        206 => {
            if encoded {
                return Ok(ProbeKind::Encoded { total: None });
            }
            match ContentRange::from_headers(headers) {
                Some(ContentRange::Bytes {
                    first: 0,
                    total: Some(total),
                    ..
                }) => Ok(ProbeKind::Ranged { total }),
                Some(ContentRange::Bytes {
                    first: 0,
                    total: None,
                    ..
                }) => Ok(ProbeKind::RangedUnknownSize),
                other => Err(Error::Protocol(format!(
                    "206 with unusable Content-Range {other:?}"
                ))),
            }
        }
        200 => {
            let len = headers::content_length(headers);
            if encoded {
                // The length is that of the encoded body; still useful for progress.
                return Ok(ProbeKind::Encoded { total: len });
            }
            Ok(match len {
                Some(total) => ProbeKind::Full { total },
                None => ProbeKind::Stream,
            })
        }
        416 => match ContentRange::from_headers(headers) {
            Some(ContentRange::Unsatisfied { total: 0 }) => Ok(ProbeKind::Empty),
            _ => Err(status_error(status, headers, url, false)),
        },
        _ => Err(status_error(status, headers, url, false)),
    }
}

pub(crate) fn status_error(
    status: StatusCode,
    headers: &HeaderMap,
    url: &Url,
    previously_worked: bool,
) -> Error {
    Error::Status {
        status: status.as_u16(),
        url: redact_url(url),
        retry_after: headers::retry_after_from(headers),
        previously_worked,
    }
}

/// A completed probe whose body is still open (it becomes worker 0).
#[derive(Debug)]
pub(crate) struct Probed {
    pub(crate) response: Response,
    pub(crate) kind: ProbeKind,
    pub(crate) validators: Validators,
    pub(crate) content_type: Option<String>,
    pub(crate) content_disposition: Option<Vec<u8>>,
    pub(crate) final_url: Url,
    pub(crate) redirects: usize,
}

/// Sleeps for `d` unless cancelled.
pub(crate) async fn sleep_or_cancel(d: Duration, cancel: &CancellationToken) -> Result<()> {
    tokio::select! {
        () = cancel.cancelled() => Err(Error::Cancelled),
        () = tokio::time::sleep(d) => Ok(()),
    }
}

/// The retry delay after `err` on `attempt` (0-based): `Retry-After` wins.
/// `None` means the server asked us to wait longer than `retry_after_max`: give up.
pub(crate) fn retry_delay(err: &Error, connector: &Connector, attempt: u32) -> Option<Duration> {
    let cfg = connector.config();
    match err.retry_after() {
        Some(ra) if ra > cfg.retry_after_max => None,
        Some(ra) => Some(ra),
        None => Some(backoff::delay(cfg.backoff_base, cfg.backoff_max, attempt)),
    }
}

/// Sends the probe request, retrying transient failures with backoff.
pub(crate) async fn probe(
    connector: &Connector,
    ctx: &RequestContext,
    cancel: &CancellationToken,
) -> Result<Probed> {
    let origin = Origin::of(ctx.url())?;
    let mut attempt = 0u32;
    loop {
        let res = tokio::select! {
            () = cancel.cancelled() => return Err(Error::Cancelled),
            r = probe_once(connector, ctx, &origin) => r,
        };
        match res {
            Ok(p) => return Ok(p),
            Err(e) if e.is_retryable() && attempt + 1 < connector.config().max_attempts => {
                let Some(d) = retry_delay(&e, connector, attempt) else {
                    return Err(e);
                };
                tracing::debug!(error = %e, delay = ?d, attempt, "probe failed; retrying");
                attempt += 1;
                sleep_or_cancel(d, cancel).await?;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn probe_once(
    connector: &Connector,
    ctx: &RequestContext,
    origin: &Origin,
) -> Result<Probed> {
    let response = connector
        .get(None, ctx.url(), origin, ctx, Some((0, None)), None)
        .await?;
    let kind = interpret(response.status, &response.headers, &response.url)?;
    let validators = Validators::from_headers(&response.headers);
    let content_type = response
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let content_disposition = response
        .headers
        .get(header::CONTENT_DISPOSITION)
        .map(|v| v.as_bytes().to_vec());
    let final_url = response.url.clone();
    let redirects = response.redirects.len();
    Ok(Probed {
        response,
        kind,
        validators,
        content_type,
        content_disposition,
        final_url,
        redirects,
    })
}

/// The verification request for a `200` probe: `Range: bytes=m-` on a second
/// connection. Returns the open `206` response (it becomes a worker) if the
/// server honours ranges with a consistent `Content-Range`.
pub(crate) async fn verify_ranges(
    connector: &Connector,
    url: &Url,
    original: &Origin,
    ctx: &RequestContext,
    m: u64,
    total: u64,
    validators: &Validators,
) -> Option<Response> {
    let resp = connector
        .get(
            None,
            url,
            original,
            ctx,
            Some((m, None)),
            validators.if_range(),
        )
        .await
        .ok()?;
    if resp.status != StatusCode::PARTIAL_CONTENT || headers::is_encoded(&resp.headers) {
        return None;
    }
    match ContentRange::from_headers(&resp.headers) {
        Some(ContentRange::Bytes {
            first,
            total: Some(t),
            ..
        }) if first == m && t == total => {
            let later = Validators::from_headers(&resp.headers);
            validators.mismatch(&later).is_none().then_some(resp)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn h(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(*k, HeaderValue::from_static(v));
        }
        m
    }

    fn url() -> Url {
        Url::parse("https://example.com/f").unwrap()
    }

    #[test]
    fn interpretation_table() {
        let s = |c| StatusCode::from_u16(c).unwrap();
        assert_eq!(
            interpret(s(206), &h(&[("content-range", "bytes 0-99/100")]), &url()).unwrap(),
            ProbeKind::Ranged { total: 100 }
        );
        assert_eq!(
            interpret(s(206), &h(&[("content-range", "bytes 0-99/*")]), &url()).unwrap(),
            ProbeKind::RangedUnknownSize
        );
        assert!(interpret(s(206), &h(&[("content-range", "bytes 5-99/100")]), &url()).is_err());
        assert!(interpret(s(206), &h(&[]), &url()).is_err());
        assert_eq!(
            interpret(
                s(200),
                &h(&[("content-length", "100"), ("accept-ranges", "bytes")]),
                &url()
            )
            .unwrap(),
            ProbeKind::Full { total: 100 }
        );
        assert_eq!(
            interpret(s(200), &h(&[("content-length", "100")]), &url()).unwrap(),
            ProbeKind::Full { total: 100 }
        );
        assert_eq!(
            interpret(s(200), &h(&[]), &url()).unwrap(),
            ProbeKind::Stream
        );
        assert_eq!(
            interpret(
                s(200),
                &h(&[("content-length", "10"), ("content-encoding", "gzip")]),
                &url()
            )
            .unwrap(),
            ProbeKind::Encoded { total: Some(10) }
        );
        assert_eq!(
            interpret(s(416), &h(&[("content-range", "bytes */0")]), &url()).unwrap(),
            ProbeKind::Empty
        );
        let e = interpret(s(416), &h(&[("content-range", "bytes */5")]), &url()).unwrap_err();
        assert_eq!(e.kind(), crate::ErrorKind::Permanent);
        let e = interpret(s(404), &h(&[]), &url()).unwrap_err();
        assert_eq!(e.kind(), crate::ErrorKind::Permanent);
        let e = interpret(s(503), &h(&[("retry-after", "7")]), &url()).unwrap_err();
        assert_eq!(e.kind(), crate::ErrorKind::Throttled);
        assert_eq!(e.retry_after(), Some(Duration::from_secs(7)));
        let e = interpret(s(403), &h(&[]), &url()).unwrap_err();
        assert_eq!(e.kind(), crate::ErrorKind::Permanent);
    }
}
