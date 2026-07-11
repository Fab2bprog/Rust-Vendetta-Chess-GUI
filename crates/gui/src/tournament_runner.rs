//! Orchestrator for an in-progress engine tournament.
//!
//! [`TournamentRunner`] encapsulates an active [`TournamentState`] as well as
//! the persisted metadata (DB identifier, database path).

use game_config::TimeControl;
use tournament::{GameResult, ScheduledGame, TournamentConfig, TournamentState};

// ── State of an active tournament ───────────────────────────────────────────

/// Data of an engine tournament currently being played.
pub struct TournamentRunner {
    /// Tournament state: game queue, scores, finished games.
    pub state: TournamentState,
    /// `SQLite` identifier of the tournament (`tournaments` table).
    pub tournament_id: i64,
    /// Absolute path to Vendetta Chess's `SQLite` database.
    pub db_path: String,
    /// `SQLite` connection opened only once when the tournament is created and
    /// reused for every game save, instead of reopening/
    /// re-migrating the database for each game played (perf audit 02/07/2026, point 6).
    pub db_conn: db::Connection,
    /// Time control chosen for all games of the tournament.
    /// Used to reset the clock at the start of each new game.
    pub time_control: TimeControl,
}

impl TournamentRunner {
    /// Creates a new runner from a validated configuration.
    ///
    /// `db_conn` must be the connection already used to create
    /// the tournament record (`tournament_repo::create_tournament`) —
    /// it is kept and reused for all game saves of this
    /// tournament.
    pub fn new(
        config: TournamentConfig,
        tournament_id: i64,
        db_path: String,
        db_conn: db::Connection,
        time_control: TimeControl,
    ) -> Self {
        Self {
            state: TournamentState::new(config),
            tournament_id,
            db_path,
            db_conn,
            time_control,
        }
    }

    /// Next game to be played (without removing it from the queue).
    pub fn next_game(&self) -> Option<&ScheduledGame> {
        self.state.next_game()
    }

    /// Records the result of the game at the head of the queue.
    ///
    /// Returns the recorded game, or `None` if the queue was empty.
    pub fn record_result(&mut self, result: GameResult) -> Option<ScheduledGame> {
        self.state.record_result(result)
    }

    /// `true` if all games of the tournament have been played.
    pub fn is_finished(&self) -> bool {
        self.state.is_finished()
    }

    /// Number of games already played.
    pub fn games_played(&self) -> usize {
        self.state.games_played()
    }

    /// Total number of games planned in this tournament.
    pub fn total_games(&self) -> usize {
        self.state.total_games()
    }
}

// ── Database path ───────────────────────────────────────────────────────────

/// Returns the absolute path of Vendetta Chess's `SQLite` database.
///
/// PHASE 24 (100% portability, USB): the database now lives in the
/// `base/` subfolder of the delivery folder (`VendettaChess/base/vendetta.db`),
/// next to the executable — no longer in a user system directory. The
/// `base/` folder is normally already created at startup by
/// `app_paths::ensure_app_dirs()` (see Step 1); `create_dir_all` is still
/// called here defensively, to remain correct even if this function were
/// one day called before that startup point.
#[must_use]
pub fn db_path() -> String {
    let dir = app_paths::base_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("vendetta.db").to_string_lossy().into_owned()
}

#[cfg(test)]
mod db_path_tests {
    use super::db_path;

    /// PHASE 24: the database must no longer live under a user system
    /// directory, but under `base/` next to the executable.
    #[test]
    fn test_db_path_ends_in_base_subdir_with_vendetta_db_filename() {
        let path = db_path();
        let path = std::path::Path::new(&path);

        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("vendetta.db"));
        assert_eq!(
            path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()),
            Some("base")
        );
    }

    #[test]
    fn test_db_path_creates_base_dir_as_side_effect() {
        let path = db_path();
        let parent = std::path::Path::new(&path).parent().expect("dossier parent");
        assert!(parent.is_dir());
    }
}
