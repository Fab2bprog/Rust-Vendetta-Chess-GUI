//! Public facade for the UCI engine.
//!
//! [`UciEngine`] assembles [`process`], [`parser`], [`protocol`], and [`state`]
//! into a high-level interface:
//!
//! ```text
//! UciEngine::connect(path)
//!     → UCI handshake (uci / uciok / isready / readyok)
//!     → analyze(position, limits) → Vec<UciInfo>  (MultiPV included)
//!     → stop() / quit()
//! ```
//!
//! ## `MultiPV`
//!
//! When the engine is configured with `MultiPV > 1`, it sends several
//! `info multipv N` lines per depth. [`UciEngine::analyze`] collects
//! all `info` lines up to `bestmove` and returns the full vector,
//! which the caller can filter by `multipv`.
//!
//! ## Timeouts
//!
//! Each line read is bounded by [`UciEngine::line_timeout`].
//! A timeout triggers [`EngineError::Timeout`] without killing the process.

use std::time::{Duration, Instant};

use crate::{
    parser::{parse_line, UciInfo, UciMessage, UciOption},
    process::{EngineProcess, ProcessError},
    protocol::{cmd_isready, cmd_uci, cmd_ucinewgame, cmd_setoption, GoLimits, cmd_go, cmd_stop, cmd_position_fen, cmd_position_startpos},
    state::UciStateMachine,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error from the UCI engine.
#[derive(Debug)]
pub enum EngineError {
    /// Communication error with the process.
    Process(ProcessError),
    /// Invalid state machine transition.
    InvalidState(String),
    /// Timeout while waiting for a response.
    Timeout,
    /// The engine did not send `uciok` during the handshake.
    HandshakeFailed,
    /// The engine did not send `readyok` after `isready`.
    NotReady,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Process(e)       => write!(f, "Erreur processus : {e}"),
            Self::InvalidState(s)  => write!(f, "État invalide : {s}"),
            Self::Timeout          => write!(f, "Timeout"),
            Self::HandshakeFailed  => write!(f, "Handshake UCI échoué"),
            Self::NotReady         => write!(f, "Moteur non prêt"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ProcessError> for EngineError {
    fn from(e: ProcessError) -> Self {
        match e {
            ProcessError::Timeout => Self::Timeout,
            other                 => Self::Process(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Position to analyze
// ---------------------------------------------------------------------------

/// Position to submit to the engine.
#[derive(Debug, Clone)]
pub enum EnginePosition {
    /// Standard starting position + UCI moves.
    StartPos { moves: Vec<String> },
    /// FEN position + UCI moves.
    Fen { fen: String, moves: Vec<String> },
}

impl EnginePosition {
    /// Starting position with no moves.
    #[must_use]
    pub fn start() -> Self {
        Self::StartPos { moves: Vec::new() }
    }

    /// Starting position with a sequence of moves.
    #[must_use]
    pub fn start_with_moves(moves: Vec<String>) -> Self {
        Self::StartPos { moves }
    }

    /// FEN position with no additional moves.
    #[must_use]
    pub fn from_fen(fen: impl Into<String>) -> Self {
        Self::Fen { fen: fen.into(), moves: Vec::new() }
    }

    /// FEN position with a sequence of moves.
    #[must_use]
    pub fn from_fen_with_moves(fen: impl Into<String>, moves: Vec<String>) -> Self {
        Self::Fen { fen: fen.into(), moves }
    }

    fn to_command(&self) -> String {
        match self {
            Self::StartPos { moves } => {
                let refs: Vec<&str> = moves.iter().map(String::as_str).collect();
                cmd_position_startpos(&refs)
            }
            Self::Fen { fen, moves } => {
                let refs: Vec<&str> = moves.iter().map(String::as_str).collect();
                cmd_position_fen(fen, &refs)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Result of an analysis
// ---------------------------------------------------------------------------

/// Result of a UCI analysis.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Best move (UCI).
    pub best_move: String,
    /// Ponder move suggested by the engine (optional).
    pub ponder:    Option<String>,
    /// All `info` lines received (`MultiPV` included).
    pub info_lines: Vec<UciInfo>,
}

impl AnalysisResult {
    /// Principal line (multipv = 1, or the last line received).
    #[must_use]
    pub fn principal_variation(&self) -> Option<&UciInfo> {
        // Prefer multipv=1; otherwise the last info with a non-empty pv
        self.info_lines
            .iter()
            .rev()
            .find(|i| i.multipv == Some(1) && !i.pv.is_empty())
            .or_else(|| self.info_lines.iter().rev().find(|i| !i.pv.is_empty()))
    }

    /// Lines matching a given `MultiPV` line number.
    ///
    /// When `n == 1`, lines without a `multipv` field are also included:
    /// UCI engines do not send `multipv 1` when MultiPV=1 is active.
    #[must_use]
    pub fn multipv_line(&self, n: u32) -> Vec<&UciInfo> {
        self.info_lines
            .iter()
            .filter(|i| i.multipv == Some(n) || (n == 1 && i.multipv.is_none()))
            .collect()
    }
}

/// Hard ceiling on the number of `info` lines accumulated by
/// [`UciEngine::analyze`]/[`UciEngine::stop`] before the oldest ones start
/// being evicted (robustness audit 11/07/2026, finding 2.5): without this
/// bound, a very verbose engine (high `MultiPV`, `currmove`/`refutation`
/// lines sent at every searched node) running for an extended time — a
/// slow time control is now correctly allowed to run long by the
/// `global_deadline` fix in [`UciEngine::analyze`] (finding 2.4), instead
/// of being cut off after a fixed 5 minutes — could accumulate an
/// unbounded number of `UciInfo` entries, each carrying its own PV
/// (`Vec<String>`), growing memory usage without limit. Set high enough
/// to never affect any realistic search: `MultiPV` rarely exceeds 5, and
/// an engine reporting updates for all 5 lines every 100 ms would still
/// take close to 3 minutes and a half to reach it. Every consumer of
/// `info_lines` (`AnalysisResult::principal_variation`/`multipv_line`)
/// already only cares about the *most recent* matching entry, so
/// dropping the oldest ones first never changes what a caller actually
/// observes once a search is long enough to hit the cap.
const MAX_INFO_LINES: usize = 10_000;

/// Pushes `info` onto `info_lines`, evicting the oldest half once
/// [`MAX_INFO_LINES`] is exceeded — see its doc for why. Evicts in one
/// batched [`Vec::drain`] rather than removing a single oldest entry per
/// line received past the cap: the latter would cost `O(MAX_INFO_LINES)`
/// of element shifting on *every subsequent line* for the rest of a long
/// search, instead of being amortized down to O(1) per line on average.
fn push_info_line(info_lines: &mut Vec<UciInfo>, info: UciInfo) {
    info_lines.push(info);
    if info_lines.len() > MAX_INFO_LINES {
        info_lines.drain(0..MAX_INFO_LINES / 2);
    }
}

// ---------------------------------------------------------------------------
// UciEngine
// ---------------------------------------------------------------------------

/// High-level facade for a UCI engine.
pub struct UciEngine {
    process:      EngineProcess,
    state:        UciStateMachine,
    /// Timeout per line read (default: 10 s).
    line_timeout: Duration,
    /// Options declared by the engine during the handshake.
    options:      Vec<UciOption>,
    /// Engine name.
    name:         Option<String>,
    /// Engine author.
    author:       Option<String>,
}

impl UciEngine {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Launches the engine at `path` and performs the full UCI handshake.
    ///
    /// Sequence: spawn → `uci` → wait for `uciok` → `isready` → wait for `readyok`.
    ///
    /// Default timeout: 5 s (per `uciok`/`readyok` phase, so ~10 s
    /// cumulative in the worst case). A real engine responds within a
    /// few milliseconds; this value is only meant to bound the wait in
    /// case of a misconfigured engine (perf audit 02/07/2026, point 7 —
    /// the old value of 10 s per phase could make the user wait ~20 s
    /// before reporting failure).
    ///
    /// # Errors
    ///
    /// - [`EngineError::Process`] if the binary cannot be found.
    /// - [`EngineError::HandshakeFailed`] if `uciok` does not arrive.
    /// - [`EngineError::NotReady`] if `readyok` does not arrive.
    pub fn connect(path: &str) -> Result<Self, EngineError> {
        Self::connect_with_timeout(path, Duration::from_secs(5))
    }

    /// Same as [`connect`] with a custom timeout.
    ///
    /// # Errors
    ///
    /// See [`connect`].
    pub fn connect_with_timeout(path: &str, timeout: Duration) -> Result<Self, EngineError> {
        let mut process = EngineProcess::spawn(path)?;
        let mut state   = UciStateMachine::new();

        // --- Handshake: uci → uciok ---
        process.send_command(&cmd_uci()).map_err(EngineError::Process)?;
        state.send_uci().map_err(|e| EngineError::InvalidState(e.to_string()))?;

        let mut options = Vec::new();
        let mut name    = None;
        let mut author  = None;

        // Global deadline: an engine that spams lines (info/option…)
        // without ever sending `uciok` must not block the handshake
        // indefinitely — each individual line respects its own
        // timeout, but without a global bound the loop never terminates
        // as long as lines keep arriving regularly. Symmetric to the
        // deadline already applied to the wait for `readyok` below.
        let uciok_deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= uciok_deadline {
                return Err(EngineError::HandshakeFailed);
            }
            let line = process.read_line_timeout(timeout)?;
            let msg  = parse_line(&line);

            match &msg {
                UciMessage::UciOk  => {
                    state.on_uciok().map_err(|e| EngineError::InvalidState(e.to_string()))?;
                    break;
                }
                UciMessage::IdName(n)   => name   = Some(n.clone()),
                UciMessage::IdAuthor(a) => author = Some(a.clone()),
                UciMessage::Option(opt) => options.push(opt.clone()),
                _ => {}
            }
        }

        if !state.is_ready() {
            return Err(EngineError::HandshakeFailed);
        }

        // --- isready → readyok ---
        process.send_command(&cmd_isready()).map_err(EngineError::Process)?;

        let readyok_deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= readyok_deadline {
                return Err(EngineError::NotReady);
            }
            let line = process.read_line_timeout(timeout)?;
            if parse_line(&line) == UciMessage::ReadyOk {
                break;
            }
        }

        Ok(Self { process, state, line_timeout: timeout, options, name, author })
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Engine name (set after the handshake).
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Engine author.
    #[must_use]
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// Options declared by the engine.
    #[must_use]
    pub fn options(&self) -> &[UciOption] {
        &self.options
    }

    /// Per-line timeout.
    #[must_use]
    pub fn line_timeout(&self) -> Duration {
        self.line_timeout
    }

    /// Changes the per-line timeout.
    pub fn set_line_timeout(&mut self, timeout: Duration) {
        self.line_timeout = timeout;
    }

    /// Latest stderr lines emitted by the engine process, useful for
    /// diagnosing a crash or error during analysis (the engine sometimes
    /// writes an explicit message to stderr just before dying).
    #[must_use]
    pub fn last_stderr_lines(&self) -> Vec<String> {
        self.process.last_stderr_lines()
    }

    // -----------------------------------------------------------------------
    // Configuration
    // -----------------------------------------------------------------------

    /// Configures an engine option (`setoption`).
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Process`] if the write fails.
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Result<(), EngineError> {
        self.process
            .send_command(&cmd_setoption(name, value))
            .map_err(EngineError::Process)
    }

    /// Sends `ucinewgame` to signal the start of a new game.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Process`] if the write fails.
    pub fn new_game(&mut self) -> Result<(), EngineError> {
        self.process
            .send_command(&cmd_ucinewgame())
            .map_err(EngineError::Process)
    }

    // -----------------------------------------------------------------------
    // Analysis
    // -----------------------------------------------------------------------

    /// Analyzes `position` with the `limits` given.
    ///
    /// Collects all `info` lines up to `bestmove` and returns
    /// an [`AnalysisResult`]. Supports `MultiPV` natively.
    ///
    /// # Errors
    ///
    /// - [`EngineError::InvalidState`] if the engine is not in the `Ready` state.
    /// - [`EngineError::Timeout`] if a line does not arrive within `line_timeout`.
    pub fn analyze(
        &mut self,
        position: &EnginePosition,
        limits:   &GoLimits,
    ) -> Result<AnalysisResult, EngineError> {
        if !self.state.is_ready() {
            return Err(EngineError::InvalidState("Le moteur n'est pas prêt".into()));
        }

        // Send position + go
        self.process
            .send_command(&position.to_command())
            .map_err(EngineError::Process)?;
        self.process
            .send_command(&cmd_go(limits))
            .map_err(EngineError::Process)?;

        self.state
            .start_thinking()
            .map_err(|e| EngineError::InvalidState(e.to_string()))?;

        // Collect lines up to bestmove
        let mut info_lines = Vec::new();
        let timeout = self.line_timeout;

        // Global timeout (robustness audit 11/07/2026, finding 2.4):
        // derived from whichever time information `limits` actually
        // carries, rather than a single fixed 5-minute fallback that used
        // to apply to *any* call without an explicit `movetime` —
        // including a real game at a slow time control, where
        // `build_go_limits` (GUI side) only ever sends `wtime`/`btime`/
        // `winc`/`binc`, never `movetime`. Under the old logic, a legal,
        // in-progress think longer than 5 minutes (a 30/90-minute game,
        // a critical position) hit this safety net, got `stop`ped, and
        // `analyze()` returned `EngineError::Timeout` — silently stalling
        // the game (the caller only logs the error, see
        // `game_bridge.rs`), with no move ever played.
        let global_deadline = {
            let grace = Duration::from_secs(10);
            let base = if let Some(ms) = limits.movetime {
                Duration::from_millis(ms) + grace
            } else if limits.infinite {
                // The caller is responsible for calling `stop()` once done
                // (infinite analysis mode — not yet exercised by the GUI
                // beyond `cmd_go`'s own tests). This safety net only
                // guarantees `analyze()` eventually returns if the caller
                // forgets to, not a cap on a legitimate long analysis —
                // hence the generous bound rather than 5 minutes.
                Duration::from_hours(6)
            } else if limits.wtime.is_some() || limits.btime.is_some() {
                // Clock-based search (real game with a time control): the
                // side actually thinking is only encoded in the FEN
                // already sent via `position.to_command()`, not directly
                // available here, so the *larger* of the two remaining
                // clocks is used as a safe upper bound — it can never be
                // smaller than the clock actually governing the engine's
                // own internal time management, so a legitimate long
                // think at a slow time control is never cut off early.
                // The 30 s grace period (vs. 10 s for `movetime`) absorbs
                // engine/OS scheduling overhead on top of a potentially
                // very large clock value.
                let clock_ms = limits.wtime.unwrap_or(0).max(limits.btime.unwrap_or(0));
                Duration::from_millis(clock_ms) + Duration::from_secs(30)
            } else {
                // No time information at all (bare `depth`/`nodes`/`mate`
                // search, or a caller-provided `GoLimits::default()`):
                // keep the previous fixed fallback.
                Duration::from_mins(5)
            };
            Instant::now() + base
        };

        loop {
            if Instant::now() >= global_deadline {
                // The engine is no longer responding: send stop and treat as a timeout.
                let _ = self.process.send_command(&cmd_stop());
                // Short drain attempt to avoid a corrupted state,
                // bounded by an absolute deadline (DRAIN_OVERALL_TIMEOUT)
                // rather than a simple iteration count: an engine
                // that keeps emitting lines regularly (without
                // ever sending back `bestmove`) must not be able to extend
                // the drain beyond this delay.
                drain_until_bestmove(&mut self.process, Duration::from_secs(2), DRAIN_OVERALL_TIMEOUT);
                let _ = self.state.on_bestmove(); // put the state machine back to Ready
                return Err(EngineError::Timeout);
            }

            let line = self.process.read_line_timeout(timeout)?;
            let msg  = parse_line(&line);

            match msg {
                UciMessage::Info(info) => {
                    push_info_line(&mut info_lines, info);
                }
                UciMessage::BestMove { mv, ponder } => {
                    self.state
                        .on_bestmove()
                        .map_err(|e| EngineError::InvalidState(e.to_string()))?;
                    return Ok(AnalysisResult { best_move: mv, ponder, info_lines });
                }
                _ => {}
            }
        }
    }

    /// Sends `stop` to interrupt an ongoing search.
    ///
    /// Waits for the `bestmove` the engine must send after `stop`.
    ///
    /// # Errors
    ///
    /// - [`EngineError::InvalidState`] if the engine is not in the `Thinking` state.
    /// - [`EngineError::Timeout`] if `bestmove` does not arrive.
    pub fn stop(&mut self) -> Result<AnalysisResult, EngineError> {
        if !self.state.is_thinking() {
            return Err(EngineError::InvalidState("Le moteur ne réfléchit pas".into()));
        }

        self.process
            .send_command(&cmd_stop())
            .map_err(EngineError::Process)?;

        // Read until bestmove
        let mut info_lines = Vec::new();
        let timeout = self.line_timeout;

        loop {
            let line = self.process.read_line_timeout(timeout)?;
            let msg  = parse_line(&line);

            match msg {
                UciMessage::Info(info) => push_info_line(&mut info_lines, info),
                UciMessage::BestMove { mv, ponder } => {
                    self.state
                        .on_bestmove()
                        .map_err(|e| EngineError::InvalidState(e.to_string()))?;
                    return Ok(AnalysisResult { best_move: mv, ponder, info_lines });
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Cleanly stops the engine (`quit`).
    pub fn quit(mut self) {
        // If thinking, send stop and drain up to bestmove (U3).
        // The drain is bounded by an absolute deadline (DRAIN_OVERALL_TIMEOUT)
        // rather than a fixed iteration count, to avoid an unbounded
        // accumulation of delays (see also `EngineProcess::quit`, which in
        // turn bounds the wait for the process to terminate).
        if self.state.is_thinking() {
            let _ = self.process.send_command(&cmd_stop());
            drain_until_bestmove(&mut self.process, Duration::from_millis(500), DRAIN_OVERALL_TIMEOUT);
        }
        self.process.quit();
    }
}

/// Maximum total delay granted to draining engine lines while waiting for a
/// `bestmove` (after a `stop` sent following a timeout, or during `quit`).
/// Absolute bound: independent of the number of lines received in the meantime.
const DRAIN_OVERALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads engine lines until a `bestmove` is received, an error occurs, or
/// until `overall_timeout` is exceeded (protection against an engine that
/// keeps emitting lines without ever sending back `bestmove`).
fn drain_until_bestmove(process: &mut EngineProcess, per_line_timeout: Duration, overall_timeout: Duration) {
    let deadline = Instant::now() + overall_timeout;
    loop {
        if Instant::now() >= deadline {
            return;
        }
        match process.read_line_timeout(per_line_timeout) {
            Ok(line) if matches!(parse_line(&line), UciMessage::BestMove { .. }) => return,
            Err(_) => return,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::UciScore;

    // -----------------------------------------------------------------------
    // Fake engine (mock) via a shell script
    // -----------------------------------------------------------------------
    //
    // We use a minimal shell script that simulates UCI responses.
    // The script reads stdin and responds according to the protocol.

    #[cfg(unix)]
    fn mock_engine_script() -> String {
        // Shell script simulating a minimal UCI engine
        r#"#!/bin/sh
while IFS= read -r line; do
    case "$line" in
        uci)
            echo "id name MockEngine"
            echo "id author TestSuite"
            echo "option name Hash type spin default 16 min 1 max 1024"
            echo "uciok"
            ;;
        isready)
            echo "readyok"
            ;;
        ucinewgame)
            ;;
        position*)
            ;;
        go*)
            echo "info depth 1 score cp 30 nodes 100 time 1 pv e2e4"
            echo "info depth 2 score cp 25 nodes 500 time 5 pv e2e4 e7e5"
            echo "bestmove e2e4 ponder e7e5"
            ;;
        stop)
            echo "bestmove e2e4"
            ;;
        quit)
            exit 0
            ;;
    esac
done
"#
        .to_owned()
    }

    /// Per-call temp file path, never reused across two calls — see
    /// [`create_mock_engine`]'s doc for why (robustness audit 11/07/2026,
    /// follow-up: same flaky-`Disconnected` bug as `comparator.rs`'s own
    /// mock engine fixture, fixed the same way here).
    #[cfg(unix)]
    fn unique_mock_script_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("vendetta_mock_engine_{}_{n}.sh", std::process::id()))
    }

    #[cfg(unix)]
    fn create_mock_engine() -> String {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        // Robustness audit 11/07/2026, follow-up: was a single fixed path
        // (`vendetta_mock_engine.sh`) shared by every test below. Under
        // `cargo test`'s default parallel execution, several of the ~7
        // tests here call this function concurrently, each truncating and
        // rewriting the same file while another thread's already-spawned
        // `sh` was still reading it as its script — a shell interpreter can
        // exit early on a script rewritten out from under it, closing its
        // stdout pipe and surfacing as an intermittent
        // `EngineError::Process(ProcessError::Disconnected)` during the
        // handshake. Each call now gets its own never-reused path.
        let path = unique_mock_script_path();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(mock_engine_script().as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn test_connect_mock_engine() {
        let path = create_mock_engine();
        let engine = UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        assert!(engine.state.is_ready());
        assert_eq!(engine.name(), Some("MockEngine"));
        assert_eq!(engine.author(), Some("TestSuite"));
        assert_eq!(engine.options().len(), 1);
        assert_eq!(engine.options()[0].name, "Hash");

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_returns_bestmove() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { depth: Some(2), ..GoLimits::default() };
        let result = engine.analyze(&pos, &limits).unwrap();

        assert_eq!(result.best_move, "e2e4");
        assert_eq!(result.ponder.as_deref(), Some("e7e5"));
        assert!(engine.state.is_ready());

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_collects_info_lines() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { depth: Some(2), ..GoLimits::default() };
        let result = engine.analyze(&pos, &limits).unwrap();

        assert_eq!(result.info_lines.len(), 2);
        assert_eq!(result.info_lines[0].depth, Some(1));
        assert_eq!(result.info_lines[1].depth, Some(2));
        assert_eq!(result.info_lines[0].score, Some(UciScore::Centipawns(30)));

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_principal_variation() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { depth: Some(2), ..GoLimits::default() };
        let result = engine.analyze(&pos, &limits).unwrap();

        // The principal PV is the last info line with a non-empty pv
        let pv = result.principal_variation().unwrap();
        assert_eq!(pv.pv, ["e2e4", "e7e5"]);

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_from_fen() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = EnginePosition::from_fen(fen);
        let limits = GoLimits { movetime: Some(10), ..GoLimits::default() };
        let result = engine.analyze(&pos, &limits).unwrap();

        assert!(!result.best_move.is_empty());

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_set_option() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        // setoption doesn't trigger a response from the mock → just check it doesn't crash
        engine.set_option("Hash", Some("64")).unwrap();
        assert!(engine.state.is_ready());

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_new_game() {
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        engine.new_game().unwrap();
        assert!(engine.state.is_ready());

        engine.quit();
    }

    #[cfg(unix)]
    #[test]
    fn test_analyze_twice() {
        // Checks that two analyses can be chained (state machine Ready → Thinking → Ready → Thinking → Ready)
        let path = create_mock_engine();
        let mut engine =
            UciEngine::connect_with_timeout(&path, Duration::from_secs(3)).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { depth: Some(1), ..GoLimits::default() };

        let r1 = engine.analyze(&pos, &limits).unwrap();
        let r2 = engine.analyze(&pos, &limits).unwrap();

        assert_eq!(r1.best_move, "e2e4");
        assert_eq!(r2.best_move, "e2e4");
        assert!(engine.state.is_ready());

        engine.quit();
    }

    #[test]
    fn test_connect_invalid_path() {
        let result = UciEngine::connect("/bin/moteur_inexistant");
        assert!(result.is_err());
    }

    // --- EnginePosition ---

    #[test]
    fn test_engine_position_startpos() {
        let pos = EnginePosition::start();
        assert_eq!(pos.to_command(), "position startpos");
    }

    #[test]
    fn test_engine_position_startpos_moves() {
        let pos = EnginePosition::start_with_moves(vec!["e2e4".into(), "e7e5".into()]);
        assert_eq!(pos.to_command(), "position startpos moves e2e4 e7e5");
    }

    #[test]
    fn test_engine_position_fen() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = EnginePosition::from_fen(fen);
        assert_eq!(pos.to_command(), format!("position fen {fen}"));
    }

    #[test]
    fn test_engine_position_fen_with_moves() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = EnginePosition::from_fen_with_moves(fen, vec!["e7e5".into()]);
        assert_eq!(pos.to_command(), format!("position fen {fen} moves e7e5"));
    }
}
