//! Data series for analysis visualization.
//!
//! This module transforms raw `info` lines from a UCI engine into series of
//! points (depth → score / nodes / nps) usable by the Phase 7 chart
//! rendering engine.
//!
//! ## Output data
//!
//! - [`DataPoint`]  — a point on the curve at a given depth.
//! - [`ScoreSeries`] — full series for an engine (one entry = one depth).
//! - [`build_series`] — builds the series from raw `info` lines.
//! - [`build_multi_series`] — multi-engine version.
//! - [`score_range`] — `(min, max)` interval for calibrating the Y axis.
//!
//! ## Mate conversion
//!
//! For plotting, mate scores are converted into fictitious centipawns:
//! - `Mate(+n)` → `30 000 − n` (≈ +∞, fastest mate first)
//! - `Mate(−n)` → `−30 000 + |n|` (≈ −∞, losing slowly is less bad)
//!
//! ## Example
//!
//! ```
//! use analysis::graph::build_series;
//! use uci::parser::{UciInfo, UciScore};
//!
//! let lines = vec![
//!     UciInfo { depth: Some(1), multipv: Some(1),
//!               score: Some(UciScore::Centipawns(10)), ..UciInfo::default() },
//!     UciInfo { depth: Some(2), multipv: Some(1),
//!               score: Some(UciScore::Centipawns(25)), ..UciInfo::default() },
//! ];
//! let series = build_series("stockfish", &lines);
//!
//! assert_eq!(series.points.len(), 2);
//! assert_eq!(series.max_depth(), Some(2));
//! assert_eq!(series.latest_score_cp(), Some(25));
//! ```

use uci::parser::{UciInfo, UciScore};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A point on an engine's score / depth curve.
#[derive(Debug, Clone)]
pub struct DataPoint {
    /// Search depth.
    pub depth: u32,
    /// Raw UCI score at this depth.
    pub score: Option<UciScore>,
    /// Nodes analyzed at this depth.
    pub nodes: Option<u64>,
    /// Nodes per second at this depth.
    pub nps:   Option<u64>,
}

impl DataPoint {
    /// Score in raw centipawns.
    ///
    /// Returns `None` if the score is absent or expressed in mate moves.
    #[must_use]
    pub fn score_cp(&self) -> Option<i32> {
        match &self.score {
            Some(UciScore::Centipawns(cp)) => Some(*cp),
            _ => None,
        }
    }

    /// "Display" centipawn score: mates are converted to fictitious values
    /// to allow plotting on a continuous Y axis.
    ///
    /// - `Mate(+n)` → `30 000 − n`
    /// - `Mate(−n)` → `−30 000 + |n|`
    ///
    /// Returns `None` if no score is available.
    #[must_use]
    pub fn score_display_cp(&self) -> Option<i32> {
        match &self.score {
            Some(UciScore::Mate(n)) if *n > 0 => Some(30_000 - *n),
            Some(UciScore::Mate(n)) => Some(-30_000 - *n), // n < 0 → --n = +|n|
            Some(UciScore::Centipawns(cp)) => Some(*cp),
            _ => None,
        }
    }

    /// `true` if the score indicates a mate (positive or negative).
    #[must_use]
    pub fn is_mate(&self) -> bool {
        matches!(&self.score, Some(UciScore::Mate(_)))
    }
}

/// Series of points (depth → data) for a given engine.
#[derive(Debug, Clone)]
pub struct ScoreSeries {
    /// Engine identifier.
    pub engine_id: String,
    /// Points sorted by increasing depth, one per depth.
    pub points:    Vec<DataPoint>,
}

impl ScoreSeries {
    /// Number of points in the series.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// `true` if the series contains no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Maximum depth reached in the series.
    #[must_use]
    pub fn max_depth(&self) -> Option<u32> {
        self.points.iter().map(|p| p.depth).max()
    }

    /// Score at the last point (maximum depth), in display centipawns.
    ///
    /// Returns `None` if the series is empty or the score is absent.
    #[must_use]
    pub fn latest_score_cp(&self) -> Option<i32> {
        self.points.last()?.score_display_cp()
    }
}

// ---------------------------------------------------------------------------
// Main functions
// ---------------------------------------------------------------------------

/// Builds the data series from an engine's raw `info` lines.
///
/// Only the **principal line** (`multipv == 1` or field absent) is kept.
/// For each depth, the **last** emitted line is kept (the most recent one
/// in the iterative search). Lines without a `depth` field are ignored.
/// The result is sorted by increasing depth.
#[must_use]
pub fn build_series(engine_id: &str, info_lines: &[UciInfo]) -> ScoreSeries {
    let mut by_depth: Vec<(u32, &UciInfo)> = Vec::new();

    for info in info_lines {
        // Ignore MultiPV lines other than the principal line.
        if info.multipv.unwrap_or(1) != 1 {
            continue;
        }
        // Ignore lines without a depth.
        let Some(depth) = info.depth else { continue };

        match by_depth.iter_mut().find(|(d, _)| *d == depth) {
            Some((_, existing)) => *existing = info, // more recent line at the same depth
            None => by_depth.push((depth, info)),
        }
    }

    // Sort by increasing depth.
    by_depth.sort_by_key(|(d, _)| *d);

    let points = by_depth
        .into_iter()
        .map(|(depth, info)| DataPoint {
            depth,
            score: info.score.clone(),
            nodes: info.nodes,
            nps:   info.nps,
        })
        .collect();

    ScoreSeries { engine_id: engine_id.to_owned(), points }
}

/// Builds one series per engine from an array of
/// `(engine_id, info_lines)` pairs.
#[must_use]
pub fn build_multi_series(data: &[(&str, &[UciInfo])]) -> Vec<ScoreSeries> {
    data.iter()
        .map(|(id, lines)| build_series(id, lines))
        .collect()
}

/// Returns the `(min_cp, max_cp)` interval across all series, for calibrating
/// a chart's Y axis.
///
/// Uses display centipawn scores (mates → ±30 000).
/// Returns `None` if no series contains a score.
#[must_use]
pub fn score_range(series: &[ScoreSeries]) -> Option<(i32, i32)> {
    let mut min = i32::MAX;
    let mut max = i32::MIN;
    let mut found = false;

    for s in series {
        for p in &s.points {
            if let Some(cp) = p.score_display_cp() {
                if cp < min { min = cp; }
                if cp > max { max = cp; }
                found = true;
            }
        }
    }

    found.then_some((min, max))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uci::parser::{UciInfo, UciScore};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn info_cp(depth: u32, multipv: u32, cp: i32) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: Some(multipv),
            score:   Some(UciScore::Centipawns(cp)),
            ..UciInfo::default()
        }
    }

    fn info_no_multipv(depth: u32, cp: i32) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: None,
            score:   Some(UciScore::Centipawns(cp)),
            ..UciInfo::default()
        }
    }

    fn info_mate(depth: u32, mate_in: i32) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: Some(1),
            score:   Some(UciScore::Mate(mate_in)),
            ..UciInfo::default()
        }
    }

    // -----------------------------------------------------------------------
    // build_series
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_series_empty() {
        let s = build_series("e", &[]);
        assert!(s.is_empty());
        assert_eq!(s.engine_id, "e");
    }

    #[test]
    fn test_build_series_single_depth() {
        let lines = vec![info_cp(5, 1, 30)];
        let s = build_series("e", &lines);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].depth, 5);
        assert_eq!(s.points[0].score_cp(), Some(30));
    }

    #[test]
    fn test_build_series_sorted_ascending() {
        // Lines out of order → sorted result.
        let lines = vec![info_cp(3, 1, 30), info_cp(1, 1, 10), info_cp(2, 1, 20)];
        let s = build_series("e", &lines);
        assert_eq!(s.points.len(), 3);
        assert_eq!(s.points[0].depth, 1);
        assert_eq!(s.points[1].depth, 2);
        assert_eq!(s.points[2].depth, 3);
    }

    #[test]
    fn test_build_series_keeps_last_duplicate_depth() {
        // Two lines at the same depth → keep the last one (most recent).
        let lines = vec![info_cp(5, 1, 10), info_cp(5, 1, 40)];
        let s = build_series("e", &lines);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].score_cp(), Some(40));
    }

    #[test]
    fn test_build_series_filters_multipv_other_than_1() {
        let lines = vec![
            info_cp(5, 1, 30), // included
            info_cp(5, 2, 10), // excluded
            info_cp(5, 3,  5), // excluded
        ];
        let s = build_series("e", &lines);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].score_cp(), Some(30));
    }

    #[test]
    fn test_build_series_no_multipv_defaults_to_principal() {
        // Line without a multipv tag → treated as the principal line.
        let lines = vec![info_no_multipv(7, 55)];
        let s = build_series("e", &lines);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].depth, 7);
        assert_eq!(s.points[0].score_cp(), Some(55));
    }

    // -----------------------------------------------------------------------
    // ScoreSeries — methods
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_depth() {
        let lines = vec![info_cp(1, 1, 0), info_cp(5, 1, 0), info_cp(10, 1, 0)];
        let s = build_series("e", &lines);
        assert_eq!(s.max_depth(), Some(10));
    }

    #[test]
    fn test_latest_score_cp() {
        let lines = vec![info_cp(5, 1, 20), info_cp(10, 1, 55)];
        let s = build_series("e", &lines);
        assert_eq!(s.latest_score_cp(), Some(55));
    }

    // -----------------------------------------------------------------------
    // DataPoint — score_display_cp / is_mate
    // -----------------------------------------------------------------------

    #[test]
    fn test_score_display_cp_mate_win() {
        // Mate(+3) → 30_000 − 3 = 29_997
        let lines = vec![info_mate(8, 3)];
        let s = build_series("e", &lines);
        assert!(s.points[0].is_mate());
        assert_eq!(s.points[0].score_cp(), None);              // no raw centipawns
        assert_eq!(s.points[0].score_display_cp(), Some(29_997));
    }

    #[test]
    fn test_score_display_cp_mate_loss() {
        // Mate(−2) → −30_000 − (−2) = −29_998
        let lines = vec![info_mate(6, -2)];
        let s = build_series("e", &lines);
        assert!(s.points[0].is_mate());
        assert_eq!(s.points[0].score_display_cp(), Some(-29_998));
    }

    // -----------------------------------------------------------------------
    // build_multi_series / score_range
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_multi_series() {
        let l1 = vec![info_cp(5, 1, 30)];
        let l2 = vec![info_cp(5, 1, 10), info_cp(8, 1, 15)];
        let multi = build_multi_series(&[("e1", l1.as_slice()), ("e2", l2.as_slice())]);
        assert_eq!(multi.len(), 2);
        assert_eq!(multi[0].engine_id, "e1");
        assert_eq!(multi[0].points.len(), 1);
        assert_eq!(multi[1].engine_id, "e2");
        assert_eq!(multi[1].points.len(), 2);
    }

    #[test]
    fn test_score_range_empty() {
        assert!(score_range(&[]).is_none());
    }

    #[test]
    fn test_score_range_basic() {
        let l = vec![info_cp(1, 1, -50), info_cp(2, 1, 0), info_cp(3, 1, 100)];
        let series = vec![build_series("e", &l)];
        assert_eq!(score_range(&series), Some((-50, 100)));
    }
}
