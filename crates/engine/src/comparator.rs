//! Parallel comparison of several UCI engines on the same position.
//!
//! [`EvalComparator`] allows submitting a position to N engines
//! simultaneously via OS threads, then collecting their results.
//!
//! Each engine runs in its own process; the threads are
//! independent and share no state.
//!
//! ## Parallelism strategy
//!
//! On each call to [`EvalComparator::compare`]:
//!
//! 1. The engines are extracted from the comparator (`mem::take`).
//! 2. Each engine is moved (`move`) into a dedicated thread.
//! 3. The threads are joined in launch order.
//! 4. The engines are reinserted into the comparator — it is therefore
//!    reusable after the call.
//!
//! If a thread panics (exceptional case), the corresponding engine
//! is lost but the others remain available.

use std::thread;

use uci::{
    engine::{AnalysisResult, EngineError, EnginePosition, UciEngine},
    protocol::GoLimits,
};

use crate::config::{ConfigError, EngineConfig};

/// Result of a thread from [`EvalComparator::compare`]: engine identifier,
/// the engine itself (reinserted afterward into the comparator), and the
/// analysis result (`clippy::type_complexity`, post-audit fixes from
/// 04/07/2026 — alias rather than an unreadable inline type).
type EngineJoinResult = (String, UciEngine, Result<AnalysisResult, EngineError>);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Comparator error.
#[derive(Debug)]
pub enum ComparatorError {
    /// An engine with this identifier is already registered.
    DuplicateId(String),
    /// Connection or communication error with the engine.
    Engine(EngineError),
    /// The engine configuration is invalid.
    Config(ConfigError),
}

impl std::fmt::Display for ComparatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) =>
                write!(f, "Identifiant déjà utilisé : '{id}'"),
            Self::Engine(e) =>
                write!(f, "Erreur moteur : {e}"),
            Self::Config(e) =>
                write!(f, "Configuration invalide : {e}"),
        }
    }
}

impl std::error::Error for ComparatorError {}

impl From<EngineError> for ComparatorError {
    fn from(e: EngineError) -> Self { Self::Engine(e) }
}

impl From<ConfigError> for ComparatorError {
    fn from(e: ConfigError) -> Self { Self::Config(e) }
}

// ---------------------------------------------------------------------------
// CompareResult
// ---------------------------------------------------------------------------

/// Result of an engine in a comparison.
#[derive(Debug)]
pub struct CompareResult {
    /// Engine identifier (as provided to [`EvalComparator::add`]).
    pub engine_id: String,
    /// Analysis result (or error if the engine failed).
    pub result:    Result<AnalysisResult, EngineError>,
}

impl CompareResult {
    /// Returns `true` if the analysis finished without error.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// Returns the best move (UCI) if the analysis succeeded.
    #[must_use]
    pub fn best_move(&self) -> Option<&str> {
        self.result.as_ref().ok().map(|r| r.best_move.as_str())
    }

    /// Returns the centipawn evaluation of the principal variation, if available.
    #[must_use]
    pub fn score_cp(&self) -> Option<i32> {
        use uci::parser::UciScore;
        self.result.as_ref().ok().and_then(|r| {
            r.principal_variation().and_then(|pv| {
                pv.score.as_ref().and_then(|s| {
                    if let UciScore::Centipawns(cp) = s { Some(*cp) } else { None }
                })
            })
        })
    }
}

// ---------------------------------------------------------------------------
// EvalComparator
// ---------------------------------------------------------------------------

/// Multi-engine comparator: analyzes the same position in parallel.
///
/// Unlike [`EnginePool`](crate::pool::EnginePool), the comparator
/// is designed to run simultaneous analyses and compare the results.
///
/// Engines are added via [`add`](EvalComparator::add) and then queried
/// via [`compare`](EvalComparator::compare).
#[derive(Default)]
pub struct EvalComparator {
    /// Active engines, each identified by a free-form name.
    engines: Vec<(String, UciEngine)>,
}

impl std::fmt::Debug for EvalComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalComparator")
            .field("engine_count", &self.engines.len())
            .field("ids", &self.engine_ids())
            .finish()
    }
}

impl EvalComparator {
    /// Creates an empty comparator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------------
    // Engine management
    // -----------------------------------------------------------------------

    /// Connects an engine and registers it under identifier `id`.
    ///
    /// # Errors
    ///
    /// - [`ComparatorError::DuplicateId`] if `id` is already present.
    /// - [`ComparatorError::Config`] if the binary is invalid.
    /// - [`ComparatorError::Engine`] if the connection or handshake fails.
    pub fn add(
        &mut self,
        id:     impl Into<String>,
        config: &EngineConfig,
    ) -> Result<(), ComparatorError> {
        let id = id.into();
        if self.engines.iter().any(|(eid, _)| eid == &id) {
            return Err(ComparatorError::DuplicateId(id));
        }
        config.validate()?;
        let path_str = config.path.to_str().unwrap_or_default();
        let mut engine = UciEngine::connect_with_timeout(path_str, config.init_timeout)?;
        for (name, value) in &config.options {
            engine.set_option(name, Some(value.as_str()))?;
        }
        self.engines.push((id, engine));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Number of registered engines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.engines.len()
    }

    /// Returns `true` if no engine is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    /// List of identifiers, sorted alphabetically.
    #[must_use]
    pub fn engine_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.engines.iter().map(|(id, _)| id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    // -----------------------------------------------------------------------
    // Parallel analysis
    // -----------------------------------------------------------------------

    /// Analyzes `position` with `limits` on all engines in parallel.
    ///
    /// Returns a vector of [`CompareResult`] in the same order as
    /// the engines were added.
    ///
    /// The engines are reinserted into the comparator after the analysis;
    /// [`compare`](Self::compare) can therefore be called multiple times.
    ///
    /// If a thread panics (exceptional case), the corresponding engine
    /// is lost but all others remain available.
    pub fn compare(
        &mut self,
        position: &EnginePosition,
        limits:   &GoLimits,
    ) -> Vec<CompareResult> {
        // Extract the engines to take ownership of them.
        let engines = std::mem::take(&mut self.engines);

        // Launch one thread per engine.
        let handles: Vec<thread::JoinHandle<EngineJoinResult>> = engines
            .into_iter()
            .map(|(id, mut engine)| {
                let pos = position.clone();
                let lim = limits.clone();
                thread::spawn(move || {
                    let result = engine.analyze(&pos, &lim);
                    (id, engine, result)
                })
            })
            .collect();

        // Join the threads and rebuild self.engines.
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Ok((id, engine, result)) = handle.join() {
                results.push(CompareResult { engine_id: id.clone(), result });
                self.engines.push((id, engine));
            }
            // Otherwise: thread panicked, the engine is lost; the other
            // engines remain available.
        }
        results
    }

    /// Convenience: returns only the best moves per engine.
    ///
    /// Each pair is `(engine_id, best_move_uci)`. Engines that
    /// returned an error are omitted.
    pub fn best_moves(
        &mut self,
        position: &EnginePosition,
        limits:   &GoLimits,
    ) -> Vec<(String, String)> {
        self.compare(position, limits)
            .into_iter()
            .filter_map(|cr| {
                cr.result.ok().map(|r| (cr.engine_id, r.best_move))
            })
            .collect()
    }
}

impl Drop for EvalComparator {
    /// Closes all engines in parallel (one thread per engine, like
    /// [`crate::pool::EnginePool::quit_all`]): avoids up to N×3 s of
    /// sequential blocking if several engines are slow to respond to
    /// `quit` (perf audit 02/07/2026, point 3).
    fn drop(&mut self) {
        let engines = std::mem::take(&mut self.engines);
        let handles: Vec<_> = engines
            .into_iter()
            .map(|(_, engine)| std::thread::spawn(move || engine.quit()))
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vendetta_path() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../../engines/vendetta_chess_motor");
        p
    }

    fn vendetta_config(name: &str) -> EngineConfig {
        EngineConfig::new(name, vendetta_path())
    }

    // -----------------------------------------------------------------------
    // Mock engine (Unix only)
    // -----------------------------------------------------------------------

    /// Builds a per-call temp file path, never reused across two calls —
    /// see [`create_mock_engine`]'s doc for why this matters (robustness
    /// audit 11/07/2026, follow-up: flaky `Disconnected` failures under
    /// `cargo test`'s default parallel execution).
    #[cfg(unix)]
    fn unique_mock_script_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("vendetta_mock_comp_{}_{n}.sh", std::process::id()))
    }

    #[cfg(unix)]
    fn create_mock_engine() -> String {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let script = r#"#!/bin/sh
while IFS= read -r line; do
    case "$line" in
        uci)
            echo "id name MockEngine"
            echo "id author TestSuite"
            echo "uciok"
            ;;
        isready)
            echo "readyok"
            ;;
        position*|ucinewgame) ;;
        go*)
            echo "info depth 1 score cp 30 nodes 100 time 5 pv e2e4"
            echo "bestmove e2e4 ponder e7e5"
            ;;
        stop) echo "bestmove e2e4" ;;
        quit) exit 0 ;;
    esac
done
"#;
        // Robustness audit 11/07/2026, follow-up: this used to be a single
        // fixed path (`vendetta_mock_comp.sh`) shared by every test in this
        // module. `cargo test` runs tests in parallel by default (several
        // OS threads within the same process), and nearly every test here
        // calls `mock_config` → this function, so multiple threads were
        // concurrently truncating and rewriting the *same* file on disk
        // while other threads' already-spawned `sh` processes were still
        // reading it as their script. A shell interpreter reading a script
        // file that is rewritten out from under it mid-execution can exit
        // early without ever reaching `uciok`, which closes its stdout pipe
        // and makes the reader thread's channel disconnect — observed as
        // `ComparatorError::Engine(EngineError::Process(ProcessError::Disconnected))`
        // from `EvalComparator::add`, intermittently and only under full
        // `cargo test` (never when a single test is run in isolation, which
        // is what made it easy to miss during manual review). Each call now
        // gets its own never-reused path via `unique_mock_script_path`.
        let path = unique_mock_script_path();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    fn mock_config(name: &str) -> EngineConfig {
        let path = create_mock_engine();
        EngineConfig::new(name, path)
    }

    // -----------------------------------------------------------------------
    // Tests without a real engine
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_is_empty() {
        let c = EvalComparator::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn test_add_invalid_path_fails() {
        let mut c = EvalComparator::new();
        let cfg = EngineConfig::new("Bad", "/nonexistent/engine");
        assert!(matches!(c.add("bad", &cfg), Err(ComparatorError::Config(_))));
        assert!(c.is_empty());
    }

    #[test]
    fn test_compare_empty_returns_empty() {
        let mut c = EvalComparator::new();
        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(10), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);
        assert!(results.is_empty());
    }

    #[test]
    fn test_comparator_error_display() {
        let e = ComparatorError::DuplicateId("v1".into());
        assert!(e.to_string().contains("v1"));
    }

    // -----------------------------------------------------------------------
    // Tests with mock engine (Unix)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn test_add_mock_engine() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock")).unwrap();
        assert_eq!(c.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_add_duplicate_fails() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock1")).unwrap();
        let result = c.add("m1", &mock_config("Mock2"));
        assert!(matches!(result, Err(ComparatorError::DuplicateId(_))));
        assert_eq!(c.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_engine_ids_sorted() {
        let mut c = EvalComparator::new();
        c.add("zebra", &mock_config("Z")).unwrap();
        c.add("alpha", &mock_config("A")).unwrap();
        assert_eq!(c.engine_ids(), vec!["alpha", "zebra"]);
    }

    #[cfg(unix)]
    #[test]
    fn test_compare_single_engine_one_result() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        assert_eq!(results.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_compare_result_engine_id_correct() {
        let mut c = EvalComparator::new();
        c.add("my_engine", &mock_config("Mock")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        assert_eq!(results[0].engine_id, "my_engine");
    }

    #[cfg(unix)]
    #[test]
    fn test_compare_result_is_ok() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        assert!(results[0].is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_compare_result_best_move_nonempty() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        let bm = results[0].best_move().unwrap();
        assert!(!bm.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_compare_two_engines_two_results() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock1")).unwrap();
        c.add("m2", &mock_config("Mock2")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(CompareResult::is_ok));
    }

    #[cfg(unix)]
    #[test]
    fn test_engines_still_alive_after_compare() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };

        // Two successive analyses
        let r1 = c.compare(&pos, &limits);
        let r2 = c.compare(&pos, &limits);

        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert!(r1[0].is_ok());
        assert!(r2[0].is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_best_moves_returns_pairs() {
        let mut c = EvalComparator::new();
        c.add("m1", &mock_config("Mock1")).unwrap();
        c.add("m2", &mock_config("Mock2")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };
        let bm = c.best_moves(&pos, &limits);

        assert_eq!(bm.len(), 2);
        assert!(bm.iter().all(|(_, mv)| !mv.is_empty()));
    }

    // -----------------------------------------------------------------------
    // Tests with a real engine (vendetta_chess_motor)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parallel_two_vendetta_engines() {
        if !vendetta_path().exists() { return; }
        let mut c = EvalComparator::new();
        c.add("v1", &vendetta_config("Vendetta-1")).unwrap();
        c.add("v2", &vendetta_config("Vendetta-2")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(100), ..GoLimits::default() };
        let results = c.compare(&pos, &limits);

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(CompareResult::is_ok));
    }

    #[test]
    fn test_parallel_both_return_valid_move() {
        if !vendetta_path().exists() { return; }
        let mut c = EvalComparator::new();
        c.add("v1", &vendetta_config("V1")).unwrap();
        c.add("v2", &vendetta_config("V2")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(100), ..GoLimits::default() };
        let bm = c.best_moves(&pos, &limits);

        assert_eq!(bm.len(), 2);
        for (_, mv) in &bm {
            // UCI format: letter+digit+letter+digit (e.g. e2e4)
            assert_eq!(mv.len(), 4, "coup UCI invalide : '{mv}'");
        }
    }

    #[test]
    fn test_parallel_reusable_after_compare() {
        if !vendetta_path().exists() { return; }
        let mut c = EvalComparator::new();
        c.add("v1", &vendetta_config("Vendetta")).unwrap();

        let pos    = EnginePosition::start();
        let limits = GoLimits { movetime: Some(50), ..GoLimits::default() };

        let r1 = c.compare(&pos, &limits);
        let r2 = c.compare(&pos, &limits);

        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert!(r1[0].is_ok());
        assert!(r2[0].is_ok());
    }
}
