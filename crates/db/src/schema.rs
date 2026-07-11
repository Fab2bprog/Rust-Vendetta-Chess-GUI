//! `SQLite` schema and versioned migration system.
//!
//! ## Tables
//!
//! | Table             | Description                                      |
//! |-------------------|--------------------------------------------------|
//! | `migrations`      | Tracking of applied migrations                   |
//! | `tournaments`     | Tournaments (name, date, type)                       |
//! | `games`           | Games (players, result, PGN, optional tournament) |
//! | `positions`       | Unique positions indexed by FEN               |
//! | `analyses`        | UCI analyses linked to a position                |
//! | `puzzles`         | Tactical puzzles imported by the user (PHASE 14) |
//! | `puzzle_progress` | Progress (solved/attempted) per puzzle (PHASE 14) |
//!
//! ## Migrations
//!
//! Each migration is a numbered SQL block. [`run_migrations`] applies them
//! in order, skipping those already recorded in `migrations`.
//! This allows adding future migrations without touching the existing ones.

use rusqlite::{Connection, Result as SqlResult};

// ---------------------------------------------------------------------------
// Migrations
// ---------------------------------------------------------------------------

/// A versioned migration.
struct Migration {
    version: u32,
    sql:     &'static str,
}

/// List of all migrations, in increasing version order.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "
            -- Migration tracking table
            CREATE TABLE IF NOT EXISTS migrations (
                version     INTEGER PRIMARY KEY,
                applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Tournaments
            CREATE TABLE IF NOT EXISTS tournaments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL,
                site        TEXT,
                date        TEXT,
                kind        TEXT    NOT NULL DEFAULT 'roundrobin',
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            -- Games
            CREATE TABLE IF NOT EXISTS games (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                tournament_id   INTEGER REFERENCES tournaments(id) ON DELETE SET NULL,
                white           TEXT    NOT NULL DEFAULT '?',
                black           TEXT    NOT NULL DEFAULT '?',
                result          TEXT    NOT NULL DEFAULT '*',
                date            TEXT,
                event           TEXT,
                site            TEXT,
                round           TEXT,
                pgn             TEXT    NOT NULL,
                initial_fen     TEXT    NOT NULL DEFAULT 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
                move_count      INTEGER NOT NULL DEFAULT 0,
                created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            -- Index for search by player
            CREATE INDEX IF NOT EXISTS idx_games_white  ON games(white);
            CREATE INDEX IF NOT EXISTS idx_games_black  ON games(black);
            CREATE INDEX IF NOT EXISTS idx_games_result ON games(result);

            -- Positions (deduplicated by FEN)
            CREATE TABLE IF NOT EXISTS positions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                fen         TEXT    NOT NULL UNIQUE,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_positions_fen ON positions(fen);

            -- UCI analyses linked to a position
            CREATE TABLE IF NOT EXISTS analyses (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                position_id INTEGER NOT NULL REFERENCES positions(id) ON DELETE CASCADE,
                engine      TEXT    NOT NULL,
                depth       INTEGER NOT NULL,
                score_cp    INTEGER,          -- NULL if score is a mate score
                score_mate  INTEGER,          -- NULL if score is in centipawns
                best_move   TEXT    NOT NULL,
                pv          TEXT    NOT NULL DEFAULT '',
                nodes       INTEGER,
                time_ms     INTEGER,
                multipv     INTEGER NOT NULL DEFAULT 1,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_analyses_position ON analyses(position_id);
            CREATE INDEX IF NOT EXISTS idx_analyses_engine   ON analyses(engine);
        ",
    },
    Migration {
        // `game_repo::find_by_player` filters on `lower(white)`/`lower(black)`
        // (case-insensitive search): the plain indexes on `white`/
        // `black` cannot be used by SQLite for an expression
        // wrapped in `lower()`. Dedicated expression indexes
        // let SQLite avoid a full scan of the `games` table
        // for this query (perf audit 02/07/2026, point 5).
        version: 2,
        sql: "
            CREATE INDEX IF NOT EXISTS idx_games_white_lower ON games(lower(white));
            CREATE INDEX IF NOT EXISTS idx_games_black_lower ON games(lower(black));
        ",
    },
    Migration {
        // PHASE 14 — Puzzles / Training (03/07/2026).
        //
        // `puzzles` reuses as-is the columns of a Lichess Puzzles-style CSV
        // export provided by the user (no database ships
        // with the software, same principle as the Polyglot opening
        // books). `puzzle_id` is the external identifier from the source file
        // (e.g. "00008" on Lichess) — UNIQUE so that reimporting the same
        // file (or a file that overlaps it) does not insert duplicates.
        //
        // `puzzle_progress` is a separate table so as to never touch the
        // imported raw data: `times_attempted` is only incremented
        // on failure or success (never on an abandonment with no wrong move
        // attempted — counting rule validated with the user), `times_solved` on
        // success. Cascade delete if the source puzzle is deleted.
        version: 3,
        sql: "
            CREATE TABLE IF NOT EXISTS puzzles (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                puzzle_id         TEXT    NOT NULL UNIQUE,
                fen               TEXT    NOT NULL,
                moves             TEXT    NOT NULL,
                rating            INTEGER NOT NULL,
                rating_deviation  INTEGER,
                popularity        INTEGER,
                nb_plays          INTEGER,
                themes            TEXT    NOT NULL DEFAULT '',
                game_url          TEXT,
                opening_tags      TEXT,
                created_at        TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles(rating);

            CREATE TABLE IF NOT EXISTS puzzle_progress (
                puzzle_id       INTEGER PRIMARY KEY REFERENCES puzzles(id) ON DELETE CASCADE,
                times_attempted INTEGER NOT NULL DEFAULT 0,
                times_solved    INTEGER NOT NULL DEFAULT 0,
                last_result     TEXT,
                last_seen_at    TEXT    NOT NULL DEFAULT (datetime('now'))
            );
        ",
    },
];

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Opens (or creates) a `SQLite` database at `path` and applies all migrations.
///
/// # Errors
///
/// Returns a `SQLite` error if opening or a migration fails.
pub fn open_and_migrate(path: &str) -> SqlResult<Connection> {
    let conn = Connection::open(path)?;
    configure_pragmas(&conn)?;
    run_migrations(&conn)?;
    Ok(conn)
}

/// Opens a **in-memory** `SQLite` database (for tests).
///
/// Enables `PRAGMA foreign_keys=ON` like [`open_and_migrate`], so that
/// behavior (in particular foreign key constraints and delete
/// cascades) is identical between tests and production — without this
/// setting, an in-memory connection would silently accept rows
/// with an invalid foreign key (e.g. a nonexistent `analyses.position_id`).
/// The journal mode, on the other hand, is not relevant for an in-memory
/// database (not persisted, single process).
///
/// # Errors
///
/// Returns a `SQLite` error if initialization fails.
pub fn open_in_memory() -> SqlResult<Connection> {
    let conn = Connection::open_in_memory()?;
    enable_foreign_keys(&conn)?;
    run_migrations(&conn)?;
    Ok(conn)
}

/// Configures the connection for a persistent database file: classic/rollback
/// journal mode (`DELETE`, `SQLite`'s historical default) and
/// foreign key constraints.
///
/// PHASE 24 (100% portability, USB): WAL mode was deliberately
/// **abandoned** in favor of the classic mode. WAL creates two companion
/// files (`-wal`, `-shm`) alongside the main file and behaves poorly
/// on removable media formatted as exFAT/FAT32 — a common format for a USB
/// stick read by both Windows and macOS. `DELETE` mode is explicitly
/// forced (not just "not enabled") because this setting is persisted **in the
/// database file itself**: a database created before this change would remain
/// in WAL mode on reopening if it were not explicitly reset here.
/// The loss of read concurrency is deemed to have no impact (single-user,
/// single-process usage).
fn configure_pragmas(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA foreign_keys=ON;")?;
    Ok(())
}

/// Enables foreign key constraints (`PRAGMA foreign_keys=ON`).
///
/// Disabled by default by `SQLite` for every new connection — must
/// be reapplied explicitly on each `Connection::open*`.
fn enable_foreign_keys(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(())
}

/// Applies all migrations not yet recorded.
///
/// # Errors
/// Returns a `SQLite` error if creating the `migrations` table,
/// running a migration, or recording its version fails.
pub fn run_migrations(conn: &Connection) -> SqlResult<()> {
    // Create the migrations table if it does not exist yet
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            version     INTEGER PRIMARY KEY,
            applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    for migration in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM migrations WHERE version = ?1",
            [migration.version],
            |row| row.get(0),
        )?;

        if !already_applied {
            conn.execute_batch(migration.sql)?;
            conn.execute(
                "INSERT INTO migrations (version) VALUES (?1)",
                [migration.version],
            )?;
        }
    }

    Ok(())
}

/// Returns the most recent applied migration version.
///
/// Returns `0` if no migration has been applied yet.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn current_version(conn: &Connection) -> SqlResult<u32> {
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let conn = open_in_memory().unwrap();
        assert!(current_version(&conn).unwrap() >= 1);
    }

    /// PHASE 24 (USB portability): a real database file must be opened
    /// in classic journal mode (`DELETE`), never WAL — WAL creates
    /// fragile companion `-wal`/`-shm` files on exFAT/FAT32.
    #[test]
    fn test_open_and_migrate_uses_delete_journal_mode_not_wal() {
        let tmp = tempfile::NamedTempFile::new().expect("fichier temporaire");
        let path = tmp.path().to_str().expect("chemin UTF-8");

        let conn = open_and_migrate(path).expect("ouverture");
        let mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("lecture pragma journal_mode");

        assert_eq!(mode.to_lowercase(), "delete");
    }

    /// A database previously created in WAL mode (before PHASE 24) must be
    /// brought back to `DELETE` mode on reopening, since this setting is
    /// persisted in the file itself and not recomputed on every connection.
    #[test]
    fn test_open_and_migrate_resets_pre_existing_wal_database_to_delete() {
        let tmp = tempfile::NamedTempFile::new().expect("fichier temporaire");
        let path = tmp.path().to_str().expect("chemin UTF-8");

        // Simulates an existing database created in WAL (pre-PHASE 24 behavior).
        {
            let conn = Connection::open(path).expect("ouverture initiale");
            conn.execute_batch("PRAGMA journal_mode=WAL;")
                .expect("forcer WAL");
        }

        let conn = open_and_migrate(path).expect("réouverture via open_and_migrate");
        let mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("lecture pragma journal_mode");

        assert_eq!(mode.to_lowercase(), "delete");
    }

    #[test]
    fn test_open_and_migrate_enables_foreign_keys_on_real_file() {
        let tmp = tempfile::NamedTempFile::new().expect("fichier temporaire");
        let path = tmp.path().to_str().expect("chemin UTF-8");

        let conn = open_and_migrate(path).expect("ouverture");
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .expect("lecture pragma foreign_keys");

        assert_eq!(fk, 1);
    }

    #[test]
    fn test_migrations_applied() {
        let conn = open_in_memory().unwrap();
        let version = current_version(&conn).unwrap();
        assert_eq!(version, MIGRATIONS.last().unwrap().version);
    }

    #[test]
    fn test_migrations_idempotent() {
        let conn = open_in_memory().unwrap();
        // Applying twice must not fail
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 3);
    }

    #[test]
    fn test_tables_exist() {
        let conn = open_in_memory().unwrap();
        let tables = [
            "migrations", "tournaments", "games", "positions", "analyses",
            "puzzles", "puzzle_progress",
        ];
        for table in &tables {
            let count: u32 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "Table manquante : {table}");
        }
    }

    #[test]
    fn test_indexes_exist() {
        let conn = open_in_memory().unwrap();
        let indexes = [
            "idx_games_white",
            "idx_games_black",
            "idx_games_result",
            "idx_positions_fen",
            "idx_analyses_position",
            "idx_analyses_engine",
            "idx_games_white_lower",
            "idx_games_black_lower",
            "idx_puzzles_rating",
        ];
        for idx in &indexes {
            let count: u32 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "Index manquant : {idx}");
        }
    }

    #[test]
    fn test_games_table_schema() {
        let conn = open_in_memory().unwrap();
        // Verifies that a minimal game can be inserted
        conn.execute(
            "INSERT INTO games (white, black, result, pgn) VALUES (?1, ?2, ?3, ?4)",
            ["Alice", "Bob", "1-0", "[Event \"Test\"]\n\n1. e4 1-0"],
        )
        .unwrap();
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_positions_unique_constraint() {
        let conn = open_in_memory().unwrap();
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        conn.execute("INSERT INTO positions (fen) VALUES (?1)", [fen]).unwrap();
        // Second insertion of the same FEN → UNIQUE constraint error
        let result = conn.execute("INSERT INTO positions (fen) VALUES (?1)", [fen]);
        assert!(result.is_err());
    }

    #[test]
    fn test_analyses_foreign_key() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // Analysis without an existing position → foreign key error
        let result = conn.execute(
            "INSERT INTO analyses (position_id, engine, depth, best_move) VALUES (999, 'test', 10, 'e2e4')",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_tournament_game_relation() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO tournaments (name) VALUES (?1)",
            ["Test Tournament"],
        )
        .unwrap();
        let tid: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO games (tournament_id, white, black, result, pgn) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![tid, "Alice", "Bob", "1/2-1/2", ""],
        )
        .unwrap();

        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM games WHERE tournament_id = ?1",
                [tid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── PHASE 14 — Puzzles ───────────────────────────────────────────────────

    fn insert_test_puzzle(conn: &Connection, puzzle_id: &str) -> i64 {
        conn.execute(
            "INSERT INTO puzzles (puzzle_id, fen, moves, rating, themes)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                puzzle_id,
                "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
                "e8g8 f3e5 c6e5 c4f7",
                1450,
                "fork middlegame",
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn test_puzzles_table_schema() {
        let conn = open_in_memory().unwrap();
        insert_test_puzzle(&conn, "test001");
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM puzzles", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_puzzles_puzzle_id_unique_constraint() {
        let conn = open_in_memory().unwrap();
        insert_test_puzzle(&conn, "dup001");
        let result = conn.execute(
            "INSERT INTO puzzles (puzzle_id, fen, moves, rating) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["dup001", "8/8/8/8/8/8/8/8 w - - 0 1", "e2e4", 1000],
        );
        assert!(result.is_err(), "puzzle_id dupliqué doit être rejeté");
    }

    #[test]
    fn test_puzzle_progress_foreign_key() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // Nonexistent puzzle_id → foreign key error
        let result = conn.execute(
            "INSERT INTO puzzle_progress (puzzle_id, times_attempted) VALUES (999, 1)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_puzzle_progress_defaults_and_update() {
        let conn = open_in_memory().unwrap();
        let pid = insert_test_puzzle(&conn, "test002");
        conn.execute(
            "INSERT INTO puzzle_progress (puzzle_id) VALUES (?1)",
            [pid],
        )
        .unwrap();

        let (attempted, solved): (i64, i64) = conn
            .query_row(
                "SELECT times_attempted, times_solved FROM puzzle_progress WHERE puzzle_id = ?1",
                [pid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempted, 0);
        assert_eq!(solved, 0);

        conn.execute(
            "UPDATE puzzle_progress SET times_attempted = times_attempted + 1,
                                        times_solved    = times_solved + 1,
                                        last_result     = 'solved'
             WHERE puzzle_id = ?1",
            [pid],
        )
        .unwrap();
        let last_result: String = conn
            .query_row(
                "SELECT last_result FROM puzzle_progress WHERE puzzle_id = ?1",
                [pid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_result, "solved");
    }

    #[test]
    fn test_puzzle_progress_cascade_delete() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let pid = insert_test_puzzle(&conn, "test003");
        conn.execute("INSERT INTO puzzle_progress (puzzle_id) VALUES (?1)", [pid])
            .unwrap();

        conn.execute("DELETE FROM puzzles WHERE id = ?1", [pid]).unwrap();

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM puzzle_progress", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "la suppression du puzzle doit cascader sur puzzle_progress");
    }
}
