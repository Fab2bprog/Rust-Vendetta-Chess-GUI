//! Aggregation of raw UCI lines into structured analysis.
//!
//! A UCI engine sends many `info` lines during a search: one per depth, and
//! several per depth in `MultiPV` mode.
//! [`aggregate`] merges this raw stream into a clean [`AggregatedAnalysis`]
//! where each `MultiPV` line is represented by its entry at the **maximum
//! depth** reached.
//!
//! ## Example
//!
//! ```
//! use analysis::aggregator::aggregate;
//! use uci::engine::{AnalysisResult, EnginePosition};
//! use uci::parser::{UciInfo, UciScore};
//!
//! let info_lines = vec![
//!     UciInfo { depth: Some(1), multipv: Some(1),
//!               score: Some(UciScore::Centipawns(20)),
//!               pv: vec!["e2e4".into()], ..UciInfo::default() },
//!     UciInfo { depth: Some(2), multipv: Some(1),
//!               score: Some(UciScore::Centipawns(30)),
//!               pv: vec!["e2e4".into(), "e7e5".into()], ..UciInfo::default() },
//! ];
//! let result = AnalysisResult { best_move: "e2e4".into(), ponder: None, info_lines };
//! let agg = aggregate(&result);
//!
//! assert_eq!(agg.lines.len(), 1);
//! assert_eq!(agg.lines[0].best_depth, 2);
//! assert_eq!(agg.best_move, "e2e4");
//! ```

use uci::{
    engine::AnalysisResult,
    parser::{UciInfo, UciScore},
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A `MultiPV` line aggregated at its maximum depth.
#[derive(Debug, Clone)]
pub struct AggregatedLine {
    /// Number of the `MultiPV` line (1 = principal, 2 = second best move…).
    /// Equals `1` if the engine did not fill in the `multipv` field.
    pub multipv:   u32,
    /// Maximum depth reached for this line.
    pub best_depth: u32,
    /// Score at the maximum depth.
    pub score:     Option<UciScore>,
    /// Principal variation at the maximum depth.
    pub pv:        Vec<String>,
    /// Number of nodes analyzed (at the maximum depth).
    pub nodes:     Option<u64>,
    /// Nodes per second (at the maximum depth).
    pub nps:       Option<u64>,
}

impl AggregatedLine {
    /// First move of the principal variation (= best move for this line).
    #[must_use]
    pub fn first_move(&self) -> Option<&str> {
        self.pv.first().map(String::as_str)
    }

    /// Returns the score in centipawns, or `None` if absent or in Mate format.
    #[must_use]
    pub fn score_cp(&self) -> Option<i32> {
        match &self.score {
            Some(UciScore::Centipawns(cp)) => Some(*cp),
            _ => None,
        }
    }

    /// Returns `true` if the score indicates a mate.
    #[must_use]
    pub fn is_mate(&self) -> bool {
        matches!(&self.score, Some(UciScore::Mate(_)))
    }
}

/// Aggregated result of a complete UCI analysis.
#[derive(Debug, Clone)]
pub struct AggregatedAnalysis {
    /// `MultiPV` lines, sorted by line number (multipv 1 first).
    pub lines:    Vec<AggregatedLine>,
    /// Best move announced by the engine (`bestmove`).
    pub best_move: String,
    /// Ponder move suggested by the engine.
    pub ponder:   Option<String>,
}

impl AggregatedAnalysis {
    /// Principal line (multipv = 1).
    #[must_use]
    pub fn principal_line(&self) -> Option<&AggregatedLine> {
        self.lines.iter().find(|l| l.multipv == 1)
    }

    /// Number of aggregated `MultiPV` lines.
    #[must_use]
    pub fn multipv_count(&self) -> usize {
        self.lines.len()
    }

    /// Score of the principal line in centipawns.
    #[must_use]
    pub fn score_cp(&self) -> Option<i32> {
        self.principal_line()?.score_cp()
    }

    /// Maximum depth reached (on the principal line).
    #[must_use]
    pub fn max_depth(&self) -> Option<u32> {
        self.principal_line().map(|l| l.best_depth)
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Aggregates the `info` lines of an [`AnalysisResult`] into an [`AggregatedAnalysis`].
///
/// For each `MultiPV` line, only the entry at the **maximum depth** is kept.
/// Lines without a `multipv` field are treated as belonging to line 1.
///
/// The resulting lines are sorted by increasing `multipv` number.
#[must_use]
pub fn aggregate(result: &AnalysisResult) -> AggregatedAnalysis {
    let lines = aggregate_lines(&result.info_lines);
    AggregatedAnalysis {
        lines,
        best_move: result.best_move.clone(),
        ponder:    result.ponder.clone(),
    }
}

/// Aggregates a slice of [`UciInfo`] into a vector of [`AggregatedLine`].
///
/// Usable independently of [`aggregate`] when the `info` lines are already
/// available directly.
#[must_use]
pub fn aggregate_lines(info_lines: &[UciInfo]) -> Vec<AggregatedLine> {
    // Map: multipv → best UciInfo (criterion: maximum depth).
    // We use a Vec<Option<UciInfo>> indexed by (multipv - 1) to preserve
    // insertion order without a HashMap.
    let mut best: Vec<(u32, UciInfo)> = Vec::new();

    for info in info_lines {
        let mv = info.multipv.unwrap_or(1);
        let depth = info.depth.unwrap_or(0);

        match best.iter_mut().find(|(n, _)| *n == mv) {
            Some((_, existing)) => {
                if depth > existing.depth.unwrap_or(0) {
                    *existing = info.clone();
                }
            }
            None => {
                best.push((mv, info.clone()));
            }
        }
    }

    // Sort by MultiPV line number.
    best.sort_by_key(|(n, _)| *n);

    best.into_iter()
        .map(|(multipv, info)| AggregatedLine {
            multipv,
            best_depth: info.depth.unwrap_or(0),
            score:      info.score,
            pv:         info.pv,
            nodes:      info.nodes,
            nps:        info.nps,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uci::{
        engine::AnalysisResult,
        parser::{UciInfo, UciScore},
    };

    // Builds a minimal UciInfo.
    fn info(depth: u32, multipv: u32, cp: i32, pv: &[&str]) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: Some(multipv),
            score:   Some(UciScore::Centipawns(cp)),
            pv:      pv.iter().map(std::string::ToString::to_string).collect(),
            ..UciInfo::default()
        }
    }

    fn result_from(best_move: &str, lines: Vec<UciInfo>) -> AnalysisResult {
        AnalysisResult { best_move: best_move.to_string(), ponder: None, info_lines: lines }
    }

    // -----------------------------------------------------------------------

    #[test]
    fn test_aggregate_empty_lines() {
        let r   = result_from("e2e4", vec![]);
        let agg = aggregate(&r);
        assert!(agg.lines.is_empty());
        assert_eq!(agg.best_move, "e2e4");
    }

    #[test]
    fn test_aggregate_single_line_single_depth() {
        let r   = result_from("e2e4", vec![info(5, 1, 30, &["e2e4"])]);
        let agg = aggregate(&r);
        assert_eq!(agg.lines.len(), 1);
        assert_eq!(agg.lines[0].best_depth, 5);
        assert_eq!(agg.lines[0].score_cp(), Some(30));
    }

    #[test]
    fn test_aggregate_keeps_deepest_depth() {
        let lines = vec![
            info(1, 1, 10, &["e2e4"]),
            info(2, 1, 20, &["e2e4", "e7e5"]),
            info(3, 1, 30, &["e2e4", "e7e5", "g1f3"]),
        ];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        assert_eq!(agg.lines.len(), 1);
        assert_eq!(agg.lines[0].best_depth, 3);
        assert_eq!(agg.lines[0].score_cp(), Some(30));
        assert_eq!(agg.lines[0].pv.len(), 3);
    }

    #[test]
    fn test_aggregate_multipv_three_lines() {
        let lines = vec![
            info(5, 1, 30, &["e2e4"]),
            info(5, 2, 10, &["d2d4"]),
            info(5, 3,  5, &["g1f3"]),
        ];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        assert_eq!(agg.lines.len(), 3);
        assert_eq!(agg.lines[0].multipv, 1);
        assert_eq!(agg.lines[1].multipv, 2);
        assert_eq!(agg.lines[2].multipv, 3);
    }

    #[test]
    fn test_aggregate_multipv_sorted() {
        // Lines arriving out of order
        let lines = vec![
            info(4, 3,  5, &["g1f3"]),
            info(4, 1, 30, &["e2e4"]),
            info(4, 2, 10, &["d2d4"]),
        ];
        let agg = aggregate_lines(&lines);
        assert_eq!(agg[0].multipv, 1);
        assert_eq!(agg[1].multipv, 2);
        assert_eq!(agg[2].multipv, 3);
    }

    #[test]
    fn test_aggregate_no_multipv_tag_defaults_to_1() {
        let line = UciInfo {
            depth:   Some(5),
            multipv: None, // no multipv tag
            score:   Some(UciScore::Centipawns(25)),
            pv:      vec!["e2e4".into()],
            ..UciInfo::default()
        };
        let agg = aggregate_lines(&[line]);
        assert_eq!(agg.len(), 1);
        assert_eq!(agg[0].multipv, 1);
    }

    #[test]
    fn test_aggregate_principal_line() {
        let lines = vec![info(8, 1, 40, &["e2e4"]), info(8, 2, 15, &["d2d4"])];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        let pl  = agg.principal_line().unwrap();
        assert_eq!(pl.multipv, 1);
        assert_eq!(pl.score_cp(), Some(40));
    }

    #[test]
    fn test_aggregate_score_cp() {
        let lines = vec![info(10, 1, 55, &["e2e4"])];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        assert_eq!(agg.score_cp(), Some(55));
    }

    #[test]
    fn test_aggregate_max_depth() {
        let lines = vec![info(1, 1, 10, &["e2e4"]), info(12, 1, 30, &["e2e4"])];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        assert_eq!(agg.max_depth(), Some(12));
    }

    #[test]
    fn test_aggregate_first_move() {
        let lines = vec![info(5, 1, 20, &["d2d4", "d7d5"])];
        let agg = aggregate_lines(&lines);
        assert_eq!(agg[0].first_move(), Some("d2d4"));
    }

    #[test]
    fn test_aggregate_is_mate() {
        let line = UciInfo {
            depth:   Some(5),
            multipv: Some(1),
            score:   Some(UciScore::Mate(3)),
            pv:      vec!["e2e4".into()],
            ..UciInfo::default()
        };
        let agg = aggregate_lines(&[line]);
        assert!(agg[0].is_mate());
        assert_eq!(agg[0].score_cp(), None);
    }

    #[test]
    fn test_aggregate_ponder_preserved() {
        let r = AnalysisResult {
            best_move:  "e2e4".into(),
            ponder:     Some("e7e5".into()),
            info_lines: vec![],
        };
        let agg = aggregate(&r);
        assert_eq!(agg.ponder.as_deref(), Some("e7e5"));
    }

    #[test]
    fn test_aggregate_multipv_count() {
        let lines = vec![info(5, 1, 30, &["e2e4"]), info(5, 2, 10, &["d2d4"])];
        let r   = result_from("e2e4", lines);
        let agg = aggregate(&r);
        assert_eq!(agg.multipv_count(), 2);
    }

    #[test]
    fn test_aggregate_mixed_depths_per_line() {
        // Line 1: depth 1, 2, 3; Line 2: depth 1, 2
        let lines = vec![
            info(1, 1, 10, &["e2e4"]),
            info(1, 2,  5, &["d2d4"]),
            info(2, 1, 20, &["e2e4", "e7e5"]),
            info(2, 2,  8, &["d2d4", "d7d5"]),
            info(3, 1, 25, &["e2e4", "e7e5", "g1f3"]),
        ];
        let agg = aggregate_lines(&lines);
        assert_eq!(agg[0].best_depth, 3); // line 1 → depth 3
        assert_eq!(agg[1].best_depth, 2); // line 2 → depth 2
    }
}
