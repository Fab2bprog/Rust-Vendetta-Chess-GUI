//! Result of a chess game.
//!
//! The full [`GameState`] (with history, notation, and rules) is in
//! [`crate::game`].

/// Result of a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// The game is in progress.
    Ongoing,
    /// White wins.
    WhiteWins,
    /// Black wins.
    BlackWins,
    /// Draw.
    Draw,
}

impl std::fmt::Display for GameResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Ongoing   => "*",
            Self::WhiteWins => "1-0",
            Self::BlackWins => "0-1",
            Self::Draw      => "1/2-1/2",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_result_display() {
        assert_eq!(GameResult::Ongoing.to_string(),   "*");
        assert_eq!(GameResult::WhiteWins.to_string(), "1-0");
        assert_eq!(GameResult::BlackWins.to_string(), "0-1");
        assert_eq!(GameResult::Draw.to_string(),      "1/2-1/2");
    }

    #[test]
    fn test_game_result_eq() {
        assert_eq!(GameResult::WhiteWins, GameResult::WhiteWins);
        assert_ne!(GameResult::WhiteWins, GameResult::BlackWins);
    }
}
