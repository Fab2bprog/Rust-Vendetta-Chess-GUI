use super::chess_move::Move;

/// Score returned by a UCI engine.
///
/// - `Centipawns(n)`: advantage in centipawns (positive = advantage for the side to move).
/// - `MateIn(n)`: mate in `n` moves (positive = the side to move mates, negative = the side to move is mated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Score {
    /// Evaluation in centipawns.
    Centipawns(i32),
    /// Forced mate in N half-moves.
    MateIn(i32),
}

impl Score {
    /// Returns `true` if the score indicates a mate.
    #[must_use]
    #[inline]
    pub fn is_mate(self) -> bool {
        matches!(self, Self::MateIn(_))
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Centipawns(cp) => write!(f, "{cp:+.2}", cp = f64::from(*cp) / 100.0),
            Self::MateIn(n) if *n > 0 => write!(f, "M{n}"),
            Self::MateIn(n)           => write!(f, "-M{}", n.unsigned_abs()),
        }
    }
}

// ---------------------------------------------------------------------------

/// Full evaluation produced by an engine for a given position.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// Score of the position.
    pub score:   Score,
    /// Analysis depth (half-moves).
    pub depth:   u8,
    /// Number of nodes explored.
    pub nodes:   u64,
    /// Analysis time in milliseconds.
    pub time_ms: u64,
    /// Principal variation (recommended move sequence).
    pub pv:      Vec<Move>,
}

impl Evaluation {
    /// Builds a minimal evaluation (without PV).
    #[must_use]
    pub fn new(score: Score, depth: u8) -> Self {
        Self {
            score,
            depth,
            nodes:   0,
            time_ms: 0,
            pv:      Vec::new(),
        }
    }

    /// Returns the first move of the principal variation, if it exists.
    #[must_use]
    pub fn best_move(&self) -> Option<Move> {
        self.pv.first().copied()
    }
}

impl std::fmt::Display for Evaluation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "score={} depth={} nodes={} time={}ms", self.score, self.depth, self.nodes, self.time_ms)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{chess_move::Move, square::Square};

    #[test]
    fn test_score_display_centipawns() {
        assert_eq!(Score::Centipawns(150).to_string(),  "+1.50");
        assert_eq!(Score::Centipawns(-50).to_string(),  "-0.50");
        assert_eq!(Score::Centipawns(0).to_string(),    "+0.00");
    }

    #[test]
    fn test_score_display_mate() {
        assert_eq!(Score::MateIn(3).to_string(),  "M3");
        assert_eq!(Score::MateIn(-2).to_string(), "-M2");
    }

    #[test]
    fn test_score_is_mate() {
        assert!(Score::MateIn(3).is_mate());
        assert!(!Score::Centipawns(100).is_mate());
    }

    #[test]
    fn test_evaluation_best_move_empty_pv() {
        let eval = Evaluation::new(Score::Centipawns(30), 10);
        assert!(eval.best_move().is_none());
    }

    #[test]
    fn test_evaluation_best_move() {
        let mut eval = Evaluation::new(Score::Centipawns(50), 12);
        let m = Move::normal(
            Square::from_algebraic("e2").unwrap(),
            Square::from_algebraic("e4").unwrap(),
        );
        eval.pv.push(m);
        assert_eq!(eval.best_move(), Some(m));
    }

    #[test]
    fn test_evaluation_display() {
        let eval = Evaluation::new(Score::Centipawns(100), 10);
        let s = eval.to_string();
        assert!(s.contains("depth=10"));
        assert!(s.contains("+1.00"));
    }
}
