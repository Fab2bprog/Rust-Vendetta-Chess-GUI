//! Debug mode — structured JSON logging (PHASE 26sexies, 04/07/2026).
//!
//! Born while diagnosing a real bug in the "variation editing" click
//! flow (PHASE 26bis/26ter/26quater), this mechanism is kept
//! permanently as a general diagnostic facility — per the user's
//! explicit request, who then asked (04/07/2026) for: activation via
//! a checkbox persisted in Preferences (the "Misc" tab)
//! rather than an environment variable ("one day I'll forget it and
//! risk shipping the program stuck in debug mode"), a structured
//! JSON format ("let's create a standard JSON bug-report record"),
//! a launch identifier and a game identifier on each line
//! (to know whether the software was relaunched or the game changed), the
//! launch date/time, and the recording of the moves actually played
//! (deemed "strategic").
//!
//! # Activation
//!
//! Disabled by default: [`log_event`] does rigorously nothing (a
//! simple boolean test, no I/O) until [`set_debug_enabled`] has
//! been called with `true` — driven from the "Debug mode" checkbox in
//! the "Misc" Preferences tab, itself persisted via
//! `prefs::{save,load}_debug_mode_enabled`. Takes effect immediately, with no
//! need to restart the software.
//!
//! # Produced file
//!
//! `logs/debug_report.jsonl`, next to the executable (see
//! [`app_paths::logs_dir`]). **JSON Lines** format: one compact JSON object
//! per line rather than a single array — allows adding lines without
//! ever having to reread/rewrite the whole file. **Reset (truncated)
//! at the very first record of each launch**, then appended to
//! for the rest of the session: a fresh log on each startup
//! rather than a file that would grow indefinitely across sessions.
//!
//! Each record contains at minimum `ts_ms` (timestamp, Unix
//! milliseconds), `launch_guid` (identical for the whole duration of the
//! process), `game_guid` (regenerated on each new game via [`new_game`]) and
//! `event` (event name) — plus fields specific to the event.
//! The very first record of the file only carries the full launch
//! date/time (`logiciel_demarre`), so the temporal context can be read
//! immediately without having to convert `ts_ms` by hand.
//!
//! The identifiers are **not** RFC 4122 globally-unique-guaranteed
//! UUIDs (no `uuid`/`rand` dependency in the project, consistent with the
//! rest of the code, which only uses `std`) — just a string in the
//! usual UUID format (readable, familiar), unique enough in practice
//! to correlate lines from the same launch or the same game together.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

const LOG_FILE_NAME: &str = "debug_report.jsonl";

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables/disables debug mode — called at startup (from the
/// persisted preference) and on each toggle of the "Misc" checkbox.
/// Takes effect immediately.
pub fn set_debug_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

fn debug_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Correlation identifiers (launch / game)
// ---------------------------------------------------------------------------

/// Unique identifier of the current launch — identical from startup to
/// the program's closing, generated on first call then cached.
fn launch_guid() -> &'static str {
    static GUID: OnceLock<String> = OnceLock::new();
    GUID.get_or_init(generate_guid)
}

static GAME_GUID: Mutex<Option<String>> = Mutex::new(None);

/// Regenerates the game identifier — to be called every time a new
/// game actually starts (Assistant H vs H/H vs Engine/M vs M, puzzle,
/// tournament), so that the log can distinguish two successive games
/// of the same launch.
pub fn new_game() {
    let mut guard = GAME_GUID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(generate_guid());
}

fn game_guid() -> String {
    let mut guard = GAME_GUID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        *guard = Some(generate_guid());
    }
    guard.clone().unwrap_or_default()
}

/// Generates a string in the usual UUID format (`xxxxxxxx-xxxx-xxxx-xxxx-
/// xxxxxxxxxxxx`) from the system clock, the process id, and
/// a counter — see the module note on uniqueness "in practice" rather
/// than guaranteed in the RFC 4122 sense (no `uuid`/`rand` dependency required).
// Clippy (04/07/2026): `#[allow(cast_possible_truncation)]` — the GUID
// mixing deliberately keeps only the low bits of the nanosecond timestamp
// (intentional truncation, not an accidental loss of precision);
// see the module note on uniqueness "in practice" rather than RFC 4122.
#[allow(clippy::cast_possible_truncation)]
fn generate_guid() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let pid = std::process::id();

    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        ((nanos >> 32) as u32) ^ pid,
        ((nanos >> 16) & 0xffff) as u16,
        (nanos & 0xffff) as u16,
        (pid & 0xffff) as u16,
        ((u64::from(pid) << 20) ^ u64::from(counter)) & 0xffff_ffff_ffff,
    )
}

// ---------------------------------------------------------------------------
// Date/time (no dependency — same algorithm as
// `pdf_export::civil_from_days`, Howard Hinnant, public domain)
// ---------------------------------------------------------------------------

// Clippy (04/07/2026): `#[allow(cast_sign_loss, cast_possible_wrap,
// cast_possible_truncation)]` — Howard Hinnant's algorithm (public
// domain); the values handled (days since the epoch, year/month/day)
// stay far below the bounds of `u64`/`i64`/`u32` for any plausible
// civil date, the conversions are safe by construction of the algorithm.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Current date and time, formatted `DD/MM/YYYY HH:MM:SS`.
// Clippy: `#[allow(cast_possible_truncation)]` — `now_millis() / 1000` (seconds
// since the Unix epoch) comfortably fits in an `i64` before the year 292 billion.
#[allow(clippy::cast_possible_truncation)]
fn now_datetime_string() -> String {
    let secs = (now_millis() / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem  = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    format!("{day:02}/{month:02}/{year:04} {h:02}:{m:02}:{s:02}")
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

// ---------------------------------------------------------------------------
// Log writing
// ---------------------------------------------------------------------------

static FRESH_LOG_DONE: AtomicBool = AtomicBool::new(false);

/// Appends a JSON record (one line) to `logs/debug_report.jsonl` —
/// `ts_ms`, `launch_guid`, `game_guid` and `event` are filled in
/// automatically; `fields` provides the fields specific to this
/// event (merged into the same JSON object).
///
/// Does rigorously nothing if debug mode is disabled (default
/// case): a simple boolean test, no disk access, no measurable cost
/// in normal usage.
///
/// Best-effort: any I/O error (folder not creatable, disk
/// full, permissions…) is silently ignored — a logging concern
/// must never crash or disturb the game.
pub fn log_event(event: &str, fields: &serde_json::Value) {
    if !debug_enabled() {
        return;
    }

    let dir = app_paths::logs_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(LOG_FILE_NAME);

    // First write of this launch → truncated file (fresh log);
    // all following ones → append, to accumulate the whole session.
    let is_first_write_this_run = !FRESH_LOG_DONE.swap(true, Ordering::Relaxed);

    let file = if is_first_write_this_run {
        std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path)
    } else {
        std::fs::OpenOptions::new().create(true).append(true).open(&path)
    };
    let Ok(mut file) = file else { return };

    if is_first_write_this_run {
        let startup = serde_json::json!({
            "ts_ms": now_millis(),
            "launch_guid": launch_guid(),
            "event": "logiciel_demarre",
            "date_heure": now_datetime_string(),
        });
        let _ = writeln!(file, "{startup}");
    }

    let mut record = serde_json::json!({
        "ts_ms": now_millis(),
        "launch_guid": launch_guid(),
        "game_guid": game_guid(),
        "event": event,
    });
    if let (Some(map), Some(extra)) = (record.as_object_mut(), fields.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }

    let _ = writeln!(file, "{record}");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_civil_from_days_unix_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn test_civil_from_days_known_reference_2000_01_01() {
        // 10957 days between 1970-01-01 and 2000-01-01 (known reference).
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }

    #[test]
    fn test_generate_guid_has_uuid_shape() {
        let guid = generate_guid();
        let parts: Vec<&str> = guid.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }

    #[test]
    fn test_generate_guid_calls_are_distinct() {
        assert_ne!(generate_guid(), generate_guid());
    }

    #[test]
    fn test_launch_guid_is_stable_across_calls() {
        assert_eq!(launch_guid(), launch_guid());
    }

    #[test]
    fn test_new_game_changes_game_guid() {
        new_game();
        let first = game_guid();
        new_game();
        let second = game_guid();
        assert_ne!(first, second);
    }

    #[test]
    fn test_log_event_no_panic_when_disabled() {
        // By default (no set_debug_enabled(true) called in this test),
        // log_event must never panic nor write anything.
        log_event("test_event", &serde_json::json!({ "foo": "bar" }));
    }

    #[test]
    fn test_now_datetime_string_has_expected_format() {
        let s = now_datetime_string();
        // "DD/MM/YYYY HH:MM:SS" → 19 characters.
        assert_eq!(s.len(), 19, "format obtenu : {s}");
        assert_eq!(s.as_bytes()[2], b'/');
        assert_eq!(s.as_bytes()[5], b'/');
        assert_eq!(s.as_bytes()[10], b' ');
        assert_eq!(s.as_bytes()[13], b':');
        assert_eq!(s.as_bytes()[16], b':');
    }
}
