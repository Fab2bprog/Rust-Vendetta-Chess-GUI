//! PHASE 24, Step 7 — automatic scan of `moteurs/` at startup.
//!
//! Detects the files present in `moteurs/` that are not yet listed
//! in the registered engines list (`engines.json`), validates them via a
//! silent UCI handshake (no dialog, no trace on failure —
//! decision settled with the user: a `moteurs/` folder can legitimately
//! contain non-executable companion files, e.g. NNUE weights, DLLs), and
//! returns the valid engines as [`prefs::SavedEngine`], named
//! after their UCI `id name` response (falls back to the file name if absent
//! — same rule as the manual add, see `on_browse_add_engine` in
//! `main.rs`).
//!
//! # A single level of subfolders
//!
//! Decision settled (PHASE 24): a direct subfolder of `moteurs/` is
//! scanned entirely — all its files are tested one by one, because the
//! actual executable cannot be identified by its name or its
//! extension alone (e.g. `stockfish`, `stockfish.exe`, a companion DLL, an
//! NNUE weight file can coexist in the same subfolder). On the
//! other hand, the subfolders of that subfolder are never explored:
//! this avoids name collisions between engines while keeping the scan
//! bounded.
//!
//! This module deliberately makes no assumption about the calling
//! execution thread: [`validate_candidates`] is meant to run on a
//! background thread (see `main.rs`), the main thread must never be
//! delayed at startup by a slow or stuck UCI handshake.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::prefs;

/// UCI handshake timeout used in production for each candidate.
const SCAN_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// Lists the files of `moteurs_dir` that are not already in
/// `known_paths`, going down a single level into any
/// subfolders (see the module documentation).
///
/// Pure function: no UCI validation here, only an enumeration of
/// candidate files. Returns a sorted vector (deterministic for tests
/// and for any display). If `moteurs_dir` does not yet exist,
/// simply returns an empty vector (no error: nothing to scan).
#[must_use]
pub fn list_scan_candidates(moteurs_dir: &Path, known_paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(moteurs_dir) else { return out; };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // A single level: we list the files of the subfolder, but
            // never explore its own subfolders.
            let Ok(sub_entries) = std::fs::read_dir(&path) else { continue; };
            for sub in sub_entries.flatten() {
                let sub_path = sub.path();
                if sub_path.is_file() {
                    push_if_unknown(&mut out, sub_path, known_paths);
                }
            }
        } else if path.is_file() {
            push_if_unknown(&mut out, path, known_paths);
        }
    }

    out.sort();
    out
}

/// PHASE 71 — comparison via `Path` (component by component), not via
/// raw `String` equality on `to_string_lossy()`.
///
/// Bug fixed (Windows only, never observed on macOS/Linux): the
/// paths of `known_paths` come from `prefs::load_engines()`, which
/// rejoins a relative path stored with forced `/` (see
/// `app_paths::to_relative_string`, to remain portable if the delivery
/// folder is moved) onto `app_dir()` via `PathBuf::join`. But `join`
/// only adds a native separator at the joining point — it never rewrites
/// the `/` already present inside the rejoined string. On
/// Windows, the resulting "known" path therefore contains a mix of `\`/`/` (e.g.
/// `C:\...\moteurs/stockfish.exe`), while the path freshly scanned
/// via `std::fs::read_dir` (this function) is entirely in `\`. The
/// two `String`s then differ even though the file is identical: the engine
/// is seen as "new" on every startup and reimported twice, three times,
/// etc. `Path`/`PathBuf` compares by components (the standard
/// library's Windows parser recognizes `/` and `\` as equivalent
/// separators during parsing), and is therefore insensitive to this mix —
/// unlike a raw `String` comparison.
fn push_if_unknown(out: &mut Vec<PathBuf>, path: PathBuf, known_paths: &[String]) {
    let is_known = known_paths.iter().any(|k| Path::new(k) == path);
    if !is_known {
        out.push(path);
    }
}

/// Validates each candidate file via a UCI handshake and returns the
/// engines recognized as such, ready to be added to the list of saved
/// engines. Fixed production timeout ([`SCAN_HANDSHAKE_TIMEOUT`]).
///
/// Meant to be called from a background thread (see `main.rs`):
/// each invalid candidate causes a wait of up to `SCAN_HANDSHAKE_TIMEOUT`
/// before being discarded, which can amount to several seconds in total
/// if `moteurs/` contains many non-executable companion files.
#[must_use]
pub fn validate_candidates(candidates: Vec<PathBuf>) -> Vec<prefs::SavedEngine> {
    validate_candidates_with_timeout(candidates, SCAN_HANDSHAKE_TIMEOUT)
}

/// Testable core of [`validate_candidates`]: explicit timeout so as not to
/// slow down the tests with invalid candidates (the failure of a UCI
/// handshake is only observed after the wait delay expires).
fn validate_candidates_with_timeout(
    candidates: Vec<PathBuf>,
    timeout: Duration,
) -> Vec<prefs::SavedEngine> {
    use uci::engine::UciEngine;

    let mut found = Vec::new();
    for path in candidates {
        let path_str = path.to_string_lossy().into_owned();
        if let Ok(engine) = UciEngine::connect_with_timeout(&path_str, timeout) {
            let name = engine.name().map_or_else(|| {
                path.file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("moteur")
                    .to_owned()
            }, str::to_owned);
            engine.quit();
            found.push(prefs::SavedEngine {
                name,
                path: path_str,
                options: HashMap::new(),
            });
        }
        // UCI handshake failure (non-executable file, not a UCI engine,
        // timeout...) → candidate silently ignored, with no dialog or
        // trace: decision settled for Step 7.
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    // ── list_scan_candidates ─────────────────────────────────────────────

    #[test]
    fn test_list_scan_candidates_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_missing_dir");
        let _ = std::fs::remove_dir_all(&dir); // make sure it doesn't exist
        assert!(list_scan_candidates(&dir, &[]).is_empty());
    }

    #[test]
    fn test_list_scan_candidates_flat_files() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_flat_files");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&dir.join("stockfish"), b"fake");
        write_file(&dir.join("komodo"), b"fake");

        let found = list_scan_candidates(&dir, &[]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&dir.join("komodo")));
        assert!(found.contains(&dir.join("stockfish")));
    }

    #[test]
    fn test_list_scan_candidates_excludes_known_paths() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_excludes_known");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&dir.join("stockfish"), b"fake");
        write_file(&dir.join("komodo"), b"fake");

        let known = vec![dir.join("stockfish").to_string_lossy().into_owned()];
        let found = list_scan_candidates(&dir, &known);
        assert_eq!(found, vec![dir.join("komodo")]);
    }

    /// PHASE 71 — regression test for the Windows duplicate-detection bug: a
    /// "known" path rebuilt from `engines.json` can contain a residual `/`
    /// within an otherwise native `\` path (resolved
    /// via `PathBuf::join` of a relative string stored with forced `/`
    /// — see `app_paths::to_relative_string`). The file must
    /// not be re-detected as "new" despite this mix of separators.
    /// Gated `cfg(windows)`: the symptom only exists when the
    /// native separator differs from `/` (never the case on macOS/Linux, see
    /// the comment of `push_if_unknown`); therefore does not run in CI
    /// (`ubuntu-latest`), to be validated manually on Windows.
    #[cfg(windows)]
    #[test]
    fn test_list_scan_candidates_tolerates_mixed_separators_windows() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_win_sep_mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&dir.join("stockfish.exe"), b"fake");

        // Simulates the result of `base.join(Path::new("moteurs/stockfish.exe"))`:
        // a literal `/` survives just before the file name.
        let known_mixed = format!("{}/stockfish.exe", dir.to_string_lossy());
        let found = list_scan_candidates(&dir, &[known_mixed]);
        assert!(
            found.is_empty(),
            "le moteur déjà connu ne doit pas être re-détecté malgré le séparateur mélangé"
        );
    }

    #[test]
    fn test_list_scan_candidates_scans_one_level_of_subdirectory() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_one_level");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&dir.join("lc0").join("lc0.exe"), b"fake");
        write_file(&dir.join("lc0").join("weights.pb.gz"), b"fake");

        let found = list_scan_candidates(&dir, &[]);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&dir.join("lc0").join("lc0.exe")));
        assert!(found.contains(&dir.join("lc0").join("weights.pb.gz")));
    }

    #[test]
    fn test_list_scan_candidates_never_explores_second_level_subdirectory() {
        let dir = std::env::temp_dir().join("vendetta_engine_scan_test_two_levels");
        let _ = std::fs::remove_dir_all(&dir);
        write_file(&dir.join("lc0").join("nested").join("ignored.bin"), b"fake");
        write_file(&dir.join("lc0").join("lc0.exe"), b"fake");

        let found = list_scan_candidates(&dir, &[]);
        // Only the file at the first level of the subfolder is kept; the
        // deeper file ("nested/ignored.bin") is explicitly
        // ignored (decision settled: no recursion beyond one level).
        assert_eq!(found, vec![dir.join("lc0").join("lc0.exe")]);
    }

    // ── validate_candidates ──────────────────────────────────────────────

    #[test]
    fn test_validate_candidates_empty_input_returns_empty() {
        assert!(validate_candidates_with_timeout(vec![], Duration::from_millis(200)).is_empty());
    }

    #[test]
    fn test_validate_candidates_skips_nonexistent_file() {
        let path = PathBuf::from("/chemin/totalement/inexistant/vendetta_test_engine_scan");
        let found = validate_candidates_with_timeout(vec![path], Duration::from_millis(200));
        assert!(found.is_empty());
    }

    #[cfg(unix)]
    fn real_cat_path() -> Option<PathBuf> {
        ["/bin/cat", "/usr/bin/cat"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_candidates_skips_non_uci_executable() {
        let Some(cat) = real_cat_path() else { return }; // binary absent from CI — skip
        let found = validate_candidates_with_timeout(vec![cat], Duration::from_millis(300));
        assert!(found.is_empty());
    }

    /// Writes a fake UCI engine (shell script) that responds `uciok` (with or
    /// without `id name`) — same principle as `mock_engine_script` in
    /// `crates/uci/src/engine.rs`, reproduced here locally so as not to
    /// depend on another crate's internal test code.
    #[cfg(unix)]
    fn write_mock_uci_script(path: &Path, id_name_line: Option<&str>) {
        use std::os::unix::fs::PermissionsExt;
        let id_line = id_name_line
            .map(|n| format!("echo 'id name {n}'; "))
            .unwrap_or_default();
        let script = format!(
            "#!/bin/sh\nwhile read -r line; do\n  case \"$line\" in\n    uci) {id_line}echo 'uciok' ;;\n    isready) echo 'readyok' ;;\n    quit) exit 0 ;;\n  esac\ndone\n"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, script).unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_candidates_uses_uci_id_name_when_present() {
        let path = std::env::temp_dir().join("vendetta_engine_scan_test_mock_named.sh");
        write_mock_uci_script(&path, Some("MockEngine Test"));

        let found = validate_candidates_with_timeout(vec![path.clone()], Duration::from_secs(3));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "MockEngine Test");
        assert_eq!(found[0].path, path.to_string_lossy().into_owned());
        assert!(found[0].options.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_candidates_falls_back_to_filename_when_no_id_name() {
        let path = std::env::temp_dir().join("vendetta_mock_unnamed_engine_scan_test.sh");
        write_mock_uci_script(&path, None);

        let found = validate_candidates_with_timeout(vec![path.clone()], Duration::from_secs(3));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "vendetta_mock_unnamed_engine_scan_test");
    }
}
