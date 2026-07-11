//! `SQLite` schema for the reference games database (PHASE 82).
//!
//! Unlike [`crate::schema`] (application database: tournaments, puzzles,
//! engine analyses), this database lives in a **separate `SQLite` file**,
//! dedicated exclusively to games imported from an external PGN database
//! (e.g. Lumbra's Gigabase). Decision settled during discussion on 09/07/2026 (see
//! `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 82, point 1): keep the
//! application database lightweight and allow deleting/reimporting the
//! games database independently of the rest — hence a separate schema module,
//! with its own migrations, rather than an extension of `schema.rs`.
//!
//! ## Tables
//!
//! | Table            | Description                                                     |
//! |------------------|-------------------------------------------------------------------|
//! | `migrations`     | Tracking of applied migrations (same principle as [`crate::schema`]) |
//! | `games`          | Imported games, enriched metadata (ECO, Elo, FIDE titles)  |
//! | `game_positions` | One row per (game, half-move) up to [`OPENING_TREE_MAX_PLIES`], basis of the opening tree |
//!
//! ## Why no pre-aggregated table for the opening tree
//!
//! The minimum Elo threshold filter must remain **adjustable** by the user
//! without reimporting the database (decision settled during discussion). A
//! pre-aggregated table (position → move → counters) would freeze this filter at
//! import time. `game_positions` therefore stores a **raw** row per half-move
//! (up to the depth limit), linked to `games` by `game_id`:
//! aggregation by position (number of games, score per next move) will
//! be done via a `SQL` `GROUP BY` query at lookup time, with the
//! Elo threshold applied in the `WHERE` clause of that query (join
//! on `games.white_elo`/`games.black_elo`) — see the future opening tree
//! query module, not yet written at this stage.
//!
//! ## Indexing depth
//!
//! [`OPENING_TREE_MAX_PLIES`] limits the number of half-moves indexed per
//! game (decision settled: strict opening, ~20-30 half-moves, not the
//! whole game) — beyond that, useful transpositions become rare and the
//! volume of rows would explode without real benefit for an opening
//! tree (see PHASE 82 discussion).
//!
//! ## Position hash storage
//!
//! [`core::polyglot::polyglot_hash`] returns a `u64`, but `SQLite`'s
//! `INTEGER` columns (via `rusqlite`) are signed `i64`. The hash is
//! never used arithmetically (only compared for equality), so
//! [`hash_to_sql`]/[`hash_from_sql`] do a simple reversible *bit-cast*
//! (`as i64`/`as u64`) — no loss of information, just a
//! reinterpretation of the same bit pattern.

use rusqlite::{Connection, Result as SqlResult};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of half-moves indexed per game in `game_positions`.
///
/// Decision settled during discussion (PHASE 82, 09/07/2026): limit the opening
/// tree to the strict opening rather than indexing the whole game,
/// for a controlled data volume on a database of several
/// hundred thousand games.
pub const OPENING_TREE_MAX_PLIES: usize = 30;

// ---------------------------------------------------------------------------
// Position hash conversion (u64 ↔ i64 `SQLite`)
// ---------------------------------------------------------------------------

/// Converts a Polyglot hash (`u64`) into a signed integer storable in an
/// `INTEGER` `SQLite` column.
///
/// Reversible bit-cast (see [`hash_from_sql`]): the hash is never used
/// arithmetically, only compared for equality, so the loss of meaning of the
/// sign bit has no consequence.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn hash_to_sql(hash: u64) -> i64 {
    hash as i64
}

/// Inverse of [`hash_to_sql`]: reconstructs the original Polyglot hash (`u64`)
/// from the `i64` value stored in the database.
#[must_use]
#[allow(clippy::cast_sign_loss)]
pub fn hash_from_sql(value: i64) -> u64 {
    value as u64
}

// ---------------------------------------------------------------------------
// Migrations
// ---------------------------------------------------------------------------

/// A versioned migration (same principle as [`crate::schema`]).
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

            -- Games imported from an external PGN database (PHASE 82).
            -- Unlike the `games` table of the application database
            -- (schema.rs), this one has additional columns
            -- (eco, elo, titles) extracted directly from the original
            -- PGN tags, needed to filter/weight the opening
            -- tree and the game list.
            CREATE TABLE IF NOT EXISTS games (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                white           TEXT    NOT NULL DEFAULT '?',
                black           TEXT    NOT NULL DEFAULT '?',
                result          TEXT    NOT NULL DEFAULT '*',
                date            TEXT,
                event           TEXT,
                site            TEXT,
                round           TEXT,
                eco             TEXT,
                white_elo       INTEGER,
                black_elo       INTEGER,
                white_title     TEXT,
                black_title     TEXT,
                ply_count       INTEGER NOT NULL DEFAULT 0,
                pgn             TEXT    NOT NULL,
                initial_fen     TEXT    NOT NULL DEFAULT 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
                created_at      TEXT    NOT NULL DEFAULT (datetime('now'))
            );

            -- Index for the game list filters (PHASE 82,
            -- discussion point 7: player, Elo range, date, opening).
            CREATE INDEX IF NOT EXISTS idx_ref_games_white     ON games(lower(white));
            CREATE INDEX IF NOT EXISTS idx_ref_games_black     ON games(lower(black));
            CREATE INDEX IF NOT EXISTS idx_ref_games_result    ON games(result);
            CREATE INDEX IF NOT EXISTS idx_ref_games_date      ON games(date);
            CREATE INDEX IF NOT EXISTS idx_ref_games_eco       ON games(eco);
            CREATE INDEX IF NOT EXISTS idx_ref_games_white_elo ON games(white_elo);
            CREATE INDEX IF NOT EXISTS idx_ref_games_black_elo ON games(black_elo);

            -- One row per (game, half-move played), up to
            -- OPENING_TREE_MAX_PLIES — raw basis of the opening tree
            -- (see this module's documentation: no pre-aggregation,
            -- so that the Elo filter remains adjustable without reimport).
            --
            -- `position_hash` = Polyglot hash (core::polyglot::polyglot_hash,
            -- converted via hash_to_sql) of the position BEFORE this move.
            -- `uci_move` = the move played from this position, in UCI format
            -- (e.g. \"e2e4\"), stored as-is (not the SAN) so it can be
            -- unambiguously reinterpreted independently of the position.
            CREATE TABLE IF NOT EXISTS game_positions (
                game_id         INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                ply             INTEGER NOT NULL,
                position_hash   INTEGER NOT NULL,
                uci_move        TEXT    NOT NULL,
                PRIMARY KEY (game_id, ply)
            );

            -- Main index of the opening tree: find all
            -- moves played from a given position (and, via the join
            -- on `games`, filter/weight by Elo at query time).
            CREATE INDEX IF NOT EXISTS idx_game_positions_hash ON game_positions(position_hash);
        ",
    },
];

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Opens (or creates) the reference games database at `path` and applies
/// all migrations.
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

/// Opens an **in-memory** reference games database (for tests).
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

/// Configures the connection for a persistent file: classic/rollback
/// journal mode (`DELETE`) and foreign key constraints — same
/// settings as the application database (see `schema::configure_pragmas` for
/// the full reasoning behind the `DELETE` mode choice).
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
///
/// Returns a `SQLite` error if creating the `migrations` table,
/// running a migration, or recording its version fails.
pub fn run_migrations(conn: &Connection) -> SqlResult<()> {
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
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 1);
    }

    #[test]
    fn test_tables_exist() {
        let conn = open_in_memory().unwrap();
        let tables = ["migrations", "games", "game_positions"];
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
            "idx_ref_games_white",
            "idx_ref_games_black",
            "idx_ref_games_result",
            "idx_ref_games_date",
            "idx_ref_games_eco",
            "idx_ref_games_white_elo",
            "idx_ref_games_black_elo",
            "idx_game_positions_hash",
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
    fn test_insert_minimal_game() {
        let conn = open_in_memory().unwrap();
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
    fn test_insert_game_with_enriched_metadata() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO games (white, black, result, eco, white_elo, black_elo,
                                 white_title, pgn)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "Carlsen, Magnus", "Nepomniachtchi, Ian", "1-0", "A00",
                2839, 2792, "GM", "[Event \"Test\"]\n\n1. e4 1-0",
            ],
        )
        .unwrap();
        let (eco, white_elo): (String, i64) = conn
            .query_row(
                "SELECT eco, white_elo FROM games WHERE white = 'Carlsen, Magnus'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(eco, "A00");
        assert_eq!(white_elo, 2839);
    }

    #[test]
    fn test_game_positions_foreign_key_cascade_delete() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO games (white, black, result, pgn) VALUES ('A', 'B', '1-0', '')",
            [],
        )
        .unwrap();
        let game_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO game_positions (game_id, ply, position_hash, uci_move)
             VALUES (?1, 0, ?2, 'e2e4')",
            rusqlite::params![game_id, hash_to_sql(0x1234_5678_9abc_def0)],
        )
        .unwrap();

        conn.execute("DELETE FROM games WHERE id = ?1", [game_id]).unwrap();

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM game_positions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "la suppression de la partie doit cascader sur game_positions");
    }

    #[test]
    fn test_game_positions_primary_key_rejects_duplicate_ply() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO games (white, black, result, pgn) VALUES ('A', 'B', '1-0', '')",
            [],
        )
        .unwrap();
        let game_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO game_positions (game_id, ply, position_hash, uci_move)
             VALUES (?1, 0, ?2, 'e2e4')",
            rusqlite::params![game_id, hash_to_sql(1)],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO game_positions (game_id, ply, position_hash, uci_move)
             VALUES (?1, 0, ?2, 'd2d4')",
            rusqlite::params![game_id, hash_to_sql(2)],
        );
        assert!(result.is_err(), "un doublon (game_id, ply) doit être rejeté");
    }

    #[test]
    fn test_hash_to_sql_roundtrip() {
        // Values covering edge cases: 0, largest possible u64
        // (sign bit set once converted), and a "normal" value.
        let values = [0u64, u64::MAX, 0x1234_5678_9abc_def0, 1, u64::from(u32::MAX)];
        for &v in &values {
            assert_eq!(hash_from_sql(hash_to_sql(v)), v);
        }
    }

    #[test]
    fn test_hash_roundtrip_via_sqlite_storage() {
        // Verifies that the bit-cast survives a real round trip through SQLite
        // (not just in memory on the Rust side) — inserts a hash with the
        // high-order bit set (would become negative as i64) and reads it back.
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO games (white, black, result, pgn) VALUES ('A', 'B', '1-0', '')",
            [],
        )
        .unwrap();
        let game_id = conn.last_insert_rowid();

        let original_hash: u64 = 0xffff_ffff_ffff_ffff;
        conn.execute(
            "INSERT INTO game_positions (game_id, ply, position_hash, uci_move)
             VALUES (?1, 0, ?2, 'e2e4')",
            rusqlite::params![game_id, hash_to_sql(original_hash)],
        )
        .unwrap();

        let stored: i64 = conn
            .query_row("SELECT position_hash FROM game_positions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(hash_from_sql(stored), original_hash);
    }
}
