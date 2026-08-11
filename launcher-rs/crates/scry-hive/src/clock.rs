//! Turning a unix timestamp into something a person reads.
//!
//! ## Why this is 60 lines and not a dependency
//!
//! Operator, 2026-08-05: *"really utilize rust and any nice trusted rust
//! packages i guess."* — and the honest answer for this particular job is no.
//! `jiff` is excellent and its author is about as trusted as Rust gets, but
//! adding it here pulls **14 packages**, including `defmt` (an embedded
//! logging framework) and a second major version of `syn` — which would trip
//! the duplicate check in `supply_chain.rs` — to render *"3m ago"*.
//!
//! That is the dependency policy working rather than being ignored: the budget
//! exists so this trade gets made deliberately instead of by reflex. When
//! something here genuinely needs civil calendars, time zones or parsing,
//! `jiff` is the right answer and the cap should move for it.
//!
//! No timezone database, no leap seconds, no parsing. UTC and arithmetic.

/// Seconds since the unix epoch, now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `"3m ago"`, `"2h ago"`, `"4d ago"`. A chat feed printing raw epoch integers
/// is a chat feed nobody reads.
pub fn ago(at: i64, now: i64) -> String {
    let d = now - at;
    if d < 0 {
        // A clock skewed the wrong way is worth showing rather than hiding as
        // "0s ago" — it is usually the reader's clock and it explains a lot.
        return format!("in {}", span(-d));
    }
    if d < 5 {
        return "just now".into();
    }
    format!("{} ago", span(d))
}

fn span(d: i64) -> String {
    match d {
        d if d < 60 => format!("{d}s"),
        d if d < 3600 => format!("{}m", d / 60),
        d if d < 86_400 => format!("{}h", d / 3600),
        d if d < 86_400 * 365 => format!("{}d", d / 86_400),
        d => format!("{}y", d / (86_400 * 365)),
    }
}

/// `"2026-08-05 01:23Z"`. The civil-from-days algorithm (Howard Hinnant's,
/// public domain), which is the whole of what a calendar needs when the answer
/// is always UTC.
pub fn stamp(at: i64) -> String {
    let days = at.div_euclid(86_400);
    let secs = at.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `"2h 14m"` — a played duration, which reads differently from an age.
pub fn duration(secs: i64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h == 0 {
        return format!("{m}m");
    }
    format!("{h}h {m}m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_reads_like_a_chat_client() {
        assert_eq!(ago(100, 102), "just now");
        assert_eq!(ago(100, 130), "30s ago");
        assert_eq!(ago(0, 600), "10m ago");
        assert_eq!(ago(0, 7200), "2h ago");
        assert_eq!(ago(0, 86_400 * 3), "3d ago");
        // A skewed clock is shown, not hidden — it usually explains the rest
        // of what the reader is confused about. (Both expectations here were
        // wrong on the first pass and the test caught the author, not the
        // code: `span` rolls to minutes at 60, and the epoch below is
        // 07-23, cross-checked against `datetime.fromtimestamp`.)
        assert_eq!(ago(200, 100), "in 1m");
    }

    #[test]
    fn the_stamp_matches_known_dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00Z");
        assert_eq!(stamp(1_784_845_562), "2026-07-23 22:26Z");
        // Leap day, and the day after — the two the algorithm is for.
        assert_eq!(stamp(1_709_164_800), "2024-02-29 00:00Z");
        assert_eq!(stamp(1_709_251_200), "2024-03-01 00:00Z");
        // Before the epoch, which `div_euclid` handles and `/` would not.
        assert_eq!(stamp(-86_400), "1969-12-31 00:00Z");
    }

    #[test]
    fn a_played_duration_is_not_an_age() {
        assert_eq!(duration(30), "30s");
        assert_eq!(duration(600), "10m");
        assert_eq!(duration(8040), "2h 14m");
    }
}
