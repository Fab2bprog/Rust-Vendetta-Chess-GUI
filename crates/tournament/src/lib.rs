//! Engine tournament logic — Phase 9.1.
//!
//! ## Supported formats
//!
//! - **Round Robin**: every engine plays every other engine. With `games_per_pair = 2`,
//!   each pair plays one game with White and one with Black.
//! - **Gauntlet**: the "challenger" (index 0) plays against all the others.
//!   The other engines do not play each other.
//!
//! ## Scores
//!
//! FIDE point system: win = 1.0, draw = 0.5, loss = 0.0.
//!
//! ## Pairings
//!
//! [`RoundRobinScheduler::pairings`] generates the rounds using the standard
//! circular algorithm (team rotation), guaranteeing color alternation across
//! multiple games.

// ── Public types ─────────────────────────────────────────────────────────────

/// Result of an individual game, from White's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// White wins.
    WhiteWins,
    /// Black wins.
    BlackWins,
    /// Draw.
    Draw,
}

impl GameResult {
    /// White's score for this result (0.0 / 0.5 / 1.0).
    #[must_use]
    pub fn white_score(self) -> f32 {
        match self {
            GameResult::WhiteWins => 1.0,
            GameResult::Draw      => 0.5,
            GameResult::BlackWins => 0.0,
        }
    }

    /// Black's score for this result.
    #[must_use]
    pub fn black_score(self) -> f32 {
        1.0 - self.white_score()
    }

    /// Converts a standard PGN result ("1-0", "0-1", "1/2-1/2") into a `GameResult`.
    /// Returns `None` if the string is not recognized.
    #[must_use]
    pub fn from_pgn(s: &str) -> Option<Self> {
        match s {
            "1-0"     => Some(GameResult::WhiteWins),
            "0-1"     => Some(GameResult::BlackWins),
            "1/2-1/2" => Some(GameResult::Draw),
            _         => None,
        }
    }

    /// Standard PGN representation.
    #[must_use]
    pub fn to_pgn(self) -> &'static str {
        match self {
            GameResult::WhiteWins => "1-0",
            GameResult::BlackWins => "0-1",
            GameResult::Draw      => "1/2-1/2",
        }
    }
}

/// Tournament type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TournamentKind {
    /// Every engine plays every other engine.
    RoundRobin,
    /// The challenger (index 0) plays against all the others; they do not meet each other.
    Gauntlet,
}

impl TournamentKind {
    /// Short representation for the database.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TournamentKind::RoundRobin => "roundrobin",
            TournamentKind::Gauntlet   => "gauntlet",
        }
    }
}

/// Configuration of a tournament before it starts.
#[derive(Debug, Clone)]
pub struct TournamentConfig {
    /// Tournament name (displayed in the UI and stored in the DB).
    pub name: String,
    /// Tournament format.
    pub kind: TournamentKind,
    /// List of participating engines: `(display_name, executable_path)`.
    /// In Gauntlet mode, index 0 is the challenger.
    pub engines: Vec<(String, String)>,
    /// Number of games played per pair (1 or 2).
    /// With 2, each engine plays once with White and once with Black.
    pub games_per_pair: u32,
    /// Time per move in milliseconds (go movetime).
    pub movetime_ms: u64,
}

impl TournamentConfig {
    /// Total number of games the tournament will play.
    #[must_use]
    pub fn total_games(&self) -> usize {
        let n = self.engines.len();
        let pairs = match self.kind {
            TournamentKind::RoundRobin => n * (n.saturating_sub(1)) / 2,
            TournamentKind::Gauntlet   => n.saturating_sub(1),
        };
        pairs * self.games_per_pair as usize
    }

    /// Validates the configuration. Returns a descriptive error if invalid.
    ///
    /// # Errors
    /// Returns a message describing the problem if the tournament has fewer
    /// than 2 engines, or if `games_per_pair` is neither 1 nor 2.
    pub fn validate(&self) -> Result<(), String> {
        if self.engines.len() < 2 {
            return Err("Un tournoi requiert au moins 2 moteurs.".into());
        }
        if self.games_per_pair == 0 || self.games_per_pair > 2 {
            return Err("games_per_pair doit être 1 ou 2.".into());
        }
        // movetime_ms == 0 is allowed: it means a timed time control is used.
        Ok(())
    }
}

/// A game scheduled in the tournament.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledGame {
    /// Index of the engine playing White in `TournamentConfig::engines`.
    pub white: usize,
    /// Index of the engine playing Black.
    pub black: usize,
    /// Round number (1-based).
    pub round: u32,
}

// ── Round Robin pairings ──────────────────────────────────────────────────────

/// Round Robin pairing generator (circular algorithm).
pub struct RoundRobinScheduler;

impl RoundRobinScheduler {
    /// Generates all the games for a Round Robin with `n` participants,
    /// `games_per_pair` games per pair (1 or 2).
    ///
    /// With `games_per_pair = 1`, each pair plays a single game.
    /// With `games_per_pair = 2`, the return matches are added (colors reversed).
    ///
    /// Returns the games in round order.
    ///
    /// Clippy (04/07/2026): `#[allow(cast_possible_truncation)]` — `round`
    /// (round number, `usize`) always stays tiny in practice (a real
    /// round-robin tournament will never exceed 2^32 rounds), so the
    /// conversion to `u32` never actually truncates.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn schedule(n: usize, games_per_pair: u32) -> Vec<ScheduledGame> {
        if n < 2 { return Vec::new(); }

        // Circular algorithm: player 0 is fixed and the others rotate.
        // With an odd number of players, a virtual "bye" is added (index n).
        let effective_n = if n.is_multiple_of(2) { n } else { n + 1 };
        let rounds = effective_n - 1;
        let per_round = effective_n / 2;

        let mut players: Vec<usize> = (0..effective_n).collect();
        let mut games: Vec<ScheduledGame> = Vec::new();

        for round in 0..rounds {
            for i in 0..per_round {
                let w = players[i];
                let b = players[effective_n - 1 - i];
                // Skip byes (index >= n)
                if w < n && b < n {
                    // Color alternation based on the parity of the round and slot
                    let (white, black) = if (round + i).is_multiple_of(2) { (w, b) } else { (b, w) };
                    games.push(ScheduledGame { white, black, round: round as u32 + 1 });
                    if games_per_pair == 2 {
                        games.push(ScheduledGame {
                            white: black,
                            black: white,
                            round: (rounds + round) as u32 + 1,
                        });
                    }
                }
            }
            // Rotation: player 0 stays fixed, the others rotate left
            players[1..].rotate_left(1);
        }

        // Sort by round number for a natural execution order
        games.sort_by_key(|g| g.round);
        games
    }
}

// ── Gauntlet pairings ─────────────────────────────────────────────────────────

/// Gauntlet pairing generator.
pub struct GauntletScheduler;

impl GauntletScheduler {
    /// Generates the games: the challenger (index 0) plays against every other engine.
    ///
    /// With `games_per_pair = 2`, the challenger plays one game with White
    /// and one with Black against each opponent.
    ///
    /// Clippy (04/07/2026): `#[allow(cast_possible_truncation)]` — `opponent`
    /// (engine index, `usize`) always stays tiny in practice (a tournament
    /// will never have 2^32 engines).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn schedule(n: usize, games_per_pair: u32) -> Vec<ScheduledGame> {
        if n < 2 { return Vec::new(); }
        let mut games = Vec::new();
        for opponent in 1..n {
            let round = (opponent as u32 - 1) * games_per_pair + 1;
            // Game 1: challenger = White
            games.push(ScheduledGame { white: 0, black: opponent, round });
            if games_per_pair == 2 {
                // Game 2: challenger = Black
                games.push(ScheduledGame { white: opponent, black: 0, round: round + 1 });
            }
        }
        games
    }
}

// ── Standings ────────────────────────────────────────────────────────────────

/// An engine's score in the tournament.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineScore {
    /// Index in `TournamentConfig::engines`.
    pub engine_idx: usize,
    /// Engine name.
    pub name: String,
    /// FIDE points (win = 1.0, draw = 0.5, loss = 0.0).
    pub points: f32,
    /// Wins.
    pub wins: u32,
    /// Draws.
    pub draws: u32,
    /// Losses.
    pub losses: u32,
}

impl EngineScore {
    /// Number of games played.
    #[must_use]
    pub fn games_played(&self) -> u32 {
        self.wins + self.draws + self.losses
    }
}

// ── Tournament state ───────────────────────────────────────────────────────────

/// Full state of a tournament in progress or finished.
#[derive(Debug, Clone)]
pub struct TournamentState {
    /// Original configuration.
    pub config: TournamentConfig,
    /// Remaining games to play (in order).
    pub pending: std::collections::VecDeque<ScheduledGame>,
    /// Games already played with their result.
    pub completed: Vec<(ScheduledGame, GameResult)>,
    /// Current scores, one per engine.
    pub scores: Vec<EngineScore>,
}

impl TournamentState {
    /// Creates a new state from a validated configuration.
    ///
    /// # Panics
    ///
    /// Panics if `config.validate()` fails (call `validate()` first).
    #[must_use]
    pub fn new(config: TournamentConfig) -> Self {
        let n = config.engines.len();
        let games = match config.kind {
            TournamentKind::RoundRobin =>
                RoundRobinScheduler::schedule(n, config.games_per_pair),
            TournamentKind::Gauntlet =>
                GauntletScheduler::schedule(n, config.games_per_pair),
        };

        let scores = config.engines.iter().enumerate().map(|(i, (name, _))| {
            EngineScore {
                engine_idx: i,
                name: name.clone(),
                points: 0.0,
                wins: 0, draws: 0, losses: 0,
            }
        }).collect();

        TournamentState {
            config,
            pending: games.into(),
            completed: Vec::new(),
            scores,
        }
    }

    /// Returns the next game to play, without removing it from the queue.
    #[must_use]
    pub fn next_game(&self) -> Option<&ScheduledGame> {
        self.pending.front()
    }

    /// Records the result of the game at the head of the queue and updates the scores.
    ///
    /// Returns `None` if the queue is empty.
    pub fn record_result(&mut self, result: GameResult) -> Option<ScheduledGame> {
        let game = self.pending.pop_front()?;

        // Update the scores
        let ws = result.white_score();
        let bs = result.black_score();

        self.scores[game.white].points += ws;
        self.scores[game.black].points += bs;

        match result {
            GameResult::WhiteWins => {
                self.scores[game.white].wins   += 1;
                self.scores[game.black].losses += 1;
            }
            GameResult::BlackWins => {
                self.scores[game.white].losses += 1;
                self.scores[game.black].wins   += 1;
            }
            GameResult::Draw => {
                self.scores[game.white].draws += 1;
                self.scores[game.black].draws += 1;
            }
        }

        self.completed.push((game.clone(), result));
        Some(game)
    }

    /// `true` if all games have been played.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.pending.is_empty()
    }

    /// Sorted standings: descending points, ties broken by descending wins.
    ///
    /// # Panics
    /// Does not panic in practice: `partial_cmp` on `points` (an `f32`)
    /// only fails for `NaN`, which can never appear here (points are
    /// always sums of `0.0`/`0.5`/`1.0`).
    #[must_use]
    pub fn standings(&self) -> Vec<&EngineScore> {
        let mut s: Vec<&EngineScore> = self.scores.iter().collect();
        s.sort_by(|a, b| {
            b.points.partial_cmp(&a.points).unwrap()
                .then(b.wins.cmp(&a.wins))
        });
        s
    }

    /// Number of games played.
    #[must_use]
    pub fn games_played(&self) -> usize {
        self.completed.len()
    }

    /// Total number of games planned.
    #[must_use]
    pub fn total_games(&self) -> usize {
        self.config.total_games()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
// Clippy (04/07/2026): `#[allow(float_cmp)]` — the tests compare FIDE
// scores against values exactly representable in `f32` (0.0/0.5/1.0),
// obtained via deterministic sums of these same constants, never via a
// floating-point computation subject to rounding; the strict comparison is
// therefore intentional and correct here.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn engines(n: usize) -> Vec<(String, String)> {
        (0..n).map(|i| (format!("Engine{i}"), format!("/bin/engine{i}"))).collect()
    }

    fn config_rr(n: usize, gpp: u32) -> TournamentConfig {
        TournamentConfig {
            name: "Test".into(),
            kind: TournamentKind::RoundRobin,
            engines: engines(n),
            games_per_pair: gpp,
            movetime_ms: 100,
        }
    }

    fn config_gauntlet(n: usize, gpp: u32) -> TournamentConfig {
        TournamentConfig {
            name: "Test".into(),
            kind: TournamentKind::Gauntlet,
            engines: engines(n),
            games_per_pair: gpp,
            movetime_ms: 100,
        }
    }

    // ── GameResult ────────────────────────────────────────────────────────────

    #[test]
    fn test_game_result_scores() {
        assert_eq!(GameResult::WhiteWins.white_score(), 1.0);
        assert_eq!(GameResult::WhiteWins.black_score(), 0.0);
        assert_eq!(GameResult::BlackWins.white_score(), 0.0);
        assert_eq!(GameResult::BlackWins.black_score(), 1.0);
        assert_eq!(GameResult::Draw.white_score(), 0.5);
        assert_eq!(GameResult::Draw.black_score(), 0.5);
    }

    #[test]
    fn test_game_result_pgn_roundtrip() {
        for (s, r) in [("1-0", GameResult::WhiteWins), ("0-1", GameResult::BlackWins), ("1/2-1/2", GameResult::Draw)] {
            assert_eq!(GameResult::from_pgn(s), Some(r));
            assert_eq!(r.to_pgn(), s);
        }
    }

    #[test]
    fn test_game_result_pgn_unknown() {
        assert_eq!(GameResult::from_pgn("*"), None);
        assert_eq!(GameResult::from_pgn(""), None);
    }

    // ── TournamentConfig ──────────────────────────────────────────────────────

    #[test]
    fn test_config_validate_ok() {
        assert!(config_rr(2, 1).validate().is_ok());
        assert!(config_rr(4, 2).validate().is_ok());
    }

    #[test]
    fn test_config_validate_too_few_engines() {
        assert!(config_rr(1, 1).validate().is_err());
        assert!(config_rr(0, 1).validate().is_err());
    }

    #[test]
    fn test_config_validate_bad_gpp() {
        assert!(config_rr(2, 0).validate().is_err());
        assert!(config_rr(2, 3).validate().is_err());
    }

    #[test]
    fn test_config_total_games_rr_2_engines_1gpp() {
        assert_eq!(config_rr(2, 1).total_games(), 1);
    }

    #[test]
    fn test_config_total_games_rr_2_engines_2gpp() {
        assert_eq!(config_rr(2, 2).total_games(), 2);
    }

    #[test]
    fn test_config_total_games_rr_3_engines_1gpp() {
        // 3 pairs → 3 games
        assert_eq!(config_rr(3, 1).total_games(), 3);
    }

    #[test]
    fn test_config_total_games_rr_4_engines_2gpp() {
        // 6 pairs × 2 = 12
        assert_eq!(config_rr(4, 2).total_games(), 12);
    }

    #[test]
    fn test_config_total_games_gauntlet_3_engines_1gpp() {
        // challenger vs 2 opponents = 2
        assert_eq!(config_gauntlet(3, 1).total_games(), 2);
    }

    #[test]
    fn test_config_total_games_gauntlet_4_engines_2gpp() {
        // challenger vs 3 × 2 = 6
        assert_eq!(config_gauntlet(4, 2).total_games(), 6);
    }

    // ── RoundRobinScheduler ───────────────────────────────────────────────────

    #[test]
    fn test_rr_2_engines_1gpp_count() {
        let g = RoundRobinScheduler::schedule(2, 1);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn test_rr_2_engines_2gpp_count() {
        let g = RoundRobinScheduler::schedule(2, 2);
        assert_eq!(g.len(), 2);
        // The colors must be reversed between the two games
        assert_ne!(g[0].white, g[1].white);
    }

    #[test]
    fn test_rr_3_engines_1gpp_count() {
        let g = RoundRobinScheduler::schedule(3, 1);
        assert_eq!(g.len(), 3);
    }

    #[test]
    fn test_rr_4_engines_1gpp_count() {
        let g = RoundRobinScheduler::schedule(4, 1);
        assert_eq!(g.len(), 6);
    }

    #[test]
    fn test_rr_4_engines_2gpp_count() {
        let g = RoundRobinScheduler::schedule(4, 2);
        assert_eq!(g.len(), 12);
    }

    #[test]
    fn test_rr_no_self_play() {
        for n in 2..=5 {
            for gpp in 1..=2 {
                let games = RoundRobinScheduler::schedule(n, gpp);
                for g in &games {
                    assert_ne!(g.white, g.black, "auto-jeu détecté n={n} gpp={gpp}");
                }
            }
        }
    }

    #[test]
    fn test_rr_all_pairs_covered_4_engines() {
        let games = RoundRobinScheduler::schedule(4, 1);
        let mut pairs: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        for g in &games {
            let pair = (g.white.min(g.black), g.white.max(g.black));
            pairs.insert(pair);
        }
        // All 6 pairs must be present
        assert_eq!(pairs.len(), 6);
    }

    #[test]
    fn test_rr_sorted_by_round() {
        let games = RoundRobinScheduler::schedule(4, 2);
        for w in games.windows(2) {
            assert!(w[0].round <= w[1].round);
        }
    }

    // ── GauntletScheduler ─────────────────────────────────────────────────────

    #[test]
    fn test_gauntlet_2_engines_1gpp() {
        let g = GauntletScheduler::schedule(2, 1);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].white, 0);
        assert_eq!(g[0].black, 1);
    }

    #[test]
    fn test_gauntlet_3_engines_1gpp() {
        let g = GauntletScheduler::schedule(3, 1);
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|p| p.white == 0 || p.black == 0));
    }

    #[test]
    fn test_gauntlet_3_engines_2gpp() {
        let g = GauntletScheduler::schedule(3, 2);
        assert_eq!(g.len(), 4);
        // The challenger must always be involved
        assert!(g.iter().all(|p| p.white == 0 || p.black == 0));
        // Non-challengers do not play each other
        for p in &g {
            assert!(p.white == 0 || p.black == 0);
        }
    }

    #[test]
    fn test_gauntlet_no_self_play() {
        for n in 2..=5 {
            for gpp in 1..=2 {
                let games = GauntletScheduler::schedule(n, gpp);
                for g in &games { assert_ne!(g.white, g.black); }
            }
        }
    }

    // ── TournamentState ───────────────────────────────────────────────────────

    #[test]
    fn test_state_initial_scores_zero() {
        let state = TournamentState::new(config_rr(3, 1));
        for s in &state.scores {
            assert_eq!(s.points, 0.0);
            assert_eq!(s.wins,   0);
            assert_eq!(s.draws,  0);
            assert_eq!(s.losses, 0);
        }
    }

    #[test]
    fn test_state_total_games() {
        let state = TournamentState::new(config_rr(4, 2));
        assert_eq!(state.total_games(), 12);
    }

    #[test]
    fn test_state_record_white_win() {
        let mut state = TournamentState::new(config_rr(2, 1));
        let g = state.next_game().cloned().unwrap();
        state.record_result(GameResult::WhiteWins);
        assert_eq!(state.scores[g.white].wins,   1);
        assert_eq!(state.scores[g.black].losses, 1);
        assert_eq!(state.scores[g.white].points, 1.0);
        assert_eq!(state.scores[g.black].points, 0.0);
        assert!(state.is_finished());
    }

    #[test]
    fn test_state_record_draw() {
        let mut state = TournamentState::new(config_rr(2, 1));
        let g = state.next_game().cloned().unwrap();
        state.record_result(GameResult::Draw);
        assert_eq!(state.scores[g.white].draws, 1);
        assert_eq!(state.scores[g.black].draws, 1);
        assert_eq!(state.scores[g.white].points, 0.5);
        assert_eq!(state.scores[g.black].points, 0.5);
    }

    #[test]
    fn test_state_not_finished_until_all_played() {
        let mut state = TournamentState::new(config_rr(3, 1));
        assert!(!state.is_finished());
        state.record_result(GameResult::WhiteWins);
        assert!(!state.is_finished());
        state.record_result(GameResult::Draw);
        assert!(!state.is_finished());
        state.record_result(GameResult::BlackWins);
        assert!(state.is_finished());
    }

    #[test]
    fn test_state_games_played_increments() {
        let mut state = TournamentState::new(config_rr(3, 1));
        assert_eq!(state.games_played(), 0);
        state.record_result(GameResult::WhiteWins);
        assert_eq!(state.games_played(), 1);
        state.record_result(GameResult::Draw);
        assert_eq!(state.games_played(), 2);
    }

    #[test]
    fn test_state_standings_sorted_by_points() {
        let mut state = TournamentState::new(config_rr(3, 1));
        // Record all the games
        while !state.is_finished() {
            state.record_result(GameResult::WhiteWins);
        }
        let standings = state.standings();
        // Descending points
        for w in standings.windows(2) {
            assert!(w[0].points >= w[1].points);
        }
    }

    #[test]
    fn test_state_standings_tiebreak_by_wins() {
        // 3 engines, all draws: each engine plays 2 games → 1.0 pt (2 × 0.5)
        let mut state = TournamentState::new(config_rr(3, 1));
        while !state.is_finished() {
            state.record_result(GameResult::Draw);
        }
        for s in &state.scores {
            assert_eq!(s.points, 1.0);
            assert_eq!(s.draws,  2);
            assert_eq!(s.wins,   0);
            assert_eq!(s.losses, 0);
        }
    }

    #[test]
    fn test_state_gauntlet_complete() {
        let mut state = TournamentState::new(config_gauntlet(4, 2));
        assert_eq!(state.total_games(), 6);
        while !state.is_finished() {
            state.record_result(GameResult::WhiteWins);
        }
        assert_eq!(state.games_played(), 6);
    }

    #[test]
    fn test_engine_score_games_played() {
        let s = EngineScore { engine_idx: 0, name: "E".into(), points: 1.5, wins: 1, draws: 1, losses: 1 };
        assert_eq!(s.games_played(), 3);
    }

    #[test]
    fn test_tournament_kind_as_str() {
        assert_eq!(TournamentKind::RoundRobin.as_str(), "roundrobin");
        assert_eq!(TournamentKind::Gauntlet.as_str(),   "gauntlet");
    }
}
