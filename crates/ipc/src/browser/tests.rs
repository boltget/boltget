use serde_json::{Value, json};

use super::*;

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

fn ok(v: &Value) -> Envelope {
    decode(&bytes(v)).unwrap_or_else(|e| panic!("{v}: {e:?}"))
}

fn refused(v: &Value) -> Rejection {
    match decode(&bytes(v)) {
        Ok(env) => panic!("accepted {v}: {env:?}"),
        Err(r) => r,
    }
}

fn reason(r: &Rejection) -> ErrorReason {
    r.error.reason
}

#[expect(clippy::needless_pass_by_value, reason = "called with json! literals")]
fn capture(extra: Value) -> Value {
    let mut base = json!({"v": 1, "type": "capture", "url": "https://dl.example.com/a.iso"});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

#[test]
fn a_full_capture() {
    let env = ok(&json!({
        "v": 1,
        "id": 7,
        "type": "capture",
        "url": "https://example.com/get?id=1",
        "final_url": "https://cdn.example.com/a.iso?sig=x",
        "filename": "a.iso",
        "size": 734_003_200_u64,
        "mime": "application/octet-stream",
        "referrer": "https://example.com/downloads",
        "page_url": "https://example.com/downloads",
        "incognito": false,
        "forced": true,
        "cookies": [{
            "name": "sid", "value": "abc", "domain": ".example.com", "path": "/",
            "host_only": false, "secure": true, "http_only": true,
            "same_site": "lax", "expires": 4_102_444_800.5
        }],
        "headers": [["User-Agent", "Mozilla/5.0"], ["Accept-Language", "en"]]
    }));
    assert_eq!(env.v, 1);
    assert_eq!(env.id, Some(7));
    let Message::Capture(c) = env.message else {
        panic!("a capture")
    };
    assert!(c.forced);
    assert_eq!(c.size, Some(734_003_200));
    assert_eq!(c.cookies[0].same_site, Some(SameSite::Lax));
    // The engine's request goes to the final URL with the browser's headers,
    // its referrer and its cookies.
    let ctx = c.context().unwrap();
    assert_eq!(ctx.url().host_str(), Some("cdn.example.com"));
    assert_eq!(ctx.suggested_name(), Some("a.iso"));
    assert_eq!(ctx.headers()["user-agent"], "Mozilla/5.0");
    assert_eq!(ctx.headers()["referer"], "https://example.com/downloads");
}

#[test]
fn the_smallest_messages() {
    assert!(matches!(
        ok(&json!({"v": 1, "type": "status"})).message,
        Message::Status(_)
    ));
    let hello = ok(&json!({"v": 1, "type": "hello", "extension": "0.3.0", "browser": "edge"}));
    assert_eq!(
        hello.message,
        Message::Hello(Hello {
            extension: "0.3.0".into(),
            browser: Browser::Edge
        })
    );
    let Message::Capture(c) = ok(&capture(json!({}))).message else {
        panic!()
    };
    assert_eq!(c, Capture::new("https://dl.example.com/a.iso"));
    let links = ok(&json!({"v": 1, "type": "add_links", "links": [{"url": "http://x.test/a"}]}));
    assert!(matches!(links.message, Message::AddLinks(_)));
    let media = ok(&json!({"v": 1, "type": "media_page", "page_url": "https://v.test/watch"}));
    assert!(matches!(media.message, Message::MediaPage(_)));
}

#[test]
fn the_envelope() {
    // Missing or wrong version, wrong id, not an object.
    for (v, id) in [
        (json!({"type": "status"}), None),
        (json!({"v": "1", "type": "status"}), None),
        (json!({"v": 1.5, "type": "status", "id": 3}), Some(3)),
        (json!({"v": 1, "type": "status", "id": -1}), None),
        (
            json!({"v": 1, "type": "status", "id": 4_294_967_296_u64}),
            None,
        ),
        (json!({"v": 1, "type": "status", "id": "a"}), None),
        (json!([1, 2]), None),
        (json!("status"), None),
    ] {
        let r = refused(&v);
        assert_eq!(reason(&r), ErrorReason::Invalid, "{v}");
        assert_eq!(r.id, id, "{v}");
    }
    assert_eq!(
        ok(&json!({"v": 1, "type": "status", "id": null})).id,
        None,
        "a null id is no id"
    );
    for junk in [&b""[..], b"{", b"\xff\xfe", b"null", b"{\"v\":1}"] {
        assert_eq!(reason(&decode(junk).unwrap_err()), ErrorReason::Invalid);
    }
}

#[test]
fn versions() {
    let newer = refused(&json!({"v": 2, "id": 1, "type": "status", "new_field": true}));
    assert_eq!(
        reason(&newer),
        ErrorReason::AppTooOld,
        "checked before the shape"
    );
    assert_eq!(newer.id, Some(1));
    assert_eq!(newer.v, VERSION);
    let huge = refused(&json!({"v": 99_999_999_999_u64, "type": "status"}));
    assert_eq!(reason(&huge), ErrorReason::AppTooOld);
    let older = refused(&json!({"v": 0, "type": "status"}));
    assert_eq!(reason(&older), ErrorReason::ExtensionTooOld);
}

#[test]
fn shapes_are_strict() {
    for v in [
        json!({"v": 1, "type": "explode"}),
        json!({"v": 1, "type": "status", "x": 1}),
        json!({"v": 1, "type": "hello", "extension": "1", "browser": "netscape"}),
        json!({"v": 1, "type": "hello", "extension": "1", "browser": "chrome", "os": "x"}),
        capture(json!({"priority": 1})),
        capture(
            json!({"cookies": [{"name": "a", "value": "b", "domain": "x.test", "partition": 1}]}),
        ),
        capture(json!({"cookies": [{"name": "a", "value": "b"}]})),
        capture(
            json!({"cookies": [{"name": "a", "value": "b", "domain": "x.test", "same_site": "none"}]}),
        ),
        capture(json!({"headers": [["Referer"]]})),
        capture(json!({"headers": {"Referer": "x"}})),
        capture(json!({"size": -1})),
        capture(json!({"url": null})),
        json!({"v": 1, "type": "add_links", "links": ["http://x.test/a"]}),
    ] {
        assert_eq!(reason(&refused(&v)), ErrorReason::Invalid, "{v}");
    }
}

#[test]
fn links_are_http_or_https() {
    for url in [
        "file:///C:/a.iso",
        "ftp://x.test/a",
        "blob:https://x.test/0f0e",
        "data:text/plain,hi",
        "javascript:alert(1)",
        "chrome://downloads",
        "https://",
        "https:// x.test/a",
        "https://x.test/a\nb",
        "",
        "x.test/a",
    ] {
        let r = refused(&capture(json!({"url": url})));
        assert_eq!(reason(&r), ErrorReason::Invalid, "{url:?}");
        refused(&capture(json!({"referrer": url})));
        refused(&capture(json!({"final_url": url})));
    }
    ok(&capture(json!({"url": "HTTPS://X.TEST/A"})));
    let long = format!("https://x.test/{}", "a".repeat(MAX_URL));
    refused(&capture(json!({"url": long})));
    let fits = format!("https://x.test/{}", "a".repeat(MAX_URL - 20));
    ok(&capture(json!({"url": fits})));
}

#[test]
fn headers_on_the_allowlist_only() {
    ok(&capture(json!({"headers": [
        ["user-agent", "a"], ["Referer", "https://x.test/"], ["ACCEPT", "*/*"],
        ["accept-language", "en"], ["Origin", "https://x.test"], ["Authorization", "Bearer t"]
    ]})));
    for h in [
        json!([["Cookie", "a=b"]]),
        json!([["Host", "evil.test"]]),
        json!([["Range", "bytes=0-"]]),
        json!([["X-Custom", "1"]]),
        json!([["Accept-Encoding", "gzip"]]),
        json!([["Referer", "a"], ["referer", "b"]]),
        json!([["User-Agent", "a\r\nX-Injected: 1"]]),
        json!([["User-Agent", "a".repeat(MAX_HEADER_VALUE + 1)]]),
    ] {
        refused(&capture(json!({ "headers": h })));
    }
}

#[test]
fn cookie_limits() {
    let cookie = |name: &str, value: &str, domain: &str| json!({"name": name, "value": value, "domain": domain});
    ok(&capture(
        json!({"cookies": [cookie("", "v", "x.test"), cookie("n", "", "[::1]")]}),
    ));
    for bad in [
        cookie("", "", "x.test"),
        cookie("a;b", "v", "x.test"),
        cookie("a=b", "v", "x.test"),
        cookie("a", "v;w", "x.test"),
        cookie("a", "v\r\n", "x.test"),
        cookie("a", "v", ""),
        cookie("a", "v", "."),
        cookie("a", "v", "x test"),
        cookie("a", "v", "x.test/"),
        json!({"name": "a", "value": "v", "domain": "x.test", "path": "a"}),
        json!({"name": "a", "value": "v", "domain": "x.test", "expires": -1}),
        json!({"name": "a", "value": "v", "domain": "x.test", "expires": 1e300}),
    ] {
        refused(&capture(json!({ "cookies": [bad] })));
    }
    let many: Vec<Value> = (0..=MAX_COOKIES)
        .map(|i| cookie(&format!("c{i}"), "v", "x.test"))
        .collect();
    refused(&capture(json!({ "cookies": many })));
    let big = "v".repeat(MAX_COOKIE_BYTES / 2);
    refused(&capture(
        json!({"cookies": [cookie("a", &big, "x.test"), cookie("b", &big, "x.test")]}),
    ));
}

#[test]
fn other_fields() {
    for bad in [
        json!({"filename": "../a.iso"}),
        json!({"filename": "a\\b.iso"}),
        json!({"filename": "a\u{0}.iso"}),
        json!({"filename": "a\n.iso"}),
        json!({"filename": "  "}),
        json!({"filename": "a".repeat(MAX_TEXT + 1)}),
        json!({"mime": "text/plain\u{7f}"}),
        json!({"mime": "téxt/plain"}),
        json!({"mime": "a".repeat(MAX_SHORT + 1)}),
    ] {
        refused(&capture(bad));
    }
    ok(&capture(
        json!({"filename": "Résumé (final).pdf", "mime": "application/pdf"}),
    ));
    refused(&json!({"v": 1, "type": "hello", "extension": "1\n", "browser": "chrome"}));
    refused(&json!({"v": 1, "type": "add_links", "links": []}));
    refused(&json!({"v": 1, "type": "add_links", "links": [{"url": "mailto:a@x.test"}]}));
    let many: Vec<Value> = (0..=MAX_LINKS)
        .map(|i| json!({"url": format!("https://x.test/{i}")}))
        .collect();
    refused(&json!({"v": 1, "type": "add_links", "links": many}));
    refused(&json!({"v": 1, "type": "media_page", "page_url": "about:blank"}));
}

#[test]
fn the_size_limit() {
    let mut big = bytes(&json!({"v": 1, "type": "status"}));
    big.resize(MAX_MESSAGE + 1, b' ');
    assert_eq!(reason(&decode(&big).unwrap_err()), ErrorReason::TooLarge);
    let mut fits = bytes(&json!({"v": 1, "type": "status"}));
    fits.resize(MAX_MESSAGE, b' ');
    decode(&fits).unwrap();
}

#[test]
fn deep_nesting_is_refused_not_a_crash() {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert_eq!(
        reason(&decode(deep.as_bytes()).unwrap_err()),
        ErrorReason::Invalid
    );
}

#[test]
fn replies_as_the_extension_reads_them() {
    let cases = [
        (
            Reply::Accepted(Accepted::added(12)),
            json!({"v": 1, "id": 3, "type": "accepted", "download": 12, "confirm": false}),
        ),
        (
            Reply::rejected(RejectReason::LinkRefused, "403"),
            json!({"v": 1, "id": 3, "type": "rejected", "reason": "link_refused", "message": "403"}),
        ),
        (
            Reply::unavailable(UnavailableReason::NotRunning, "x"),
            json!({"v": 1, "id": 3, "type": "unavailable", "reason": "not_running", "message": "x"}),
        ),
        (
            Reply::error(ErrorReason::AppTooOld, "x"),
            json!({"v": 1, "id": 3, "type": "error", "reason": "app_too_old", "message": "x"}),
        ),
        (
            Reply::Status(StatusReply::new(2, 5, 1000)),
            json!({"v": 1, "id": 3, "type": "status", "active": 2, "queued": 5, "speed": 1000}),
        ),
    ];
    for (reply, expected) in cases {
        let env = ReplyEnvelope::new(1, Some(3), reply);
        let json: Value = serde_json::from_slice(&env.to_json()).unwrap();
        assert_eq!(json, expected);
        assert_eq!(serde_json::from_value::<ReplyEnvelope>(json).unwrap(), env);
    }
    let hello = ReplyEnvelope::new(1, None, Reply::Hello(HelloReply::new(None, None)));
    let json: Value = serde_json::from_slice(&hello.to_json()).unwrap();
    assert_eq!(
        json,
        json!({"v": 1, "type": "hello", "app": null, "host": "", "oldest": 1, "current": 1, "rules": null})
    );
}

#[test]
fn replies_never_exceed_the_browser_limit() {
    let rules = CaptureRules {
        never_sites: vec!["x".repeat(1024); 2048],
        ..CaptureRules::default()
    };
    let env = ReplyEnvelope::new(1, Some(1), Reply::Hello(HelloReply::new(None, Some(rules))));
    let out = env.to_json();
    assert!(out.len() <= MAX_REPLY);
    let json: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["type"], "error");
    assert_eq!(json["id"], 1);
}

#[test]
fn envelopes_round_trip_through_decode() {
    let mut c = Capture::new("https://x.test/a.zip");
    c.cookies.push(CookieRecord::new("sid", "1", "x.test"));
    c.headers.push(("Referer".into(), "https://x.test/".into()));
    for message in [
        Message::Capture(c),
        Message::Status(Status {}),
        Message::Hello(Hello {
            extension: "0.3.0".into(),
            browser: Browser::Firefox,
        }),
    ] {
        let env = Envelope {
            v: VERSION,
            id: Some(9),
            message,
        };
        assert_eq!(decode(&serde_json::to_vec(&env).unwrap()).unwrap(), env);
    }
}

#[test]
fn debug_output_hides_secrets() {
    let mut c = Capture::new("https://u:pw@x.test/a.iso?token=secret");
    c.cookies.push(CookieRecord::new("sid", "secret", "x.test"));
    c.headers
        .push(("Authorization".into(), "Bearer secret".into()));
    let text = format!("{c:?} {:?}", c.cookies[0]);
    assert!(!text.contains("secret") && !text.contains("pw"), "{text}");
    assert!(text.contains("Authorization"), "{text}");
}

#[test]
fn cookies_reach_the_engine() {
    let mut c = Capture::new("https://dl.x.test/a.iso");
    let mut expired = CookieRecord::new("old", "1", "x.test");
    expired.expires = Some(1.0);
    c.cookies.push(expired);
    c.cookies.push(CookieRecord::new("sid", "1", ".x.test"));
    // Validated input never fails to become a context.
    c.context().unwrap();
    let links = AddLinks {
        links: vec![Link::new("https://x.test/a")],
        page_url: Some("https://x.test/".into()),
        incognito: false,
        cookies: c.cookies.clone(),
        headers: Vec::new(),
    };
    let ctx = links.context("https://x.test/a").unwrap();
    assert_eq!(ctx.headers()["referer"], "https://x.test/");
}

#[test]
fn default_rules_follow_the_docs() {
    let r = CaptureRules::default();
    assert!(r.enabled && !r.private_windows);
    assert_eq!(r.min_size, 1 << 20);
    for ext in ["zip", "iso", "exe", "mp4", "flac", "bin"] {
        assert!(r.extensions.iter().any(|e| e == ext), "{ext}");
    }
    for ext in ["pdf", "docx", "torrent"] {
        assert!(!r.extensions.iter().any(|e| e == ext), "{ext}");
    }
}

#[test]
fn the_fuzz_seed_corpus_passes() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/native_message");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        fuzz(&std::fs::read(entry.unwrap().path()).unwrap());
        n += 1;
    }
    assert!(n > 0, "no seeds in {}", dir.display());
    let manifest = std::fs::read_to_string(dir.join("../../Cargo.toml")).unwrap();
    assert!(manifest.contains("name = \"native_message\""));
}

#[test]
fn random_and_mutated_inputs() {
    // A small deterministic mutation run, so the target is exercised by a
    // plain `cargo test`; libFuzzer runs it properly every night.
    let seeds: Vec<Vec<u8>> = vec![
        bytes(&capture(
            json!({"cookies": [{"name": "a", "value": "b", "domain": "x.test"}]}),
        )),
        bytes(&json!({"v": 1, "type": "add_links", "links": [{"url": "https://x.test/a"}]})),
    ];
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..4000 {
        let mut data = seeds[(next() % 2) as usize].clone();
        for _ in 0..=(next() % 4) {
            let i = (next() as usize) % data.len();
            match next() % 3 {
                0 => data[i] = next() as u8,
                1 => {
                    data.remove(i);
                }
                _ => data.insert(i, b"{}[]\",:0\\"[(next() % 9) as usize]),
            }
            if data.is_empty() {
                data.push(b'{');
            }
        }
        fuzz(&data);
    }
}
