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
    let newer = refused(&json!({"v": VERSION + 1, "id": 1, "type": "status", "new_field": true}));
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
    // Every version from the oldest to this one is served.
    for v in OLDEST_VERSION..=VERSION {
        ok(&json!({"v": v, "type": "status"}));
    }
}

fn media(extra: &Value) -> Value {
    let mut base = json!({"v": 2, "type": "media_page", "page_url": "https://v.test/watch?v=1"});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

#[test]
fn a_media_page_with_hints() {
    let env = ok(&media(&json!({
        "title": "A clip",
        "cookies": [{"name": "sid", "value": "s", "domain": ".v.test"}],
        "headers": [["User-Agent", "Mozilla/5.0"], ["Referer", "https://v.test/watch?v=1"]],
        "hints": [
            {"url": "https://cdn.v.test/master.m3u8", "kind": "hls",
             "mime": "application/vnd.apple.mpegurl"},
            {"url": "https://cdn.v.test/a.mpd", "kind": "dash"},
            {"url": "https://cdn.v.test/a.mp4", "kind": "file", "mime": "video/mp4",
             "size": 5_000_000_000_u64}
        ]
    })));
    let Message::MediaPage(m) = env.message else {
        panic!("a media page")
    };
    assert_eq!(m.hints.len(), 3);
    assert_eq!(m.hints[0].kind, MediaKind::Hls);
    assert_eq!(m.hints[2].size, Some(5_000_000_000));
    assert_eq!(m.header("user-agent"), Some("Mozilla/5.0"));
    assert_eq!(m.header("cookie"), None);
    // A hint is fetched as the page's own request: the browser's headers,
    // the page as referrer and the cookies.
    let ctx = m.context(&m.hints[0].url).unwrap();
    assert_eq!(ctx.url().host_str(), Some("cdn.v.test"));
    assert_eq!(ctx.headers()["referer"], "https://v.test/watch?v=1");
    // Without a Referer header the page is the referrer.
    let mut bare = MediaPage::new("https://v.test/p");
    bare.cookies.push(CookieRecord::new("sid", "s", "v.test"));
    let ctx = bare.context("https://cdn.v.test/a.m3u8").unwrap();
    assert_eq!(ctx.headers()["referer"], "https://v.test/p");
}

#[test]
fn media_hints_are_checked() {
    let hint = |url: &str| json!({"url": url, "kind": "file"});
    for bad in [
        json!({"hints": [hint("blob:https://v.test/0f")]}),
        json!({"hints": [hint("file:///C:/a.mp4")]}),
        json!({"hints": [{"url": "https://v.test/a", "kind": "rtmp"}]}),
        json!({"hints": [{"url": "https://v.test/a"}]}),
        json!({"hints": [{"url": "https://v.test/a", "kind": "hls", "fps": 30}]}),
        json!({"hints": [{"url": "https://v.test/a", "kind": "hls", "mime": "vidéo/mp4"}]}),
        json!({"hints": [{"url": "https://v.test/a", "kind": "hls", "size": -1}]}),
        json!({"hints": {"url": "https://v.test/a", "kind": "hls"}}),
    ] {
        assert_eq!(
            reason(&refused(&media(&bad))),
            ErrorReason::Invalid,
            "{bad}"
        );
    }
    let most: Vec<Value> = (0..MAX_HINTS)
        .map(|i| hint(&format!("https://v.test/{i}.mp4")))
        .collect();
    ok(&media(&json!({ "hints": most })));
    let too_many: Vec<Value> = (0..=MAX_HINTS)
        .map(|i| hint(&format!("https://v.test/{i}.mp4")))
        .collect();
    refused(&media(&json!({ "hints": too_many })));
}

#[test]
fn hints_need_protocol_two() {
    let hints = json!({"hints": [{"url": "https://v.test/a.m3u8", "kind": "hls"}]});
    let mut old = media(&hints);
    old["v"] = json!(1);
    let r = refused(&old);
    assert_eq!(reason(&r), ErrorReason::Invalid);
    assert!(r.error.message.contains("hints"), "{}", r.error.message);
    // A version 1 media page without hints is still served, as are version
    // 1 captures (the extension sends those at version 1).
    let mut plain = media(&json!({}));
    plain["v"] = json!(1);
    ok(&plain);
    ok(&media(&hints));
    let Message::MediaPage(m) = ok(&media(&hints)).message else {
        panic!()
    };
    assert!(Message::MediaPage(m.clone()).validate_at(2).is_ok());
    assert!(Message::MediaPage(m).validate_at(1).is_err());
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
        json!({"v": 1, "type": "hello", "app": null, "host": "", "oldest": 1, "current": 2, "rules": null})
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
    let mut page = MediaPage::new("https://v.test/watch");
    let mut hint = MediaHint::new("https://cdn.v.test/a.mp4", MediaKind::File);
    hint.size = Some(1 << 40);
    hint.mime = Some("video/mp4".into());
    page.hints.push(hint);
    page.headers.push(("User-Agent".into(), "UA".into()));
    for message in [
        Message::Capture(c),
        Message::MediaPage(page),
        Message::MediaPage(MediaPage::new("https://v.test/plain")),
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
    let mut page = MediaPage::new("https://u:pw@v.test/w?token=secret");
    page.cookies
        .push(CookieRecord::new("sid", "secret", "v.test"));
    page.hints.push(MediaHint::new(
        "https://cdn.v.test/a.m3u8?sig=secret",
        MediaKind::Hls,
    ));
    let text = format!("{c:?} {:?} {page:?}", c.cookies[0]);
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
fn rule_entries_normalise_as_the_shared_cases_say() {
    let cases: Value = serde_json::from_str(include_str!("rules-cases.json")).unwrap();
    let check = |list: &str, f: fn(&str) -> Option<String>| {
        let entries = cases[list].as_array().unwrap();
        assert!(entries.len() > 10);
        for case in entries {
            let typed = case[0].as_str().unwrap();
            let kept = case[1].as_str();
            assert_eq!(f(typed).as_deref(), kept, "{list}: {typed:?}");
            // What is kept is kept as it is.
            if let Some(k) = kept {
                assert_eq!(f(k).as_deref(), Some(k));
            }
        }
    };
    check("sites", normalise_site);
    check("extensions", normalise_extension);
    let long = cases["sites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c[0].as_str().unwrap())
        .max_by_key(|s| s.len())
        .unwrap();
    assert!(long.len() > MAX_RULE_SITE, "a case over the limit");
}

#[test]
fn rules_are_validated_and_sanitised() {
    let ok = CaptureRules::default();
    ok.validate().unwrap();
    assert_eq!(ok.sanitised(), ok);

    let mut r = CaptureRules {
        never_sites: vec!["https://Bank.example/x".into()],
        ..CaptureRules::default()
    };
    assert_eq!(r.validate().unwrap_err().field, "never_sites");
    r.never_sites = vec!["bank.example".into(), "bank.example".into()];
    assert_eq!(r.validate().unwrap_err().field, "never_sites");
    r.never_sites = vec!["bank.example".into()];
    r.always_sites = (0..=MAX_RULE_SITES)
        .map(|i| format!("s{i}.example"))
        .collect();
    assert_eq!(r.validate().unwrap_err().field, "always_sites");
    r.always_sites.clear();
    r.extensions.push("MP4".into());
    assert_eq!(r.validate().unwrap_err().field, "extensions");
    r.extensions.pop();
    r.min_size = MAX_MIN_SIZE + 1;
    assert_eq!(r.validate().unwrap_err().field, "min_size");
    r.min_size = MAX_MIN_SIZE;
    r.validate().unwrap();

    // Read back from storage: made valid, nothing refused.
    let stored = CaptureRules {
        enabled: false,
        min_size: u64::MAX,
        extensions: (0..300)
            .map(|i| format!(".E{i}"))
            .chain(["bad ext".into()])
            .collect(),
        never_sites: vec!["*.Bank.example".into(), "bank.example".into(), "?".into()],
        always_sites: vec!["https://cdn.example:8443/x".into()],
        private_windows: true,
    };
    let clean = stored.sanitised();
    clean.validate().unwrap();
    assert!(!clean.enabled && clean.private_windows);
    assert_eq!(clean.min_size, MAX_MIN_SIZE);
    assert_eq!(clean.extensions.len(), MAX_RULE_EXTENSIONS);
    assert_eq!(clean.extensions[0], "e0");
    assert_eq!(clean.never_sites, ["bank.example"]);
    assert_eq!(clean.always_sites, ["cdn.example"]);
    assert_eq!(clean.sanitised(), clean);
}

#[test]
fn the_largest_rules_fit_in_a_reply() {
    let site = |i: usize| format!("{}{i:04}.example", "a".repeat(MAX_RULE_SITE - 12));
    let rules = CaptureRules {
        enabled: true,
        min_size: MAX_MIN_SIZE,
        extensions: (0..MAX_RULE_EXTENSIONS)
            .map(|i| format!("{}{i:03}", "x".repeat(MAX_RULE_EXTENSION - 3)))
            .collect(),
        never_sites: (0..MAX_RULE_SITES).map(site).collect(),
        always_sites: (MAX_RULE_SITES..2 * MAX_RULE_SITES).map(site).collect(),
        private_windows: true,
    };
    rules.validate().unwrap();
    let env = ReplyEnvelope::new(1, Some(1), Reply::Hello(HelloReply::new(None, Some(rules))));
    let json: Value = serde_json::from_slice(&env.to_json()).unwrap();
    assert_eq!(json["type"], "hello", "not cut to an error");
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
