//! Portable application directory layout (PHASE 24 — Step 1).
//!
//! Vendetta Chess ships as a self-contained folder (`VendettaChess/`), with
//! the executable directly at its root. Everything the software needs
//! (settings, `SQLite` database, UCI engines, opening books, logs) must live
//! in subfolders of that same folder, never in a system directory
//! (`~/Library/Application Support`, `~/.config`, `%APPDATA%`) — a necessary
//! condition for a USB drive containing the folder to travel from one
//! computer to another without losing anything.
//!
//! The application folder is recomputed on **every startup** from
//! `std::env::current_exe()` (standard API, already Windows/macOS/Linux
//! compatible): never cached, never stored as an absolute path elsewhere —
//! which allows the whole delivery folder to be moved, renamed, or have its
//! drive letter changed (USB drive) without breaking anything.
//!
//! Directory layout created automatically if missing:
//!
//! | Subfolder                | Intended content                                 |
//! |--------------------------|--------------------------------------------------|
//! | `parametres/`            | Preference files (language, engines, etc.)       |
//! | `parametres/parties/`    | Latest game configurations (HvH/HvM/MvM)         |
//! | `base/`                  | `SQLite` database (`vendetta.db`)                |
//! | `moteurs/`                | UCI engine executables                           |
//! | `ouvertures/`            | Polyglot opening books                           |
//! | `logs/`                  | Diagnostic logs (optional debug mode, Preferences → Misc) |
//! | `bases_parties/`          | `SQLite` database of reference games imported from an external PGN base (PHASE 82) |
//!
//! This Step 1 is limited to exposing `app_dir()`, the subfolder accessors,
//! and `ensure_app_dirs()` — actually redirecting preferences, the database,
//! and engine/book imports to these folders is the subject of later steps
//! (see `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 24).

use std::io;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Application folder
// ---------------------------------------------------------------------------

/// Returns the folder containing the currently running executable — i.e. the
/// root of the `VendettaChess/` delivery folder.
///
/// Recomputed on every call via `std::env::current_exe()`: never cache the
/// result beyond the duration of a single run, to stay correct if the
/// delivery folder was moved between two runs.
#[must_use]
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .map_or_else(|_| PathBuf::from("."), |exe| compute_app_dir(&exe))
}

/// Testable core of [`app_dir`]: the application folder is the parent of the
/// executable's path. Falls back to `.` (current directory) in the
/// degenerate case where the executable would have no parent (root path).
fn compute_app_dir(exe_path: &Path) -> PathBuf {
    exe_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// Subfolders
// ---------------------------------------------------------------------------

/// Preference files folder (`parametres/`).
#[must_use]
pub fn parametres_dir() -> PathBuf { app_dir().join("parametres") }

/// Latest game configurations folder (`parametres/parties/`).
#[must_use]
pub fn parametres_parties_dir() -> PathBuf { parametres_dir().join("parties") }

/// `SQLite` database folder (`base/`).
#[must_use]
pub fn base_dir() -> PathBuf { app_dir().join("base") }

/// UCI engine executables folder (`moteurs/`).
#[must_use]
pub fn moteurs_dir() -> PathBuf { app_dir().join("moteurs") }

/// Polyglot opening books folder (`ouvertures/`).
#[must_use]
pub fn ouvertures_dir() -> PathBuf { app_dir().join("ouvertures") }

/// Fixed path of the White Polyglot book (`ouvertures/blancs.bin`).
///
/// PHASE 24, Step 6: whatever the original name of the file chosen by the
/// user, the imported book is always copied then renamed to this fixed
/// location — the presence of a book is deduced directly from this file's
/// existence, with no separate registry.
#[must_use]
pub fn book_blancs_path() -> PathBuf { ouvertures_dir().join("blancs.bin") }

/// Fixed path of the Black Polyglot book (`ouvertures/noirs.bin`).
/// See [`book_blancs_path`].
#[must_use]
pub fn book_noirs_path() -> PathBuf { ouvertures_dir().join("noirs.bin") }

/// Diagnostic logs folder (`logs/`) — used by the GUI's optional debug mode
/// (`gui::debug_log`, enabled via the persisted "Debug mode" checkbox in
/// Preferences → Misc), inactive by default.
#[must_use]
pub fn logs_dir() -> PathBuf { app_dir().join("logs") }

/// Reference games database folder (`bases_parties/`), PHASE 82.
///
/// Decision made in discussion (09/07/2026, see
/// `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 82, point 1): this database
/// lives in a **separate `SQLite` file** from `base/vendetta.db` (dedicated
/// schema, see `db::reference_schema`), so it stays removable/reimportable
/// independently from the rest of the application and doesn't bloat the
/// application database — hence a dedicated subfolder rather than one more
/// file in `base/`.
#[must_use]
pub fn bases_parties_dir() -> PathBuf { app_dir().join("bases_parties") }

/// Fixed path of the `SQLite` file for the reference games database
/// **imported from PGN** (`bases_parties/reference.db`).
///
/// Like [`book_blancs_path`]/[`book_noirs_path`], the location is fixed:
/// whatever the source PGN file imported by the user, the resulting
/// `SQLite` database always lives here (reimported in place, see
/// `db::reference_schema`).
///
/// Decision made in discussion (11/07/2026): this PGN database and the SCID
/// database ([`reference_scid_db_path`]) live in **two distinct files** —
/// importing one never erases the other. File name unchanged
/// (`reference.db`, no `_pgn` suffix) to stay compatible with a database
/// already imported by an earlier version of the software.
#[must_use]
pub fn reference_pgn_db_path() -> PathBuf { bases_parties_dir().join("reference.db") }

/// Fixed path of the `SQLite` file for the reference games database
/// **imported from SCID** (`.si4`/`.si5`, see `crates/scid`) —
/// (`bases_parties/reference_scid.db`).
///
/// Distinct file from [`reference_pgn_db_path`] (see its documentation): an
/// si4 or si5 import always writes here, regardless of the exact
/// sub-format (si4/si5 share the same SCID database, decision made
/// 11/07/2026).
#[must_use]
pub fn reference_scid_db_path() -> PathBuf { bases_parties_dir().join("reference_scid.db") }

/// List of the seven expected subfolders, relative to `base` — used both by
/// [`ensure_app_dirs`] and by the tests.
fn expected_subdirs(base: &Path) -> [PathBuf; 7] {
    [
        base.join("parametres"),
        base.join("parametres").join("parties"),
        base.join("base"),
        base.join("moteurs"),
        base.join("ouvertures"),
        base.join("logs"),
        base.join("bases_parties"),
    ]
}

// ---------------------------------------------------------------------------
// Relative ↔ absolute path conversion
// ---------------------------------------------------------------------------

/// Converts an absolute path into a representation **relative to
/// `app_dir()`** if the path is actually located under the application
/// folder (e.g. an engine already copied into `moteurs/`) — always
/// represented with `/` (portable), regardless of OS. If the path is
/// outside the application folder (external reference, normal case before
/// import), it is returned **unchanged, absolute**: nothing is lost, it's
/// just not yet "brought back" into the delivery folder.
#[must_use]
pub fn to_relative_string(path: &Path) -> String {
    to_relative_against(&app_dir(), path)
}

/// Converts a stored string (relative or absolute) into an absolute path
/// usable immediately. A relative string is re-joined onto `app_dir()`
/// (recomputed on every call, so always correct even if the delivery folder
/// was moved/renamed). An already-absolute string is returned unchanged —
/// backward compatibility with preferences written before PHASE 24
/// (absolute paths to engines located anywhere on disk).
#[must_use]
pub fn to_absolute_path(stored: &str) -> PathBuf {
    to_absolute_against(&app_dir(), stored)
}

/// Testable core of [`to_relative_string`].
fn to_relative_against(base: &Path, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Testable core of [`to_absolute_path`].
fn to_absolute_against(base: &Path, stored: &str) -> PathBuf {
    let p = Path::new(stored);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

// ---------------------------------------------------------------------------
// Automatic creation
// ---------------------------------------------------------------------------

/// Creates the seven subfolders of the portable layout under [`app_dir`],
/// if missing. Idempotent: does not touch folders already present (nor
/// their content), does not fail if everything already exists.
///
/// Call as early as possible at startup, before any preferences, database,
/// or engine read/write.
///
/// # Errors
/// Returns an I/O error if creating one of the subfolders fails
/// (permissions, read-only filesystem, etc.).
pub fn ensure_app_dirs() -> io::Result<()> {
    ensure_dirs_at(&app_dir())
}

/// Testable core of [`ensure_app_dirs`]: creates the directory layout under
/// `base`.
fn ensure_dirs_at(base: &Path) -> io::Result<()> {
    for dir in expected_subdirs(base) {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Import with automatic renaming (PHASE 24, Step 5 — UCI engines)
// ---------------------------------------------------------------------------

/// Copies `source` into `dest_dir`, automatically renaming on name collision
/// (`stockfish`, `stockfish_2`, `stockfish_3`… — extension preserved,
/// important on Windows so that `stockfish_2.exe` is still recognized as an
/// executable). **Never** modifies or overwrites a file already present.
/// Creates `dest_dir` if it doesn't exist yet. Returns the full path of the
/// copied file.
///
/// # Errors
///
/// Returns an I/O error if `source` has no usable file name, if `dest_dir`
/// cannot be created, or if the copy fails.
pub fn copy_with_auto_rename(source: &Path, dest_dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dest_dir)?;
    let file_name = source.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "chemin source sans nom de fichier")
    })?;
    let dest = unique_dest_path(dest_dir, file_name);
    std::fs::copy(source, &dest)?;
    Ok(dest)
}

/// Copies `source` to `dest` (**exact** path, no renaming), **overwriting**
/// any file already present at `dest`. Creates `dest`'s parent folder if it
/// doesn't exist yet.
///
/// Used for Polyglot books (PHASE 24, Step 6): unlike engines
/// ([`copy_with_auto_rename`]), an imported book always replaces the
/// previous one for its role (White or Black), whatever its original name —
/// no collision to handle, the destination is fixed.
///
/// # Errors
///
/// Returns an I/O error if the parent folder cannot be created or if the
/// copy fails.
pub fn copy_overwrite(source: &Path, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, dest)?;
    Ok(())
}

/// Testable core of [`copy_with_auto_rename`]: computes a destination path
/// that does not yet exist in `dir`, deriving `name`, `name_2`, `name_3`…
/// from `file_name` on collision (extension preserved).
fn unique_dest_path(dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let as_path = Path::new(file_name);
    let stem = as_path.file_stem().and_then(|s| s.to_str()).unwrap_or("fichier");
    let ext = as_path.extension().and_then(|s| s.to_str());

    let mut n = 2;
    loop {
        let name = match ext {
            Some(ext) => format!("{stem}_{n}.{ext}"),
            None => format!("{stem}_{n}"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_app_dir_returns_parent_of_exe() {
        let exe = Path::new("/some/dir/VendettaChess/vendetta_chess_gui");
        assert_eq!(
            compute_app_dir(exe),
            PathBuf::from("/some/dir/VendettaChess")
        );
    }

    #[test]
    fn test_compute_app_dir_windows_style_path() {
        // Path::parent() also works on paths with `\` separators
        // interpreted as plain components under Unix, but at least we
        // check the case of a Unix-style path representative of a real
        // Windows use (the actual executable will be tested by cargo
        // directly under Windows, see the project's validation rules).
        let exe = Path::new("/D/VendettaChess/vendetta_chess_gui.exe");
        assert_eq!(
            compute_app_dir(exe),
            PathBuf::from("/D/VendettaChess")
        );
    }

    #[test]
    fn test_compute_app_dir_fallback_when_exe_has_no_parent() {
        let exe = Path::new("vendetta_chess_gui");
        assert_eq!(compute_app_dir(exe), PathBuf::from(""));
    }

    #[test]
    fn test_ensure_dirs_at_creates_all_seven_missing_dirs() {
        let tmp = tempfile::tempdir().expect("temp dir");
        ensure_dirs_at(tmp.path()).expect("création dossiers");

        for dir in expected_subdirs(tmp.path()) {
            assert!(dir.is_dir(), "dossier manquant : {dir:?}");
        }
    }

    #[test]
    fn test_ensure_dirs_at_is_idempotent_when_dirs_already_exist() {
        let tmp = tempfile::tempdir().expect("temp dir");
        ensure_dirs_at(tmp.path()).expect("premier appel");
        // Second call: must not fail even though everything already exists.
        ensure_dirs_at(tmp.path()).expect("second appel idempotent");

        for dir in expected_subdirs(tmp.path()) {
            assert!(dir.is_dir());
        }
    }

    #[test]
    fn test_ensure_dirs_at_preserves_existing_files_in_dirs() {
        let tmp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(tmp.path().join("base")).expect("précréation");
        let sentinel = tmp.path().join("base").join("vendetta.db");
        std::fs::write(&sentinel, b"donnees existantes").expect("écriture sentinelle");

        ensure_dirs_at(tmp.path()).expect("ne doit pas écraser l'existant");

        let contenu = std::fs::read(&sentinel).expect("lecture sentinelle");
        assert_eq!(contenu, b"donnees existantes");
    }

    #[test]
    fn test_to_relative_against_strips_base_and_uses_forward_slashes() {
        let base = Path::new("/racine/VendettaChess");
        let path = Path::new("/racine/VendettaChess/moteurs/stockfish");
        assert_eq!(to_relative_against(base, path), "moteurs/stockfish");
    }

    #[test]
    fn test_to_relative_against_returns_absolute_unchanged_when_outside_base() {
        let base = Path::new("/racine/VendettaChess");
        let path = Path::new("/ailleurs/stockfish");
        assert_eq!(to_relative_against(base, path), "/ailleurs/stockfish");
    }

    #[test]
    fn test_to_absolute_against_joins_relative_onto_base() {
        let base = Path::new("/racine/VendettaChess");
        assert_eq!(
            to_absolute_against(base, "moteurs/stockfish"),
            PathBuf::from("/racine/VendettaChess/moteurs/stockfish")
        );
    }

    #[test]
    fn test_to_absolute_against_returns_absolute_unchanged() {
        let base = Path::new("/racine/VendettaChess");
        assert_eq!(
            to_absolute_against(base, "/ailleurs/stockfish"),
            PathBuf::from("/ailleurs/stockfish")
        );
    }

    #[test]
    fn test_relative_absolute_roundtrip_when_under_base() {
        let base = Path::new("/racine/VendettaChess");
        let original = Path::new("/racine/VendettaChess/moteurs/sous-dossier/stockfish");
        let rel = to_relative_against(base, original);
        let back = to_absolute_against(base, &rel);
        assert_eq!(back, original);
    }

    #[test]
    fn test_relative_absolute_roundtrip_when_external() {
        let base = Path::new("/racine/VendettaChess");
        let original = Path::new("/ailleurs/quelque_part/stockfish");
        let rel = to_relative_against(base, original);
        let back = to_absolute_against(base, &rel);
        assert_eq!(back, original);
    }

    #[test]
    fn test_copy_with_auto_rename_copies_content() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let dest_dir = tempfile::tempdir().expect("temp dir dest");
        let src_file = src_dir.path().join("stockfish");
        std::fs::write(&src_file, b"binaire factice").expect("écriture source");

        let dest = copy_with_auto_rename(&src_file, dest_dir.path()).expect("copie");

        assert_eq!(dest, dest_dir.path().join("stockfish"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"binaire factice");
    }

    #[test]
    fn test_copy_with_auto_rename_creates_dest_dir_if_missing() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let parent = tempfile::tempdir().expect("temp dir parent");
        let dest_dir = parent.path().join("moteurs_pas_encore_cree");
        let src_file = src_dir.path().join("stockfish");
        std::fs::write(&src_file, b"x").expect("écriture source");

        assert!(!dest_dir.exists());
        let dest = copy_with_auto_rename(&src_file, &dest_dir).expect("copie");
        assert!(dest.exists());
    }

    #[test]
    fn test_copy_with_auto_rename_renames_on_collision_preserving_extension() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let dest_dir = tempfile::tempdir().expect("temp dir dest");

        // A first "stockfish.exe" already exists in the destination folder.
        std::fs::write(dest_dir.path().join("stockfish.exe"), b"ancien").expect("préexistant");

        let src_file = src_dir.path().join("stockfish.exe");
        std::fs::write(&src_file, b"nouveau").expect("écriture source");

        let dest = copy_with_auto_rename(&src_file, dest_dir.path()).expect("copie");

        assert_eq!(dest, dest_dir.path().join("stockfish_2.exe"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"nouveau");
        // The previous file was not touched.
        assert_eq!(
            std::fs::read(dest_dir.path().join("stockfish.exe")).unwrap(),
            b"ancien"
        );
    }

    #[test]
    fn test_copy_with_auto_rename_increments_past_multiple_collisions() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let dest_dir = tempfile::tempdir().expect("temp dir dest");

        std::fs::write(dest_dir.path().join("moteur"), b"1").expect("collision 1");
        std::fs::write(dest_dir.path().join("moteur_2"), b"2").expect("collision 2");
        std::fs::write(dest_dir.path().join("moteur_3"), b"3").expect("collision 3");

        let src_file = src_dir.path().join("moteur");
        std::fs::write(&src_file, b"nouveau").expect("écriture source");

        let dest = copy_with_auto_rename(&src_file, dest_dir.path()).expect("copie");
        assert_eq!(dest, dest_dir.path().join("moteur_4"));
    }

    #[test]
    fn test_copy_overwrite_copies_content() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let dest_dir = tempfile::tempdir().expect("temp dir dest");
        let src_file = src_dir.path().join("perfect2023.bin");
        std::fs::write(&src_file, b"book polyglot").expect("écriture source");
        let dest = dest_dir.path().join("blancs.bin");

        copy_overwrite(&src_file, &dest).expect("copie");
        assert_eq!(std::fs::read(&dest).unwrap(), b"book polyglot");
    }

    #[test]
    fn test_copy_overwrite_creates_dest_parent_dir_if_missing() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let parent = tempfile::tempdir().expect("temp dir parent");
        let dest = parent.path().join("ouvertures_pas_encore_cree").join("blancs.bin");
        let src_file = src_dir.path().join("book.bin");
        std::fs::write(&src_file, b"x").expect("écriture source");

        assert!(!dest.parent().unwrap().exists());
        copy_overwrite(&src_file, &dest).expect("copie");
        assert!(dest.exists());
    }

    #[test]
    fn test_copy_overwrite_replaces_previous_file_at_same_dest() {
        let src_dir = tempfile::tempdir().expect("temp dir source");
        let dest_dir = tempfile::tempdir().expect("temp dir dest");
        let dest = dest_dir.path().join("blancs.bin");
        std::fs::write(&dest, b"ancien book").expect("préexistant");

        let src_file = src_dir.path().join("nouveau_nom_quelconque.bin");
        std::fs::write(&src_file, b"nouveau book").expect("écriture source");

        copy_overwrite(&src_file, &dest).expect("copie");
        assert_eq!(std::fs::read(&dest).unwrap(), b"nouveau book");
    }

    #[test]
    fn test_unique_dest_path_no_collision_returns_plain_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = unique_dest_path(dir.path(), std::ffi::OsStr::new("stockfish"));
        assert_eq!(path, dir.path().join("stockfish"));
    }

    #[test]
    fn test_unique_dest_path_no_extension_collision() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("moteur"), b"x").unwrap();
        let path = unique_dest_path(dir.path(), std::ffi::OsStr::new("moteur"));
        assert_eq!(path, dir.path().join("moteur_2"));
    }

    #[test]
    fn test_subdir_accessors_are_relative_to_given_base() {
        let base = Path::new("/racine/VendettaChess");
        let dirs = expected_subdirs(base);
        assert_eq!(dirs[0], base.join("parametres"));
        assert_eq!(dirs[1], base.join("parametres").join("parties"));
        assert_eq!(dirs[2], base.join("base"));
        assert_eq!(dirs[3], base.join("moteurs"));
        assert_eq!(dirs[4], base.join("ouvertures"));
        assert_eq!(dirs[5], base.join("logs"));
        assert_eq!(dirs[6], base.join("bases_parties"));
    }

    #[test]
    fn test_bases_parties_dir_is_subdir_of_app_dir() {
        // Cannot compare directly to app_dir() (depends on the test
        // executable), but at least we check the terminal subfolder name.
        assert_eq!(bases_parties_dir().file_name().unwrap(), "bases_parties");
    }

    #[test]
    fn test_reference_pgn_db_path_is_inside_bases_parties_dir() {
        assert_eq!(reference_pgn_db_path(), bases_parties_dir().join("reference.db"));
    }

    #[test]
    fn test_reference_scid_db_path_is_inside_bases_parties_dir() {
        assert_eq!(reference_scid_db_path(), bases_parties_dir().join("reference_scid.db"));
    }

    #[test]
    fn test_reference_pgn_and_scid_db_paths_differ() {
        assert_ne!(reference_pgn_db_path(), reference_scid_db_path());
    }
}
