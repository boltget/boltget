//! Probing pages with a fake yt-dlp, and the presets computed from realistic
//! info JSON (`tests/fixtures/info/`).
#![allow(clippy::unwrap_used)] // Test helpers outside #[test] functions.

mod support;

use std::path::{Path, PathBuf};

use boltget_engine::{Cookie, CookieJar};
use boltget_media::{
    ContainerPreference, Error, ItemRange, MediaInfo, PlaylistMode, Preset, ProbeOptions, Prober,
    ResolvedPreset, SizeEstimate, playlist, presets,
};

fn info(name: &str) -> MediaInfo {
    let bytes = std::fs::read(support::fixture("info").join(name)).unwrap();
    MediaInfo::from_json(&bytes).unwrap()
}

/// A fake yt-dlp in a fresh directory, and a prober for it.
fn fake() -> (tempfile::TempDir, PathBuf, Prober) {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join(support::exe("yt-dlp"));
    std::fs::write(&exe, support::fake_exe()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prober = Prober::new(
        exe,
        Some(tmp.path().to_path_buf()),
        tmp.path().join(".jobs"),
    );
    let dir = tmp.path().to_path_buf();
    (tmp, dir, prober)
}

fn summary(p: &[ResolvedPreset]) -> Vec<(Preset, Vec<&str>, Option<u32>, &str)> {
    p.iter()
        .map(|r| {
            (
                r.preset,
                r.format_ids.iter().map(String::as_str).collect(),
                r.height,
                r.container.as_str(),
            )
        })
        .collect()
}

fn exact(bytes: u64) -> SizeEstimate {
    SizeEstimate {
        bytes,
        approximate: false,
    }
}

fn about(bytes: u64) -> SizeEstimate {
    SizeEstimate {
        bytes,
        approximate: true,
    }
}

#[tokio::test]
async fn a_probe_runs_yt_dlp_and_parses_its_answer() {
    let (_tmp, dir, prober) = fake();
    std::fs::copy(
        support::fixture("info").join("youtube_like.json"),
        dir.join("fake-info.json"),
    )
    .unwrap();
    let mut jar = CookieJar::new();
    jar.insert(
        Cookie::from_browser("SID", "secret", ".youtube.com", false, "/", true, None)
            .with_http_only(true),
    );
    let opts = ProbeOptions {
        cookies: Some(jar),
        user_agent: Some("Mozilla/5.0".into()),
        ..ProbeOptions::default()
    };
    let info = prober
        .probe("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &opts)
        .await
        .unwrap();
    assert_eq!(info.title.as_deref(), Some("A Video About Downloading"));
    assert_eq!(info.duration, Some(212.0));

    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(
        &args[..5],
        [
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-update",
            "--ffmpeg-location",
            dir.to_str().unwrap()
        ]
    );
    for expected in [
        "-J",
        "--no-playlist",
        "--flat-playlist",
        "--ignore-no-formats-error",
    ] {
        assert!(args.contains(&expected), "{expected} missing from {args:?}");
    }
    assert!(args.contains(&"User-Agent:Mozilla/5.0"));
    assert_eq!(
        &args[args.len() - 2..],
        ["--", "https://www.youtube.com/watch?v=dQw4w9WgXcQ"]
    );
    // yt-dlp got the cookies as a file, which is gone now.
    let cookie_path = args[args.iter().position(|a| *a == "--cookies").unwrap() + 1];
    assert!(Path::new(cookie_path).starts_with(dir.join(".jobs")));
    assert!(!Path::new(cookie_path).exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("fake-cookies.txt")).unwrap(),
        "# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tsecret\n"
    );
}

#[tokio::test]
async fn a_failed_probe_keeps_yt_dlps_message_and_deletes_the_cookies() {
    let (_tmp, dir, prober) = fake();
    std::fs::write(
        dir.join("fake-probe-error.txt"),
        "WARNING: [generic] Falling back on generic information extractor\n\
         ERROR: Unsupported URL: https://example.com/\n",
    )
    .unwrap();
    let mut jar = CookieJar::new();
    jar.insert(Cookie::for_host("example.com", "a", "1"));
    let opts = ProbeOptions {
        cookies: Some(jar),
        playlist: PlaylistMode::Entries,
        ..ProbeOptions::default()
    };
    let err = prober
        .probe("https://example.com/", &opts)
        .await
        .unwrap_err();
    match &err {
        Error::Extraction { category, message } => {
            assert_eq!(message, "Unsupported URL: https://example.com/");
            // No video or audio there, not a broken extractor.
            assert_eq!(*category, boltget_media::ErrorCategory::NoMedia);
        }
        other => panic!("{other}"),
    }
    assert_eq!(err.message_key(), "error.media.no_media");
    assert!(dir.join("fake-cookies.txt").exists());
    assert_eq!(std::fs::read_dir(dir.join(".jobs")).unwrap().count(), 0);
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    assert!(args.lines().any(|a| a == "--yes-playlist"));
}

#[tokio::test]
async fn output_that_is_not_info_json_is_an_error() {
    let (_tmp, dir, prober) = fake();
    std::fs::write(dir.join("fake-info.json"), "not json").unwrap();
    let err = prober
        .probe("https://example.com/v", &ProbeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InfoJson { .. }), "{err}");
    // Without cookies, no cookie file and no --cookies.
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    assert!(!args.lines().any(|a| a == "--cookies"));
}

#[tokio::test]
async fn refused_options_never_start_yt_dlp() {
    let (_tmp, dir, prober) = fake();
    let opts = ProbeOptions {
        referer: Some("https://a.test/\nX: y".into()),
        ..ProbeOptions::default()
    };
    let err = prober.probe("https://a.test/", &opts).await.unwrap_err();
    assert!(matches!(err, Error::Argument { .. }), "{err}");
    let err = prober
        .probe("file:///C:/Windows/win.ini", &ProbeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Argument { .. }), "{err}");
    assert!(!dir.join("fake-args.txt").exists());
}

#[test]
fn youtube_like_page_prefer_mp4() {
    let page = info("youtube_like.json");
    assert_eq!(page.publisher(), Some("Example Channel"));
    assert_eq!(page.max_height(), Some(2160));
    assert_eq!(
        page.best_thumbnail(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg")
    );
    assert!(!page.is_playlist() && !page.drm_only());
    let p = presets(&page, ContainerPreference::Mp4);
    // 1080p is Best again (the best MP4 video is 1080p), so it is collapsed.
    assert_eq!(
        summary(&p),
        [
            (Preset::Best, vec!["399", "140"], Some(1080), "mp4"),
            (Preset::P720, vec!["398", "140"], Some(720), "mp4"),
            (Preset::Small, vec!["135", "140"], Some(480), "mp4"),
            (Preset::Audio, vec!["140"], None, "m4a"),
        ]
    );
    assert_eq!(p[0].size, Some(exact(27_703_020 + 3_437_753)));
    assert_eq!(p[0].video_codec.as_deref(), Some("av01.0.08M.08"));
    assert_eq!(p[0].audio_codec.as_deref(), Some("mp4a.40.2"));
    assert_eq!((p[0].width, p[0].fps), (Some(1920), Some(25.0)));
    assert_eq!(p[0].dynamic_range, None, "SDR is not worth a label");
    assert_eq!(
        p[0].expression,
        "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b"
    );
    assert!(p[3].audio_only && p[3].video_codec.is_none());

    let langs = page.subtitle_languages();
    let codes: Vec<(&str, bool)> = langs
        .iter()
        .map(|l| (l.code.as_str(), l.automatic))
        .collect();
    assert_eq!(
        codes,
        [
            ("en", false),
            ("pt-BR", false),
            ("de", true),
            ("en-orig", true)
        ]
    );
    assert_eq!(langs[1].name.as_deref(), Some("Portuguese (Brazil)"));
}

#[test]
fn youtube_like_page_prefer_original() {
    let p = presets(&info("youtube_like.json"), ContainerPreference::Original);
    assert_eq!(
        summary(&p),
        [
            (Preset::Best, vec!["313", "251"], Some(2160), "webm"),
            (Preset::P1080, vec!["399", "251"], Some(1080), "mkv"),
            (Preset::P720, vec!["398", "251"], Some(720), "mkv"),
            (Preset::Small, vec!["244", "251"], Some(480), "webm"),
            (Preset::Audio, vec!["251"], None, "opus"),
        ]
    );
    // The 2160p stream's size is the site's estimate.
    assert_eq!(p[0].size, Some(about(250_160_003 + 3_600_402)));
    assert_eq!(p[0].expression, "bv*+ba/b");
}

#[test]
fn progressive_only_page() {
    let page = info("progressive_only.json");
    let p = presets(&page, ContainerPreference::Mp4);
    // No 1080p (the page tops out at 720p, which is Best); audio comes out of
    // the best combined file.
    assert_eq!(
        summary(&p),
        [
            (Preset::Best, vec!["720p"], Some(720), "mp4"),
            (Preset::Small, vec!["360p"], Some(360), "mp4"),
            (Preset::Audio, vec!["720p"], None, "m4a"),
        ]
    );
    assert_eq!(p[0].size, Some(exact(24_688_012)));
    assert_eq!(p[0].fps, Some(30.0));
    assert_eq!(page.best_thumbnail(), page.thumbnail.as_deref());
    assert_eq!(page.subtitle_languages().len(), 0);
}

#[test]
fn hls_only_page_estimates_sizes_from_bit_rates() {
    let page = info("hls_only.json");
    assert_eq!(page.publisher(), None);
    let p = presets(&page, ContainerPreference::Original);
    assert_eq!(
        summary(&p),
        [
            (Preset::Best, vec!["hls-1080p"], Some(1080), "mp4"),
            (Preset::P720, vec!["hls-480p"], Some(480), "mp4"),
            (Preset::Audio, vec!["hls-1080p"], None, "mp4"),
        ]
    );
    // Codecs unknown count as present, as in yt-dlp.
    assert_eq!(p[0].video_codec, None);
    // 4800 kbit/s for 120 s.
    assert_eq!(p[0].size, Some(about(72_000_000)));
    assert_eq!(p[1].size, Some(about(18_007_500)));
}

#[test]
fn audio_only_page() {
    let page = info("audio_only.json");
    assert_eq!(page.max_height(), None);
    let mp4 = presets(&page, ContainerPreference::Mp4);
    assert_eq!(
        summary(&mp4),
        [(Preset::Audio, vec!["http_mp3_128"], None, "m4a")]
    );
    assert_eq!(mp4[0].size, Some(exact(6_592_000)));
    let original = presets(&page, ContainerPreference::Original);
    assert_eq!(original[0].container, "mp3");
    assert_eq!(original[0].audio_codec.as_deref(), Some("mp3"));
    assert_eq!(
        page.best_thumbnail(),
        Some("https://i1.sndcdn.com/artworks-t500x500.jpg")
    );
}

#[test]
fn flat_playlist() {
    let page = info("flat_playlist.json");
    assert!(page.is_playlist());
    assert_eq!(page.entry_count(), 3);
    assert_eq!(page.entries[1].title.as_deref(), Some("Part 2: the build"));
    assert_eq!(page.entries[1].duration, Some(1322.0));
    assert_eq!(page.entries[2].duration, None);
    assert_eq!(
        page.entries[0].url.as_deref(),
        Some("https://www.youtube.com/watch?v=aaaaaaaaaa1")
    );
    assert_eq!(presets(&page, ContainerPreference::Mp4).len(), 0);
    assert!(!page.drm_only());
}

#[test]
fn flat_playlist_entries_for_the_picker() {
    let list = playlist::entries(&info("flat_playlist.json"), 1);
    let index: Vec<u32> = list.iter().map(|e| e.index).collect();
    assert_eq!(index, [1, 2, 3]);
    let available: Vec<bool> = list.iter().map(|e| e.available).collect();
    assert_eq!(
        available,
        [true, true, false],
        "the private video is shown disabled"
    );
}

/// A watch page that belongs to a playlist (`watch?v=…&list=…`): "this
/// video" (`--no-playlist`) is the video, "the whole playlist"
/// (`--yes-playlist`) lists the playlist's entries, a page at a time.
#[tokio::test]
async fn a_watch_page_in_a_playlist_is_the_video_or_the_playlist() {
    let (_tmp, dir, prober) = fake();
    let fixtures = support::fixture("info");
    std::fs::copy(
        fixtures.join("youtube_like.json"),
        dir.join("fake-info.json"),
    )
    .unwrap();
    std::fs::copy(
        fixtures.join("watch_in_playlist.json"),
        dir.join("fake-info-playlist.json"),
    )
    .unwrap();
    let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLbpi6ZahtOH6Blw3RGYpWkSByi_T7Rygb";

    let video = prober.probe(url, &ProbeOptions::default()).await.unwrap();
    assert!(!video.is_playlist());
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    assert!(args.lines().any(|a| a == "--no-playlist"), "{args}");
    assert!(!args.contains("--playlist-items"), "{args}");

    let opts = ProbeOptions {
        playlist: PlaylistMode::Entries,
        items: Some(ItemRange::page(1)),
        ..ProbeOptions::default()
    };
    let list = prober.probe(url, &opts).await.unwrap();
    assert!(list.is_playlist());
    assert_eq!(list.title.as_deref(), Some("Making a chair: the series"));
    assert_eq!(list.publisher(), Some("Workshop Notes"));
    let args = std::fs::read_to_string(dir.join("fake-args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert!(args.contains(&"--yes-playlist") && args.contains(&"--flat-playlist"));
    let i = args.iter().position(|a| *a == "--playlist-items").unwrap();
    assert_eq!(args[i + 1], "1:200");
    let entries = playlist::entries(&list, 1);
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries[0].thumbnail.as_deref(),
        Some("https://i.ytimg.com/vi/bbbbbbbbbb1/hqdefault.jpg?sqp=b")
    );
    assert!(!entries[2].available);
    assert_eq!(entries[3].duration, Some(1306.0));
    assert_eq!(
        playlist::folder_name(list.title.as_deref(), list.id.as_deref()),
        "Making a chair_ the series"
    );
}

#[test]
fn drm_protected_page_has_no_presets() {
    let page = info("drm.json");
    assert!(page.drm_only());
    assert_eq!(presets(&page, ContainerPreference::Mp4).len(), 0);
    assert_eq!(presets(&page, ContainerPreference::Original).len(), 0);

    // Also when yt-dlp lists the protected formats (they are never chosen).
    let listed = MediaInfo::from_json(
        br#"{"formats": [
            {"format_id": "v", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 720, "has_drm": true},
            {"format_id": "a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "has_drm": true}
        ]}"#,
    )
    .unwrap();
    assert!(listed.drm_only());
    assert_eq!(presets(&listed, ContainerPreference::Mp4).len(), 0);
    // A page with some unprotected formats: those are used.
    let mixed = MediaInfo::from_json(
        br#"{"formats": [
            {"format_id": "free", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a.40.2", "height": 360},
            {"format_id": "v", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a.40.2", "height": 720, "has_drm": true}
        ]}"#,
    )
    .unwrap();
    assert!(!mixed.drm_only());
    let p = presets(&mixed, ContainerPreference::Mp4);
    assert_eq!(p[0].format_ids, ["free"]);
    assert_eq!(mixed.max_height(), Some(360));
}
