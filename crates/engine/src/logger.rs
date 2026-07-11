//! Logger for UCI analysis sessions.
//!
//! [`SessionLogger`] writes analyses to a structured text file in
//! **append** mode: several sessions can coexist in the same
//! file, each delimited by a header and a footer.
//!
//! ## File format
//!
//! ```text
//! ============================================================
//! SESSION  ts=1705329000
//! ============================================================
//!
//! [+87ms] ANALYSIS #1
//!   engine   : Vendetta
//!   position : startpos
//!   bestmove : e2e4
//!   score    : +30 cp
//!   depth    : 12
//!   nodes    : 125840
//!   pv       : e2e4 e7e5 Nf3 Nc6
//!
//! [+889ms] SESSION END — 1 analysis — total 889 ms
//! ============================================================
//! ```
//!
//! ## Typical usage
//!
//! ```ignore
//! let mut logger = SessionLogger::open("sessions.log")?;
//! logger.log_start(&["Vendetta", "Stockfish"])?;
//!
//! let t0 = Instant::now();
//! let result = engine.analyze(&pos, &limits)?;
//! logger.log_analysis("Vendetta", &pos, &result, t0.elapsed())?;
//!
//! logger.log_end()?;
//! ```

use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use uci::{
    engine::{AnalysisResult, EnginePosition},
    parser::UciScore,
};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Logger error (I/O only).
#[derive(Debug)]
pub enum LogError {
    /// Input/output error.
    Io(io::Error),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Erreur I/O : {e}"),
        }
    }
}

impl std::error::Error for LogError {}

impl From<io::Error> for LogError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

// ---------------------------------------------------------------------------
// SessionLogger
// ---------------------------------------------------------------------------

/// Logger for UCI analysis sessions to a text file.
///
/// Sessions are **appended** to the existing file.
/// Calling [`log_end`](SessionLogger::log_end) finalizes the session and
/// forces the write to disk.
pub struct SessionLogger {
    /// Path to the log file.
    path:       PathBuf,
    /// Write buffer.
    writer:     BufWriter<File>,
    /// Instant the session was created (for relative time).
    started_at: Instant,
    /// Unix timestamp of the session (seconds since the epoch).
    epoch_secs: u64,
    /// Number of analyses recorded since the start of the session.
    count:      usize,
}

impl std::fmt::Debug for SessionLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `writer`/`started_at`/`epoch_secs` deliberately omitted
        // (clippy::missing_fields_in_debug, post-audit fixes from
        // 04/07/2026): not very useful to display (write buffer, internal
        // instants) — `finish_non_exhaustive` honestly signals that this
        // `Debug` is partial rather than pretending otherwise.
        f.debug_struct("SessionLogger")
            .field("path", &self.path)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl SessionLogger {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Opens or creates `path` in append mode and initializes the session.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Io`] if the file cannot be opened.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, LogError> {
        let path = path.into();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let epoch_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(Self {
            path,
            writer:     BufWriter::new(file),
            started_at: Instant::now(),
            epoch_secs,
            count:      0,
        })
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Path to the log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of analyses recorded in this session.
    #[must_use]
    pub fn analysis_count(&self) -> usize {
        self.count
    }

    /// Duration elapsed since the session was opened.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    // -----------------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------------

    /// Writes the session header along with the optional list of engines.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Io`] on a write error.
    pub fn log_start(&mut self, engine_ids: &[&str]) -> Result<(), LogError> {
        let sep = "=".repeat(60);
        writeln!(self.writer, "\n{sep}")?;
        writeln!(self.writer, "SESSION  ts={}", self.epoch_secs)?;
        if !engine_ids.is_empty() {
            writeln!(self.writer, "Engines: {}", engine_ids.join(", "))?;
        }
        writeln!(self.writer, "{sep}")?;
        Ok(())
    }

    /// Records an analysis and its result.
    ///
    /// - `engine_id`: engine identifier (as displayed in the UI).
    /// - `position`:  position submitted.
    /// - `result`:    result returned by [`UciEngine::analyze`].
    /// - `elapsed`:   actual duration of the analysis (measured by the caller).
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Io`] on a write error.
    pub fn log_analysis(
        &mut self,
        engine_id: &str,
        position:  &EnginePosition,
        result:    &AnalysisResult,
        elapsed:   Duration,
    ) -> Result<(), LogError> {
        self.count += 1;
        let ms      = elapsed.as_millis();
        let pos_str = fmt_position(position);
        let score   = fmt_score(result);
        let depth   = result.principal_variation()
            .and_then(|pv| pv.depth)
            .map_or_else(|| "?".to_string(), |d| d.to_string());
        let nodes   = result.principal_variation()
            .and_then(|pv| pv.nodes)
            .map_or_else(|| "?".to_string(), |n| n.to_string());
        let pv_str  = result.principal_variation()
            .map(|pv| pv.pv.join(" "))
            .unwrap_or_default();

        writeln!(self.writer, "\n[+{ms}ms] ANALYSIS #{}", self.count)?;
        writeln!(self.writer, "  engine   : {engine_id}")?;
        writeln!(self.writer, "  position : {pos_str}")?;
        writeln!(self.writer, "  bestmove : {}", result.best_move)?;
        if !score.is_empty() {
            writeln!(self.writer, "  score    : {score}")?;
        }
        writeln!(self.writer, "  depth    : {depth}")?;
        writeln!(self.writer, "  nodes    : {nodes}")?;
        if !pv_str.is_empty() {
            writeln!(self.writer, "  pv       : {pv_str}")?;
        }
        Ok(())
    }

    /// Writes the session footer and forces a flush.
    ///
    /// After this call, the session is finished and the file is
    /// synced to disk.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Io`] on a write or flush error.
    pub fn log_end(&mut self) -> Result<(), LogError> {
        let total_ms = self.started_at.elapsed().as_millis();
        let sep      = "=".repeat(60);
        let plural   = if self.count == 1 { "analysis" } else { "analyses" };
        writeln!(self.writer,
            "\n[+{total_ms}ms] SESSION END — {} {plural}",
            self.count
        )?;
        writeln!(self.writer, "{sep}\n")?;
        self.flush()
    }

    /// Forces the buffer to be written to disk.
    ///
    /// # Errors
    ///
    /// Returns [`LogError::Io`] on a flush error.
    pub fn flush(&mut self) -> Result<(), LogError> {
        self.writer.flush().map_err(LogError::Io)
    }
}

impl Drop for SessionLogger {
    /// Automatic flush on destruction (best-effort, errors ignored).
    fn drop(&mut self) {
        let _ = self.writer.flush();
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Text representation of a position.
fn fmt_position(pos: &EnginePosition) -> String {
    match pos {
        EnginePosition::StartPos { moves } if moves.is_empty() =>
            "startpos".to_string(),
        EnginePosition::StartPos { moves } =>
            format!("startpos moves {}", moves.join(" ")),
        EnginePosition::Fen { fen, moves } if moves.is_empty() =>
            fen.clone(),
        EnginePosition::Fen { fen, moves } =>
            format!("{fen} moves {}", moves.join(" ")),
    }
}

/// Text representation of the principal line's score.
fn fmt_score(result: &AnalysisResult) -> String {
    result
        .principal_variation()
        .and_then(|pv| pv.score.as_ref())
        .map(|s| match s {
            UciScore::Centipawns(cp) => {
                if *cp >= 0 { format!("+{cp} cp") } else { format!("{cp} cp") }
            }
            UciScore::Mate(m)       => format!("M{m}"),
            UciScore::Lowerbound(v) => format!("{v} cp (lb)"),
            UciScore::Upperbound(v) => format!("{v} cp (ub)"),
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uci::{
        engine::{AnalysisResult, EnginePosition},
        parser::{UciInfo, UciScore},
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn make_result(best_move: &str) -> AnalysisResult {
        AnalysisResult {
            best_move:  best_move.to_string(),
            ponder:     None,
            info_lines: vec![],
        }
    }

    fn make_result_with_info(best_move: &str, depth: u32, cp: i32, pv: &[&str]) -> AnalysisResult {
        AnalysisResult {
            best_move:  best_move.to_string(),
            ponder:     None,
            info_lines: vec![UciInfo {
                depth:  Some(depth),
                score:  Some(UciScore::Centipawns(cp)),
                multipv: Some(1),
                nodes:  Some(125_840),
                pv:     pv.iter().map(std::string::ToString::to_string).collect(),
                ..UciInfo::default()
            }],
        }
    }

    fn read_log(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_open_creates_file() {
        let p = tmp_path("vendetta_logger_create.log");
        let _ = fs::remove_file(&p);
        let _logger = SessionLogger::open(&p).unwrap();
        assert!(p.exists());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_path_accessor() {
        let p = tmp_path("vendetta_logger_path.log");
        let logger = SessionLogger::open(&p).unwrap();
        assert_eq!(logger.path(), p.as_path());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_count_starts_at_zero() {
        let p = tmp_path("vendetta_logger_count0.log");
        let logger = SessionLogger::open(&p).unwrap();
        assert_eq!(logger.analysis_count(), 0);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_start_writes_session_header() {
        let p = tmp_path("vendetta_logger_start.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        logger.log_start(&["Vendetta"]).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("SESSION"), "attendu 'SESSION' dans : {content}");
        assert!(content.contains("Vendetta"), "attendu 'Vendetta' dans : {content}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_start_empty_engines() {
        let p = tmp_path("vendetta_logger_noeng.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        logger.log_start(&[]).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("SESSION"));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_analysis_increments_count() {
        let p = tmp_path("vendetta_logger_cnt.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        let pos    = EnginePosition::start();
        let result = make_result("e2e4");
        logger.log_analysis("V", &pos, &result, Duration::from_millis(50)).unwrap();
        assert_eq!(logger.analysis_count(), 1);
        logger.log_analysis("V", &pos, &result, Duration::from_millis(60)).unwrap();
        assert_eq!(logger.analysis_count(), 2);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_analysis_writes_bestmove() {
        let p = tmp_path("vendetta_logger_bm.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        let pos    = EnginePosition::start();
        let result = make_result("e2e4");
        logger.log_analysis("V", &pos, &result, Duration::from_millis(50)).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("e2e4"), "attendu 'e2e4' dans : {content}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_analysis_writes_engine_id() {
        let p = tmp_path("vendetta_logger_eng.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        let pos    = EnginePosition::start();
        let result = make_result("e2e4");
        logger.log_analysis("MyEngine", &pos, &result, Duration::from_millis(50)).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("MyEngine"));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_analysis_with_score_and_pv() {
        let p = tmp_path("vendetta_logger_score.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        let pos    = EnginePosition::start();
        let result = make_result_with_info("e2e4", 12, 30, &["e2e4", "e7e5"]);
        logger.log_analysis("V", &pos, &result, Duration::from_millis(87)).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("+30 cp"), "score attendu dans : {content}");
        assert!(content.contains("e7e5"),  "PV attendue dans : {content}");
        assert!(content.contains("12"),    "profondeur attendue dans : {content}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_end_writes_footer() {
        let p = tmp_path("vendetta_logger_end.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        logger.log_start(&[]).unwrap();
        logger.log_end().unwrap();
        let content = read_log(&p);
        assert!(content.contains("SESSION END"), "attendu 'SESSION END' dans : {content}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_log_full_session() {
        let p = tmp_path("vendetta_logger_full.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        logger.log_start(&["Vendetta"]).unwrap();
        let pos    = EnginePosition::start();
        let result = make_result_with_info("e2e4", 10, 25, &["e2e4", "e7e5"]);
        logger.log_analysis("Vendetta", &pos, &result, Duration::from_millis(100)).unwrap();
        logger.log_end().unwrap();
        let content = read_log(&p);
        assert!(content.contains("SESSION"));
        assert!(content.contains("ANALYSIS #1"));
        assert!(content.contains("SESSION END"));
        assert!(content.contains("1 analysis"));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_multiple_sessions_append() {
        let p = tmp_path("vendetta_logger_append.log");
        let _ = fs::remove_file(&p);

        // First session
        {
            let mut l = SessionLogger::open(&p).unwrap();
            l.log_start(&["V1"]).unwrap();
            l.log_end().unwrap();
        }
        // Second session (append)
        {
            let mut l = SessionLogger::open(&p).unwrap();
            l.log_start(&["V2"]).unwrap();
            l.log_end().unwrap();
        }

        let content = read_log(&p);
        let count = content.matches("SESSION END").count();
        assert_eq!(count, 2, "attendu 2 fins de session dans : {content}");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_flush_no_error() {
        let p = tmp_path("vendetta_logger_flush.log");
        let mut logger = SessionLogger::open(&p).unwrap();
        assert!(logger.flush().is_ok());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_fen_position_display() {
        let p  = tmp_path("vendetta_logger_fen.log");
        let _ = fs::remove_file(&p);
        let mut logger = SessionLogger::open(&p).unwrap();
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = EnginePosition::from_fen(fen);
        let result = make_result("e7e5");
        logger.log_analysis("V", &pos, &result, Duration::from_millis(10)).unwrap();
        logger.flush().unwrap();
        let content = read_log(&p);
        assert!(content.contains("RNBQKBNR"), "FEN attendue dans : {content}");
        let _ = fs::remove_file(&p);
    }
}
