//! JSON persistence of game configurations.
//!
//! PHASE 24 (100% portability, USB): files are stored in the
//! `parametres/parties/` subfolder of the delivery folder
//! (`VendettaChess/parametres/parties/`), next to the executable — no longer
//! in a user system directory.
//!
//! One JSON file per game mode, plus `last_mode.txt`:
//!
//! | File             | Mode               |
//! |------------------|--------------------|
//! | `last_hvh.json`  | Human vs Human     |
//! | `last_hvm.json`  | Human vs Engine    |
//! | `last_mvm.json`  | Engine vs Engine   |
//! | `last_mode.txt`  | Last mode used (any of the three) |

use std::path::PathBuf;

use crate::{GameConfig, GameMode};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Persistence error.
#[derive(Debug)]
pub enum PersistError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e)   => write!(f, "Erreur I/O : {e}"),
            Self::Json(e) => write!(f, "Erreur JSON : {e}"),
        }
    }
}

impl std::error::Error for PersistError {}

impl From<std::io::Error> for PersistError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

impl From<serde_json::Error> for PersistError {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}

// ---------------------------------------------------------------------------
// Configuration directory
// ---------------------------------------------------------------------------

/// Returns the Vendetta Chess configuration directory (`parametres/parties/`).
///
/// In test mode (`cfg(test)`), returns a dedicated temporary directory
/// instead of the application's actual `parametres/parties/` folder — same
/// principle as `gui::prefs::prefs_dir()` (PHASE 24, Step 3), so that no
/// test ever depends on the real state of the delivery folder.
#[cfg(not(test))]
#[must_use]
pub fn config_dir() -> PathBuf {
    app_paths::parametres_parties_dir()
}

/// Test variant of [`config_dir`]: isolated temporary directory.
#[cfg(test)]
#[must_use]
pub fn config_dir() -> PathBuf {
    std::env::temp_dir().join("vendetta_chess_test_game_config")
}

// ---------------------------------------------------------------------------
// Internal keys
// ---------------------------------------------------------------------------

/// Writes `contents` to `path` atomically: writes to a temporary file in the
/// same directory then `rename`s it (atomic on POSIX and Windows as long as
/// source and destination are on the same volume, which is guaranteed here
/// since the temporary file is created next to the target).
///
/// Without this precaution, an interruption (crash, power loss) during a
/// direct `std::fs::write` to the final file could leave a truncated JSON
/// file — silently treated as "corrupted" on the next load
/// (`load_last_config` returns `None`), losing the last saved configuration.
fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vendetta_chess_tmp");
    // PID suffix: avoids collisions between concurrent instances of the
    // application writing simultaneously (rare but possible case).
    let tmp_path = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));

    std::fs::write(&tmp_path, contents)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn mode_key(mode: GameMode) -> &'static str {
    match mode {
        GameMode::HumanVsHuman   => "hvh",
        GameMode::HumanVsEngine  => "hvm",
        GameMode::EngineVsEngine => "mvm",
    }
}

// ---------------------------------------------------------------------------
// Remembering the last mode used
// ---------------------------------------------------------------------------

/// Saves the last game mode used.
///
/// # Errors
///
/// [`PersistError::Io`] if the write fails.
pub fn save_last_mode(mode: GameMode) -> Result<(), PersistError> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    write_atomic(&dir.join("last_mode.txt"), mode_key(mode).as_bytes())?;
    Ok(())
}

/// Loads the last game mode used.
///
/// Returns `None` if the file is missing or corrupted.
#[must_use]
pub fn load_last_mode() -> Option<GameMode> {
    let path = config_dir().join("last_mode.txt");
    match std::fs::read_to_string(path).ok()?.trim() {
        "hvh" => Some(GameMode::HumanVsHuman),
        "hvm" => Some(GameMode::HumanVsEngine),
        "mvm" => Some(GameMode::EngineVsEngine),
        _     => None,
    }
}

fn config_path(mode: GameMode) -> PathBuf {
    config_dir().join(format!("last_{}.json", mode_key(mode)))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Saves the configuration for its game mode.
///
/// Creates the `parametres/parties/` directory if needed.
///
/// # Errors
///
/// [`PersistError::Io`] if creating the directory or writing fails.
/// [`PersistError::Json`] if serialization fails (should not happen).
pub fn save_last_config(config: &GameConfig) -> Result<(), PersistError> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = config_path(config.mode);
    let json = serde_json::to_string_pretty(config)?;
    write_atomic(&path, json.as_bytes())?;
    Ok(())
}

/// Loads the last saved configuration for a given mode.
///
/// Returns `None` if the file is missing or corrupted (not a fatal error).
#[must_use]
pub fn load_last_config(mode: GameMode) -> Option<GameConfig> {
    let path = config_path(mode);
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Indicates whether a saved configuration exists for a given mode.
#[must_use]
pub fn has_last_config(mode: GameMode) -> bool {
    config_path(mode).exists()
}

/// Deletes the saved configuration for a given mode.
///
/// Silent if the file does not exist.
///
/// # Errors
///
/// [`PersistError::Io`] on an unexpected I/O error (permissions, etc.).
pub fn delete_last_config(mode: GameMode) -> Result<(), PersistError> {
    let path = config_path(mode);
    match std::fs::remove_file(path) {
        Ok(())                                                         => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e)                                                         => Err(PersistError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeControl;

    /// Creates an isolated temporary directory for persistence tests.
    fn temp_config_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// Saves to a given directory (bypasses `config_dir()`).
    fn save_to(dir: &std::path::Path, config: &GameConfig) -> Result<(), PersistError> {
        std::fs::create_dir_all(dir)?;
        let key  = mode_key(config.mode);
        let path = dir.join(format!("last_{key}.json"));
        let json = serde_json::to_string_pretty(config)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Loads from a given directory.
    fn load_from(dir: &std::path::Path, mode: GameMode) -> Option<GameConfig> {
        let key  = mode_key(mode);
        let path = dir.join(format!("last_{key}.json"));
        let json = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&json).ok()
    }

    #[test]
    fn test_write_atomic_creates_file_with_content() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("atomic_test.txt");
        write_atomic(&path, b"hello world").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_write_atomic_no_leftover_tmp_file() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("atomic_test2.txt");
        write_atomic(&path, b"content").unwrap();
        // No leftover temporary file in the directory.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["atomic_test2.txt".to_owned()]);
    }

    #[test]
    fn test_write_atomic_overwrites_existing() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("atomic_test3.txt");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn test_mode_keys_are_distinct() {
        assert_ne!(mode_key(GameMode::HumanVsHuman),   mode_key(GameMode::HumanVsEngine));
        assert_ne!(mode_key(GameMode::HumanVsEngine),  mode_key(GameMode::EngineVsEngine));
        assert_ne!(mode_key(GameMode::HumanVsHuman),   mode_key(GameMode::EngineVsEngine));
    }

    #[test]
    fn test_config_dir_is_absolute() {
        let dir = config_dir();
        assert!(dir.is_absolute(), "config_dir doit être un chemin absolu : {dir:?}");
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = temp_config_dir();

        let mut config = GameConfig::human_vs_engine("/usr/bin/stockfish");
        if let Some(e) = config.black_engine.as_mut() {
            e.set_option("Hash", "256");
            e.set_option("Threads", "4");
            e.time_control = TimeControl::MoveTime(2000);
        }

        save_to(tmp.path(), &config).unwrap();
        let loaded = load_from(tmp.path(), GameMode::HumanVsEngine).unwrap();

        assert_eq!(loaded.mode,        config.mode);
        assert_eq!(loaded.human_color, config.human_color);
        let engine = loaded.black_engine.unwrap();
        assert_eq!(engine.path,                  "/usr/bin/stockfish");
        assert_eq!(engine.get_option("Hash"),    Some("256"));
        assert_eq!(engine.get_option("Threads"), Some("4"));
        assert_eq!(engine.time_control,          TimeControl::MoveTime(2000));
    }

    #[test]
    fn test_save_and_load_engine_vs_engine() {
        let tmp = temp_config_dir();

        let mut config = GameConfig::engine_vs_engine("/bin/sf", "/bin/lc0");
        if let Some(e) = config.white_engine.as_mut() {
            e.set_option("Threads", "8");
        }

        save_to(tmp.path(), &config).unwrap();
        let loaded = load_from(tmp.path(), GameMode::EngineVsEngine).unwrap();

        assert_eq!(loaded.white_engine.unwrap().get_option("Threads"), Some("8"));
        assert_eq!(loaded.black_engine.unwrap().path, "/bin/lc0");
    }

    #[test]
    fn test_load_missing_returns_none() {
        let tmp = temp_config_dir();
        // No file saved
        let result = load_from(tmp.path(), GameMode::HumanVsHuman);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_corrupted_returns_none() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("last_hvm.json");
        std::fs::write(&path, b"{ invalid json }").unwrap();
        let result = load_from(tmp.path(), GameMode::HumanVsEngine);
        assert!(result.is_none());
    }

    #[test]
    // Clippy (04/07/2026): `#[allow(similar_names)]` — `loaded_hvh`/`loaded_hvm`
    // deliberately reuse the tested `GameMode` abbreviations (HvH/HvM), not an
    // accidental mix-up.
    #[allow(clippy::similar_names)]
    fn test_modes_are_stored_independently() {
        let tmp = temp_config_dir();

        let hvh = GameConfig::human_vs_human();
        let hvm = GameConfig::human_vs_engine("/bin/engine");

        save_to(tmp.path(), &hvh).unwrap();
        save_to(tmp.path(), &hvm).unwrap();

        let loaded_hvh = load_from(tmp.path(), GameMode::HumanVsHuman).unwrap();
        let loaded_hvm = load_from(tmp.path(), GameMode::HumanVsEngine).unwrap();

        assert_eq!(loaded_hvh.mode, GameMode::HumanVsHuman);
        assert_eq!(loaded_hvm.mode, GameMode::HumanVsEngine);
        assert!(loaded_hvh.white_engine.is_none());
        assert!(loaded_hvm.black_engine.is_some());
    }

    #[test]
    fn test_save_overwrites_previous() {
        let tmp = temp_config_dir();

        let config1 = GameConfig::human_vs_engine("/bin/engine_v1");
        save_to(tmp.path(), &config1).unwrap();

        let config2 = GameConfig::human_vs_engine("/bin/engine_v2");
        save_to(tmp.path(), &config2).unwrap();

        let loaded = load_from(tmp.path(), GameMode::HumanVsEngine).unwrap();
        assert_eq!(loaded.black_engine.unwrap().path, "/bin/engine_v2");
    }

    // ── Real public API, via config_dir() (PHASE 24, Step 4) ──────────
    //
    // The tests above exercise the serialization logic via save_to/load_from
    // on a temporary directory dedicated to each test. These additionally
    // exercise the real public API (save_last_config, load_last_config,
    // save_last_mode, load_last_mode, has_last_config, delete_last_config),
    // which until now had no direct coverage. Since they all share the same
    // test config_dir(), a lock serializes them (same principle as
    // BOOK_TEST_LOCK in gui::prefs).

    static REAL_API_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_real_api_tests() -> std::sync::MutexGuard<'static, ()> {
        REAL_API_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn test_save_and_load_last_config_via_real_api() {
        let _guard = lock_real_api_tests();
        let config = GameConfig::human_vs_engine("/bin/real_api_engine");
        save_last_config(&config).unwrap();
        let loaded = load_last_config(GameMode::HumanVsEngine).unwrap();
        assert_eq!(loaded.black_engine.unwrap().path, "/bin/real_api_engine");
    }

    #[test]
    fn test_has_last_config_true_after_save() {
        let _guard = lock_real_api_tests();
        save_last_config(&GameConfig::human_vs_human()).unwrap();
        assert!(has_last_config(GameMode::HumanVsHuman));
    }

    #[test]
    fn test_delete_last_config_removes_file() {
        let _guard = lock_real_api_tests();
        save_last_config(&GameConfig::engine_vs_engine("/bin/a", "/bin/b")).unwrap();
        assert!(has_last_config(GameMode::EngineVsEngine));
        delete_last_config(GameMode::EngineVsEngine).unwrap();
        assert!(!has_last_config(GameMode::EngineVsEngine));
    }

    #[test]
    fn test_delete_last_config_missing_file_is_noop() {
        let _guard = lock_real_api_tests();
        // Must succeed whether the file already exists or not (see function doc).
        assert!(delete_last_config(GameMode::HumanVsHuman).is_ok());
    }

    #[test]
    fn test_save_and_load_last_mode_via_real_api() {
        let _guard = lock_real_api_tests();
        save_last_mode(GameMode::EngineVsEngine).unwrap();
        assert_eq!(load_last_mode(), Some(GameMode::EngineVsEngine));
    }

    /// `save_last_config` must recreate `parametres/parties/` if it was
    /// deleted in the meantime (consistent with `app_paths::ensure_app_dirs()`,
    /// which already recreates it at startup — belt and suspenders for calls
    /// outside of startup).
    #[test]
    fn test_config_dir_is_recreated_automatically_if_missing() {
        let _guard = lock_real_api_tests();
        let _ = std::fs::remove_dir_all(config_dir());
        assert!(!config_dir().exists());

        save_last_config(&GameConfig::human_vs_human()).unwrap();
        assert!(config_dir().is_dir());
    }
}
