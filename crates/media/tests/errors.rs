//! yt-dlp's failures sorted into the user-facing categories, on a corpus of
//! its messages (`fixtures/errors/stderr.txt`).
#![allow(clippy::unwrap_used)]

use boltget_media::{ErrorCategory, Failure};

fn category(name: &str) -> ErrorCategory {
    ErrorCategory::ALL
        .into_iter()
        .find(|c| c.message_key() == format!("error.media.{name}"))
        .unwrap_or_else(|| panic!("unknown category {name}"))
}

/// The corpus: `(category, standard error)` per block.
fn corpus() -> Vec<(ErrorCategory, String)> {
    let text = include_str!("fixtures/errors/stderr.txt");
    let mut out: Vec<(ErrorCategory, String)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            out.push((category(name.trim()), String::new()));
        } else if let Some((_, stderr)) = out.last_mut()
            && !line.starts_with('#')
        {
            stderr.push_str(line);
            stderr.push('\n');
        }
    }
    out
}

#[test]
fn every_sample_lands_in_its_category() {
    let samples = corpus();
    assert!(samples.len() >= 40, "{} samples", samples.len());
    let mut wrong = Vec::new();
    for (expected, stderr) in &samples {
        let failure = Failure::from_stderr(stderr, "exit code: 1");
        if failure.category != *expected {
            wrong.push(format!(
                "expected {expected:?}, got {:?}: {}",
                failure.category, failure.message
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // Every category has samples.
    for c in ErrorCategory::ALL {
        assert!(samples.iter().any(|(e, _)| *e == c), "no sample for {c:?}");
    }
}

#[test]
fn the_message_is_the_last_error_line() {
    let f = Failure::from_stderr(
        "WARNING: [youtube] x: nsig extraction failed\nERROR: [youtube] x: Video unavailable\n",
        "exit code: 1",
    );
    assert_eq!(f.message, "[youtube] x: Video unavailable");
    assert_eq!(f.category, ErrorCategory::Unavailable);
    let f = Failure::from_stderr("", "exit code: 3");
    assert_eq!(f.category, ErrorCategory::Other);
    assert!(f.message.contains("exit code: 3"), "{}", f.message);
}
