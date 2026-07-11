//! Comparison of N engines' analyses on the same position.
//!
//! This module takes already-aggregated analysis results (via [`crate::aggregator`])
//! and ranks them by decreasing score, from best to worst engine.
//!
//! ## Ranking rules
//!
//! 1. Positive mate (win) first — fastest mate first.
//! 2. Centipawns, from highest to lowest.
//! 3. Negative mate (loss) last — slowest first (less bad).
//! 4. No score: bottom of the list.
//!
//! ## Example
//!
//! ```
//! use analysis::aggregator::{AggregatedAnalysis, AggregatedLine};
//! use analysis::comparator::{EngineResult, compare_engines, best_consensus_move};
//! use uci::parser::UciScore;
//!
//! let line = AggregatedLine {
//!     multipv: 1, best_depth: 10,
//!     score: Some(UciScore::Centipawns(40)),
//!     pv: vec!["e2e4".into()], nodes: None, nps: None,
//! };
//! let analysis = AggregatedAnalysis {
//!     lines: vec![line], best_move: "e2e4".into(), ponder: None,
//! };
//! let results = vec![EngineResult::new("stockfish", analysis)];
//! let ranked  = compare_engines(&results);
//!
//! assert_eq!(ranked[0].rank, 1);
//! assert_eq!(ranked[0].engine_id, "stockfish");
//! assert_eq!(best_consensus_move(&results), Some("e2e4"));
//! ```

use std::collections::HashMap;

use uci::parser::UciScore;

use crate::aggregator::AggregatedAnalysis;

// ---------------------------------------------------------------------------
// Sort key (private)
// ---------------------------------------------------------------------------

/// Computes an `i64` sort key for a UCI score (higher = better).
///
/// | Score              | Approximate key               |
/// |---------------------|-------------------------------|
/// | `Mate(+1)` (win)   | ≈ 100 999 (maximum)          |
/// | `Mate(+5)` (win)   | ≈ 100 995                    |
/// | `Centipawns(cp)`   | `cp` (typical range ±3 000)  |
/// | `Mate(-5)` (loss)  | ≈ −99 995 (less bad)         |
/// | `Mate(-1)` (loss)  | ≈ −99 999 (worst)            |
/// | Absent / other     | `i64::MIN / 2`               |
fn score_sort_key(score: Option<&UciScore>) -> i64 {
    match score {
        Some(UciScore::Mate(n)) if *n > 0 => {
            // Win: fastest mate first.
            100_000_i64 + (1_000_i64 - i64::from(*n).clamp(1, 1_000))
        }
        Some(UciScore::Centipawns(cp)) => i64::from(*cp),
        Some(UciScore::Mate(n)) => {
            // Loss (n ≤ 0): losing as slowly as possible = better.
            // Mate(-5) → −100 000 + 5 = −99 995 > Mate(-1) → −99 999.
            -100_000_i64 - i64::from(*n)
        }
        _ => i64::MIN / 2,
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Aggregated analysis result for a given engine.
#[derive(Debug, Clone)]
pub struct EngineResult {
    /// Unique engine identifier.
    pub engine_id: String,
    /// Aggregated analysis produced by [`crate::aggregator::aggregate`].
    pub analysis:  AggregatedAnalysis,
}

impl EngineResult {
    /// Creates a new [`EngineResult`].
    pub fn new(engine_id: impl Into<String>, analysis: AggregatedAnalysis) -> Self {
        Self { engine_id: engine_id.into(), analysis }
    }

    /// Score in centipawns of the principal line, or `None`.
    #[must_use]
    pub fn score_cp(&self) -> Option<i32> {
        self.analysis.score_cp()
    }

    /// Best move (`bestmove`) announced by the engine.
    #[must_use]
    pub fn best_move(&self) -> &str {
        &self.analysis.best_move
    }

    /// Maximum depth reached (principal line).
    #[must_use]
    pub fn depth(&self) -> Option<u32> {
        self.analysis.max_depth()
    }

    /// `true` if the principal line indicates a mate.
    #[must_use]
    pub fn is_mate(&self) -> bool {
        self.analysis
            .principal_line()
            .is_some_and(super::aggregator::AggregatedLine::is_mate)
    }
}

/// An entry in the engines' comparative ranking.
#[derive(Debug, Clone)]
pub struct RankedResult {
    /// Rank in the ranking (1 = best score).
    pub rank:      usize,
    /// Engine identifier.
    pub engine_id: String,
    /// Score in centipawns of the principal line, or `None`.
    pub score_cp:  Option<i32>,
    /// Best move proposed by this engine.
    pub best_move: String,
    /// Maximum depth reached (principal line).
    pub depth:     Option<u32>,
    /// `true` if the score is a mate.
    pub is_mate:   bool,
}

// ---------------------------------------------------------------------------
// Main functions
// ---------------------------------------------------------------------------

/// Ranks the results of N engines by decreasing score.
///
/// Returns a vector of [`RankedResult`] sorted from best to worst,
/// with ranks assigned starting at 1. An empty input vector returns
/// an empty vector.
#[must_use]
pub fn compare_engines(results: &[EngineResult]) -> Vec<RankedResult> {
    let mut indices: Vec<usize> = (0..results.len()).collect();
    indices.sort_by(|&a, &b| {
        let ka = score_sort_key(
            results[a].analysis.principal_line().and_then(|l| l.score.as_ref()),
        );
        let kb = score_sort_key(
            results[b].analysis.principal_line().and_then(|l| l.score.as_ref()),
        );
        kb.cmp(&ka)
    });

    indices
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let r = &results[idx];
            RankedResult {
                rank:      i + 1,
                engine_id: r.engine_id.clone(),
                score_cp:  r.score_cp(),
                best_move: r.best_move().to_owned(),
                depth:     r.depth(),
                is_mate:   r.is_mate(),
            }
        })
        .collect()
}

/// Returns the move most frequently proposed among the engines.
///
/// In case of a tie in the number of votes, the move proposed by the engine
/// with the **best score** is returned.
///
/// Returns `None` if `results` is empty or if all `best_move`s are empty.
#[must_use]
pub fn best_consensus_move(results: &[EngineResult]) -> Option<&str> {
    if results.is_empty() {
        return None;
    }

    // Count votes for each move.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for r in results {
        let mv = r.best_move();
        if !mv.is_empty() {
            *counts.entry(mv).or_insert(0) += 1;
        }
    }

    let max_votes = *counts.values().max()?;

    // Sort engines by decreasing score.
    let mut sorted_indices: Vec<usize> = (0..results.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        let ka = score_sort_key(
            results[a].analysis.principal_line().and_then(|l| l.score.as_ref()),
        );
        let kb = score_sort_key(
            results[b].analysis.principal_line().and_then(|l| l.score.as_ref()),
        );
        kb.cmp(&ka)
    });

    // First engine (best score) whose move has the max number of votes.
    for idx in sorted_indices {
        let mv = results[idx].best_move();
        if counts.get(mv).copied().unwrap_or(0) == max_votes {
            return Some(mv);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::{AggregatedLine, AggregatedAnalysis};
    use uci::parser::UciScore;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_line(score: UciScore, depth: u32, mv: &str) -> AggregatedLine {
        AggregatedLine {
            multipv:    1,
            best_depth: depth,
            score:      Some(score),
            pv:         vec![mv.to_string()],
            nodes:      None,
            nps:        None,
        }
    }

    fn make_analysis(best_move: &str, score: UciScore, depth: u32) -> AggregatedAnalysis {
        AggregatedAnalysis {
            lines:     vec![make_line(score, depth, best_move)],
            best_move: best_move.to_string(),
            ponder:    None,
        }
    }

    fn make_result(id: &str, mv: &str, score: UciScore, depth: u32) -> EngineResult {
        EngineResult::new(id, make_analysis(mv, score, depth))
    }

    fn no_score_result(id: &str, mv: &str) -> EngineResult {
        EngineResult {
            engine_id: id.to_string(),
            analysis:  AggregatedAnalysis { lines: vec![], best_move: mv.to_string(), ponder: None },
        }
    }

    // -----------------------------------------------------------------------
    // compare_engines
    // -----------------------------------------------------------------------

    #[test]
    fn test_compare_empty() {
        assert!(compare_engines(&[]).is_empty());
    }

    #[test]
    fn test_compare_single_engine() {
        let r      = make_result("e1", "e2e4", UciScore::Centipawns(30), 10);
        let ranked = compare_engines(&[r]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[0].engine_id, "e1");
        assert_eq!(ranked[0].score_cp,  Some(30));
        assert_eq!(ranked[0].best_move, "e2e4");
        assert_eq!(ranked[0].depth,     Some(10));
        assert!(!ranked[0].is_mate);
    }

    #[test]
    fn test_compare_two_engines_score_order() {
        let r1 = make_result("e1", "e2e4", UciScore::Centipawns(50), 10);
        let r2 = make_result("e2", "d2d4", UciScore::Centipawns(20), 10);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e1");
        assert_eq!(ranked[1].engine_id, "e2");
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[1].rank, 2);
    }

    #[test]
    fn test_compare_mate_beats_centipawns() {
        let r1 = make_result("e1", "e2e4", UciScore::Centipawns(500), 15);
        let r2 = make_result("e2", "d1h5", UciScore::Mate(3), 12);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e2");
        assert!(ranked[0].is_mate);
        assert!(!ranked[1].is_mate);
    }

    #[test]
    fn test_compare_faster_mate_is_better() {
        // Mate in 1 is better than mate in 5.
        let r1 = make_result("e1", "e2e4", UciScore::Mate(5), 10);
        let r2 = make_result("e2", "d2d4", UciScore::Mate(1), 10);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e2");
    }

    #[test]
    fn test_compare_negative_mate_is_last() {
        // Negative centipawns still rank ahead of an unfavorable mate.
        let r1 = make_result("e1", "e2e4", UciScore::Centipawns(-300), 10);
        let r2 = make_result("e2", "d2d4", UciScore::Mate(-2), 10);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e1");
        assert_eq!(ranked[1].engine_id, "e2");
    }

    #[test]
    fn test_compare_negative_mate_ordering() {
        // Losing in 5 moves is less bad than losing in 1 move.
        let r1 = make_result("e1", "e2e4", UciScore::Mate(-1), 10);
        let r2 = make_result("e2", "d2d4", UciScore::Mate(-5), 10);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e2");
    }

    #[test]
    fn test_compare_no_score_is_last() {
        // Even a very negative score ranks ahead of no score at all.
        let r1 = no_score_result("e1", "e2e4");
        let r2 = make_result("e2", "d2d4", UciScore::Centipawns(-900), 5);
        let ranked = compare_engines(&[r1, r2]);
        assert_eq!(ranked[0].engine_id, "e2");
        assert_eq!(ranked[1].engine_id, "e1");
    }

    // -----------------------------------------------------------------------
    // best_consensus_move
    // -----------------------------------------------------------------------

    #[test]
    fn test_consensus_empty() {
        assert!(best_consensus_move(&[]).is_none());
    }

    #[test]
    fn test_consensus_single_engine() {
        let r = make_result("e1", "e2e4", UciScore::Centipawns(30), 10);
        assert_eq!(best_consensus_move(&[r]), Some("e2e4"));
    }

    #[test]
    fn test_consensus_majority() {
        // e2e4: 2 votes, d2d4: 1 vote → e2e4 wins.
        let r1 = make_result("e1", "e2e4", UciScore::Centipawns(30), 10);
        let r2 = make_result("e2", "e2e4", UciScore::Centipawns(25), 10);
        let r3 = make_result("e3", "d2d4", UciScore::Centipawns(20), 10);
        assert_eq!(best_consensus_move(&[r1, r2, r3]), Some("e2e4"));
    }

    #[test]
    fn test_consensus_tie_uses_best_score() {
        // 1-1 tie: e1 (+50) → e2e4, e2 (+40) → d2d4
        // → move from the engine with the best score (e1 → e2e4).
        let r1 = make_result("e1", "e2e4", UciScore::Centipawns(50), 10);
        let r2 = make_result("e2", "d2d4", UciScore::Centipawns(40), 10);
        assert_eq!(best_consensus_move(&[r1, r2]), Some("e2e4"));
    }
}
