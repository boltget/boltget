//! The string catalog for text Rust shows itself: the tray's menu and
//! tooltip, and notifications. It is the page's catalog
//! (`packages/ui/src/messages/en.json`, TECH-STACK.md ADR-015), compiled in,
//! with the same rules: named placeholders, plural messages chosen by
//! `count`, and the same number and unit formatting as `format.ts`.

use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{Map, Value};

static CATALOG: LazyLock<Map<String, Value>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../packages/ui/src/messages/en.json"))
        .unwrap_or_default()
});

/// A placeholder value.
#[derive(Debug, Clone)]
pub(crate) enum Arg<'a> {
    Text(&'a str),
    Number(u64),
}

/// Looks up a message without placeholders; the key itself if it is
/// missing, like the page's `t()`.
pub(crate) fn t(key: &str) -> String {
    tr(key, &[])
}

/// Whether the catalog has `key`.
pub(crate) fn has(key: &str) -> bool {
    CATALOG.contains_key(key)
}

/// Looks up a message and fills its placeholders. A plural message picks
/// `one` when `count` is 1 and `other` otherwise (English).
pub(crate) fn tr(key: &str, args: &[(&str, Arg<'_>)]) -> String {
    let text = match CATALOG.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(forms)) => {
            let count = args.iter().find_map(|(name, a)| match (name, a) {
                (&"count", Arg::Number(n)) => Some(*n),
                _ => None,
            });
            let form = if count == Some(1) { "one" } else { "other" };
            forms
                .get(form)
                .or_else(|| forms.get("other"))
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_owned()
        }
        _ => return key.to_owned(),
    };
    let mut out = text;
    for (name, arg) in args {
        let value = match arg {
            Arg::Text(s) => (*s).to_owned(),
            Arg::Number(n) => grouped(*n),
        };
        out = out.replace(&format!("{{{name}}}"), &value);
    }
    out
}

/// `1,234`, as `Intl.NumberFormat("en")` writes integers.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

const UNIT_KEYS: [&str; 5] = ["unit.b", "unit.kib", "unit.mib", "unit.gib", "unit.tib"];
const SPEED_KEYS: [&str; 5] = [
    "speed.b",
    "speed.kib",
    "speed.mib",
    "speed.gib",
    "speed.tib",
];

/// The binary unit (0 = B, 1 = KiB, …) that shows `bytes` best.
fn unit_of(bytes: u64) -> usize {
    let mut unit = 0;
    let mut value = bytes as f64;
    while value >= 1000.0 && unit < UNIT_KEYS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    unit
}

/// `bytes` in `unit`: 2 decimals under 10, 1 under 100, then none; `fixed`
/// keeps trailing zeros (numbers that change live).
fn scaled(bytes: u64, unit: usize, fixed: bool) -> String {
    if unit == 0 {
        return grouped(bytes);
    }
    let value = bytes as f64 / 1024f64.powi(i32::try_from(unit).unwrap_or(0));
    let digits = if value < 10.0 {
        2
    } else {
        usize::from(value < 100.0)
    };
    let text = format!("{value:.digits$}");
    if fixed || !text.contains('.') {
        text
    } else {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// `420 MiB`, `1.25 GiB`.
pub(crate) fn bytes(n: u64) -> String {
    let unit = unit_of(n);
    tr(
        UNIT_KEYS[unit],
        &[("value", Arg::Text(&scaled(n, unit, false)))],
    )
}

/// `48.2 MiB/s`.
pub(crate) fn speed(bps: u64) -> String {
    let unit = unit_of(bps);
    tr(
        SPEED_KEYS[unit],
        &[("value", Arg::Text(&scaled(bps, unit, true)))],
    )
}

/// `8 s left`, `about 3 min left`, `about 2 h 10 min left`.
pub(crate) fn eta(seconds: u64) -> String {
    if seconds < 60 {
        return tr("eta.seconds", &[("count", Arg::Number(seconds.max(1)))]);
    }
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return tr("eta.minutes", &[("count", Arg::Number(minutes))]);
    }
    if seconds >= 86_400 {
        return t("eta.days");
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if rest == 0 {
        tr("eta.hours", &[("count", Arg::Number(hours))])
    } else {
        tr(
            "eta.hours_minutes",
            &[
                ("hours", Arg::Number(hours)),
                ("minutes", Arg::Number(rest)),
            ],
        )
    }
}

/// How long something took: `7.9 s` under 10 s, then `42 s`, `4 min`,
/// `2 h 10 min`.
pub(crate) fn took(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 10.0 {
        let tenths = format!("{:.1}", secs.max(0.1));
        // A message with a count placeholder; the count here has a decimal.
        return tr("duration.seconds", &[("count", Arg::Text(&tenths))]);
    }
    let s = d.as_secs();
    if s < 60 {
        return tr("duration.seconds", &[("count", Arg::Number(s))]);
    }
    let minutes = (s + 30) / 60;
    if minutes < 60 {
        return tr("duration.minutes", &[("count", Arg::Number(minutes))]);
    }
    tr(
        "duration.hours",
        &[
            ("hours", Arg::Number(minutes / 60)),
            ("minutes", Arg::Number(minutes % 60)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_messages_like_the_page() {
        assert_eq!(t("app.name"), "Boltget");
        assert_eq!(t("no.such.key"), "no.such.key");
        assert_eq!(tr("tray.active", &[("count", Arg::Number(3))]), "3 active");
        assert_eq!(
            tr("notify.batch", &[("count", Arg::Number(1))]),
            "1 download finished"
        );
        assert_eq!(
            tr("notify.batch", &[("count", Arg::Number(1234))]),
            "1,234 downloads finished"
        );
        assert_eq!(
            tr(
                "status.join",
                &[("first", Arg::Text("a")), ("second", Arg::Text("b"))]
            ),
            "a · b"
        );
    }

    #[test]
    fn formats_units_like_format_ts() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(420 * 1024 * 1024), "420 MiB");
        assert_eq!(bytes(7 * 1024 * 1024), "7 MiB");
        assert_eq!(bytes(1_342_177_280), "1.25 GiB");
        assert_eq!(speed(50_541_363), "48.2 MiB/s");
        assert_eq!(speed(5 * 1024 * 1024), "5.00 MiB/s");
        assert_eq!(speed(0), "0 B/s");
    }

    #[test]
    fn formats_times() {
        assert_eq!(eta(8), "8 s left");
        assert_eq!(eta(0), "1 s left");
        assert_eq!(eta(150), "about 3 min left");
        assert_eq!(eta(7800), "about 2 h 10 min left");
        assert_eq!(eta(7200), "about 2 h left");
        assert_eq!(eta(90_000), "more than a day left");
        assert_eq!(took(Duration::from_millis(7900)), "7.9 s");
        assert_eq!(took(Duration::from_secs(42)), "42 s");
        assert_eq!(took(Duration::from_secs(250)), "4 min");
    }
}
