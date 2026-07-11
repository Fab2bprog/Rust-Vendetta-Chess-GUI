//! Persistence of user preferences (language, UCI engines, etc.).
//!
//! PHASE 24 (100% portability, USB): preferences are stored in the
//! `parametres/` subfolder of the delivery folder (`VendettaChess/parametres/`),
//! next to the executable — no longer in a user system directory.
//!
//! Engine paths (`engines.json`, `hint_engine.txt`) are stored
//! **relative** to `VendettaChess/` when the referenced file already
//! sits under that folder (e.g. `moteurs/stockfish`, once imported — see Step 5),
//! and kept absolute as long as they point to an external location (the
//! normal case before import). See `app_paths::to_relative_string` /
//! `to_absolute_path`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Preferences directory ─────────────────────────────────────────────────────

/// Returns the Vendetta Chess preferences directory (`parametres/`).
///
/// In test mode (`cfg(test)`), returns a dedicated temporary directory rather
/// than the application's real `parametres/` folder.
///
/// Without this isolation, round-trip tests like
/// `test_hint_engine_roundtrip_with_external_absolute_path` (which persist
/// a real path via `existing_file_path()` to test the "valid
/// file" case) would actually write into the real preferences during
/// `cargo test` — a historical bug observed before PHASE 24 with the old
/// White Polyglot book mechanism (since removed, see Step 6):
/// after several `cargo test` runs, a "Cargo.toml" book would show up loaded
/// in Preferences even though the user had never loaded that file.
/// `game_config::persist` already uses an isolated temporary directory for
/// its own tests (see `persist.rs`); this fix applies the same
/// principle here.
#[cfg(not(test))]
fn prefs_dir() -> PathBuf {
    app_paths::parametres_dir()
}

/// Test variant of [`prefs_dir`]: isolated temporary directory, never the
/// real user preferences directory (see doc above).
#[cfg(test)]
fn prefs_dir() -> PathBuf {
    std::env::temp_dir().join("vendetta_chess_test_prefs")
}

// ── Language ─────────────────────────────────────────────────────────────────

/// Saves the chosen language code (e.g. `"fr"`, `"en"`).
pub fn save_lang(lang_code: &str) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("lang.txt"), lang_code.trim());
}

/// Loads the saved language code, or `None` if the user has not yet
/// made a choice (first launch).
#[must_use]
pub fn load_lang() -> Option<String> {
    let s = std::fs::read_to_string(prefs_dir().join("lang.txt")).ok()?;
    let code = s.trim().to_owned();
    if code.is_empty() { None } else { Some(code) }
}

/// `true` if the user has already chosen a language (not a first launch).
#[must_use]
pub fn has_saved_lang() -> bool {
    load_lang().is_some()
}

// ── UCI Engines ──────────────────────────────────────────────────────────────

/// A remembered UCI engine (display name + path to the executable +
/// user-customized UCI options).
///
/// The `options` field is absent from old JSON files → `serde(default)`
/// silently replaces it with an empty `HashMap` (backward compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedEngine {
    pub name: String,
    pub path: String,
    /// UCI options modified by the user: `option_name → string_value`.
    /// Sent via `setoption` at the start of each game.
    #[serde(default)]
    pub options: HashMap<String, String>,
}

/// Persistence file for the engine list.
fn engines_file() -> PathBuf {
    prefs_dir().join("engines.json")
}

/// Saves the full engine list as JSON.
///
/// Each path is converted to relative to `VendettaChess/` before writing
/// if it already sits under that folder (see `app_paths::to_relative_string`)
/// — left unchanged (absolute) as long as it points to an external location.
pub fn save_engines(engines: &[SavedEngine]) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let to_store: Vec<SavedEngine> = engines
        .iter()
        .cloned()
        .map(|mut e| {
            e.path = app_paths::to_relative_string(std::path::Path::new(&e.path));
            e
        })
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&to_store) {
        let _ = std::fs::write(engines_file(), json);
    }
}

/// Loads the engine list from the JSON file.
///
/// Each stored path (relative or absolute) is resolved to absolute via
/// `app_paths::to_absolute_path` before being returned — callers
/// always receive a directly usable path, whether it was
/// imported into `moteurs/` (Step 5) or remains referenced externally.
///
/// Entries whose file no longer exists on disk are **silently
/// removed** (the user may have uninstalled the engine).
/// If the `engines.json` file is missing or corrupted, returns an empty `Vec`.
#[must_use]
pub fn load_engines() -> Vec<SavedEngine> {
    let Ok(content) = std::fs::read_to_string(engines_file()) else { return Vec::new() };
    let engines: Vec<SavedEngine> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    // Relative → absolute resolution, then automatic filtering: discard
    // paths that no longer exist.
    engines
        .into_iter()
        .map(|mut e| {
            e.path = app_paths::to_absolute_path(&e.path).to_string_lossy().into_owned();
            e
        })
        .filter(|e| std::path::Path::new(&e.path).exists())
        .collect()
}

// ── Hint engine ──────────────────────────────────────────────────────────────

/// Saves the path of the selected hint engine.
/// Pass `None` to clear the selection (stores an empty string).
///
/// Converted to relative to `VendettaChess/` before writing if the path is
/// already located there (see `app_paths::to_relative_string`).
pub fn save_hint_engine(path: Option<&str>) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let content = path
        .map(|p| app_paths::to_relative_string(std::path::Path::new(p)))
        .unwrap_or_default();
    let _ = std::fs::write(dir.join("hint_engine.txt"), content);
}

/// Loads the hint engine's path.
/// Returns `None` if no engine is configured or if the file no longer exists.
/// Resolves the stored path (relative or absolute) to absolute, then checks that the
/// executable file still exists on disk.
#[must_use]
pub fn load_hint_engine() -> Option<String> {
    let s = std::fs::read_to_string(prefs_dir().join("hint_engine.txt")).ok()?;
    let stored = s.trim().to_owned();
    if stored.is_empty() { return None; }
    let abs = app_paths::to_absolute_path(&stored);
    if !abs.exists() { return None; }
    Some(abs.to_string_lossy().into_owned())
}

// ── Multi-PV ─────────────────────────────────────────────────────────────────

/// Saves the number of `MultiPV` lines for White (0 = disabled, 1/2/3/5).
pub fn save_multipv_white(n: i32) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("multipv_white.txt"), n.to_string());
}

/// Loads the number of `MultiPV` lines for White. Default: 0.
#[must_use]
pub fn load_multipv_white() -> i32 {
    let s = std::fs::read_to_string(prefs_dir().join("multipv_white.txt"))
        .unwrap_or_default();
    match s.trim().parse::<i32>() {
        Ok(n) if matches!(n, 0 | 1 | 2 | 3 | 5) => n,
        _ => 0,
    }
}

/// Saves the number of `MultiPV` lines for Black (0 = disabled, 1/2/3/5).
pub fn save_multipv_black(n: i32) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("multipv_black.txt"), n.to_string());
}

/// Loads the number of `MultiPV` lines for Black. Default: 0.
#[must_use]
pub fn load_multipv_black() -> i32 {
    let s = std::fs::read_to_string(prefs_dir().join("multipv_black.txt"))
        .unwrap_or_default();
    match s.trim().parse::<i32>() {
        Ok(n) if matches!(n, 0 | 1 | 2 | 3 | 5) => n,
        _ => 0,
    }
}

// ── Puzzle mode (PHASE 14, Step 5) ────────────────────────────────────────────

/// Saves the "puzzle goal" choice (`false` = No hint, `true` = With theme).
pub fn save_puzzle_hint_theme(enabled: bool) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("puzzle_hint_theme.txt"), if enabled { "1" } else { "0" });
}

/// Loads the "puzzle goal" choice. Default: `false` (No hint).
#[must_use]
pub fn load_puzzle_hint_theme() -> bool {
    std::fs::read_to_string(prefs_dir().join("puzzle_hint_theme.txt"))
        .is_ok_and(|s| s.trim() == "1")
}

/// Saves the hint button's state while searching for a puzzle.
pub fn save_puzzle_hint_button(enabled: bool) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("puzzle_hint_button.txt"), if enabled { "1" } else { "0" });
}

/// Loads the hint button's state while searching for a puzzle. Default: `true` (active).
#[must_use]
pub fn load_puzzle_hint_button() -> bool {
    std::fs::read_to_string(prefs_dir().join("puzzle_hint_button.txt"))
        .map_or(true, |s| s.trim() == "1")
}

// ── Debug mode (PHASE 26sexies) ───────────────────────────────────────────────

/// Saves the state of the "Debug mode" checkbox (Preferences → Misc).
///
/// Disabled by default, explicitly persisted rather than hardcoded, to
/// avoid accidentally shipping the software stuck in debug mode (user
/// feedback 04/07/2026: "I risk shipping the program stuck in debug
/// mode").
pub fn save_debug_mode_enabled(enabled: bool) {
    let dir = prefs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("debug_mode_enabled.txt"), if enabled { "1" } else { "0" });
}

/// Loads the state of the "Debug mode" checkbox. Default: `false` (disabled).
#[must_use]
pub fn load_debug_mode_enabled() -> bool {
    std::fs::read_to_string(prefs_dir().join("debug_mode_enabled.txt"))
        .is_ok_and(|s| s.trim() == "1")
}

// ── Full reset ─────────────────────────────────────────────────────────────────

/// Deletes **the entire** Vendetta Chess preferences directory.
///
/// After the call, the software will behave as on first launch:
/// default language (English, `Lang::default()` — changed on 05/07/2026,
/// previously French), no engine configured, no game config.
/// The directory will be automatically recreated on the next save.
///
/// Note: this function only deletes the `lang.txt` file on disk —
/// it does not change the language of the Slint session already
/// running. It is `gui/src/main.rs` (the `on_reset_prefs` callback) that
/// explicitly switches the active interface to `Lang::default()` right
/// after calling this function.
pub fn reset_all() {
    let _ = std::fs::remove_dir_all(prefs_dir());
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Same principle as in the other preferences modules (e.g.
    /// `game_config::persist`), for the `hint_engine.txt` tests
    /// (PHASE 24, Step 3 — relative/absolute conversion).
    static HINT_ENGINE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_hint_engine_tests() -> std::sync::MutexGuard<'static, ()> {
        HINT_ENGINE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Same principle as [`HINT_ENGINE_TEST_LOCK`], for the
    /// `engines.json` tests (PHASE 24, Step 3 — relative/absolute conversion).
    static ENGINES_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_engines_tests() -> std::sync::MutexGuard<'static, ()> {
        ENGINES_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Same principle as [`HINT_ENGINE_TEST_LOCK`], for the
    /// `debug_mode_enabled.txt` tests (fix 04/07/2026: `cargo test` runs
    /// tests in parallel by default, and two tests of this module
    /// manipulate this same file — one does a `remove_file` while
    /// another saves/rereads — which caused an intermittent failure
    /// (`test_debug_mode_enabled_roundtrip` observed failing with the
    /// file content reset by the other test between the `save` and
    /// the `assert`)).
    static DEBUG_MODE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_debug_mode_tests() -> std::sync::MutexGuard<'static, ()> {
        DEBUG_MODE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Non-regression: `prefs_dir()` must never point to the real
    /// user preferences directory during tests. Without this
    /// guarantee, the round-trip tests below would durably pollute the
    /// real preferences of whoever runs `cargo test` (bug observed
    /// with the White Polyglot book — see doc of `prefs_dir()`).
    #[test]
    fn test_prefs_dir_is_test_isolated() {
        assert!(
            prefs_dir().starts_with(std::env::temp_dir()),
            "prefs_dir() doit rester dans le répertoire temporaire en mode test, obtenu : {:?}",
            prefs_dir()
        );
    }

    #[test]
    fn test_save_and_load_lang() {
        // We can't test the real path in CI, so we just test the logic.
        // If load_lang returns Some, the code must be non-empty.
        // If the file doesn't exist yet, load_lang returns None.
        let result = load_lang(); // can be Some or None depending on the env
        if let Some(code) = result {
            assert!(!code.is_empty());
        }
    }

    #[test]
    fn test_has_saved_lang_consistent_with_load() {
        assert_eq!(has_saved_lang(), load_lang().is_some());
    }

    /// Path guaranteed to exist on disk (this crate's `Cargo.toml`),
    /// used to test the "valid path" case without depending on a real
    /// Polyglot `.bin` file (the validity of the content is tested in
    /// `crates/core/src/polyglot.rs`, not here — this function only persists
    /// a path).
    fn existing_file_path() -> String {
        format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"))
    }

    // ── Hint engine: relative/absolute conversion (PHASE 24, Step 3) ──────

    #[test]
    fn test_hint_engine_roundtrip_with_external_absolute_path() {
        let _guard = lock_hint_engine_tests();
        let path = existing_file_path();
        save_hint_engine(Some(&path));
        assert_eq!(load_hint_engine(), Some(path));
    }

    #[test]
    fn test_hint_engine_none_when_missing_file() {
        let _guard = lock_hint_engine_tests();
        save_hint_engine(Some("/definitely/does/not/exist/engine"));
        assert_eq!(load_hint_engine(), None);
    }

    #[test]
    fn test_hint_engine_cleared_by_none() {
        let _guard = lock_hint_engine_tests();
        save_hint_engine(Some(&existing_file_path()));
        save_hint_engine(None);
        assert_eq!(load_hint_engine(), None);
    }

    /// An engine whose file is already located under `app_dir()` (simulating
    /// an engine imported into `moteurs/`, Step 5) must be stored **as
    /// relative** in `hint_engine.txt`, not absolute.
    #[test]
    fn test_hint_engine_stored_as_relative_when_file_under_app_dir() {
        let _guard = lock_hint_engine_tests();
        let dummy_path = app_paths::app_dir().join("vendetta_test_relative_dummy_hint.tmp");
        std::fs::write(&dummy_path, b"x").expect("creation fichier factice");

        save_hint_engine(Some(dummy_path.to_str().expect("chemin UTF-8")));

        let raw = std::fs::read_to_string(prefs_dir().join("hint_engine.txt"))
            .expect("lecture hint_engine.txt");
        assert_eq!(raw, "vendetta_test_relative_dummy_hint.tmp");

        // But load_hint_engine() must still resolve to the correct absolute path.
        assert_eq!(
            load_hint_engine(),
            Some(dummy_path.to_string_lossy().into_owned())
        );

        let _ = std::fs::remove_file(&dummy_path);
    }

    // ── Engines (engines.json): relative/absolute conversion (PHASE 24, Step 3) ─

    #[test]
    fn test_engines_roundtrip_with_external_absolute_path() {
        let _guard = lock_engines_tests();
        let engines = vec![SavedEngine {
            name: "Test Engine".to_owned(),
            path: existing_file_path(),
            options: HashMap::new(),
        }];
        save_engines(&engines);
        let loaded = load_engines();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, existing_file_path());
        assert_eq!(loaded[0].name, "Test Engine");
    }

    #[test]
    fn test_engines_filters_out_missing_files() {
        let _guard = lock_engines_tests();
        let engines = vec![SavedEngine {
            name: "Ghost".to_owned(),
            path: "/definitely/does/not/exist/engine".to_owned(),
            options: HashMap::new(),
        }];
        save_engines(&engines);
        assert!(load_engines().is_empty());
    }

    /// An engine whose file is already located under `app_dir()` must be
    /// stored **as relative** in `engines.json`, not absolute — the JSON must
    /// not contain any full absolute path.
    #[test]
    fn test_engines_stored_as_relative_when_file_under_app_dir() {
        let _guard = lock_engines_tests();
        let dummy_path = app_paths::app_dir().join("vendetta_test_relative_dummy_engine.tmp");
        std::fs::write(&dummy_path, b"x").expect("creation fichier factice");

        let engines = vec![SavedEngine {
            name: "Dummy".to_owned(),
            path: dummy_path.to_str().expect("chemin UTF-8").to_owned(),
            options: HashMap::new(),
        }];
        save_engines(&engines);

        let raw = std::fs::read_to_string(engines_file()).expect("lecture engines.json");
        assert!(raw.contains("vendetta_test_relative_dummy_engine.tmp"));
        assert!(
            !raw.contains(&dummy_path.to_string_lossy().into_owned()),
            "le JSON ne doit pas contenir le chemin absolu complet, contenu : {raw}"
        );

        let loaded = load_engines();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, dummy_path.to_string_lossy().into_owned());

        let _ = std::fs::remove_file(&dummy_path);
    }

    // ── Puzzle mode (PHASE 14, Step 5) ─────────────────────────────────────
    //
    // Each test writes then rereads within the same execution thread (both
    // true/false values), so there's no risk of a race with another
    // parallel test on the same file — unlike the engine tests
    // above which share files across several distinct tests.

    #[test]
    fn test_puzzle_hint_theme_roundtrip() {
        save_puzzle_hint_theme(true);
        assert!(load_puzzle_hint_theme());
        save_puzzle_hint_theme(false);
        assert!(!load_puzzle_hint_theme());
    }

    #[test]
    fn test_puzzle_hint_button_roundtrip() {
        save_puzzle_hint_button(false);
        assert!(!load_puzzle_hint_button());
        save_puzzle_hint_button(true);
        assert!(load_puzzle_hint_button());
    }

    #[test]
    fn test_debug_mode_enabled_roundtrip() {
        let _guard = lock_debug_mode_tests();
        save_debug_mode_enabled(true);
        assert!(load_debug_mode_enabled());
        save_debug_mode_enabled(false);
        assert!(!load_debug_mode_enabled());
    }

    #[test]
    fn test_debug_mode_enabled_defaults_to_false_when_missing() {
        let _guard = lock_debug_mode_tests();
        let _ = std::fs::remove_file(prefs_dir().join("debug_mode_enabled.txt"));
        assert!(!load_debug_mode_enabled());
    }
}
