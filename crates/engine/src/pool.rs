//! Pool of UCI engines.
//!
//! [`EnginePool`] manages a set of engines identified by a `String`.
//! Each engine is connected, configured, and kept active until
//! explicitly removed or the pool is shut down.
//!
//! ## Example
//!
//! ```ignore
//! let mut pool = EnginePool::new();
//! let cfg = EngineConfig::new("Vendetta", "/path/to/engine");
//! pool.add("v1", &cfg)?;
//!
//! let engine = pool.get_mut("v1")?;
//! let result = engine.analyze(&position, &limits)?;
//! ```

use std::collections::HashMap;

use uci::engine::{EngineError, UciEngine};

use crate::{
    config::{ConfigError, EngineConfig},
    handle::EngineHandle,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error from the engine pool.
#[derive(Debug)]
pub enum PoolError {
    /// An engine with this identifier is already present in the pool.
    DuplicateId(String),
    /// No engine matches this identifier.
    NotFound(String),
    /// Error while connecting to or communicating with the engine.
    Engine(EngineError),
    /// The engine configuration is invalid (path, permissions…).
    Config(ConfigError),
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) =>
                write!(f, "Identifiant déjà utilisé : '{id}'"),
            Self::NotFound(id) =>
                write!(f, "Moteur introuvable : '{id}'"),
            Self::Engine(e) =>
                write!(f, "Erreur moteur : {e}"),
            Self::Config(e) =>
                write!(f, "Configuration invalide : {e}"),
        }
    }
}

impl std::error::Error for PoolError {}

impl From<EngineError> for PoolError {
    fn from(e: EngineError) -> Self { Self::Engine(e) }
}

impl From<ConfigError> for PoolError {
    fn from(e: ConfigError) -> Self { Self::Config(e) }
}

// ---------------------------------------------------------------------------
// EnginePool
// ---------------------------------------------------------------------------

/// Pool of active UCI engines.
///
/// Engines are stored in a [`HashMap`] indexed by a free-form identifier
/// (chosen by the caller). Insertion order is not guaranteed.
#[derive(Default)]
pub struct EnginePool {
    engines: HashMap<String, UciEngine>,
}

impl std::fmt::Debug for EnginePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnginePool")
            .field("engine_count", &self.engines.len())
            .field("ids", &self.ids())
            .finish()
    }
}

impl EnginePool {
    /// Creates an empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects an engine from its configuration, adds it to the pool,
    /// and returns an [`EngineHandle`] for referencing it.
    ///
    /// # Errors
    ///
    /// - [`PoolError::DuplicateId`] if `id` is already present.
    /// - [`PoolError::Config`] if the binary is invalid or not executable.
    /// - [`PoolError::Engine`] if the connection or handshake fails.
    pub fn add(&mut self, id: impl Into<String>, config: &EngineConfig) -> Result<EngineHandle, PoolError> {
        let id = id.into();
        if self.engines.contains_key(&id) {
            return Err(PoolError::DuplicateId(id));
        }

        // Validate the config before attempting to launch the process.
        config.validate()?;

        // Connection + UCI handshake.
        let path_str = config.path.to_str().unwrap_or_default();
        let mut engine =
            UciEngine::connect_with_timeout(path_str, config.init_timeout)?;

        // Apply the UCI options.
        for (name, value) in &config.options {
            engine.set_option(name, Some(value.as_str()))?;
        }

        let handle = EngineHandle::new(id.clone(), config);
        self.engines.insert(id, engine);
        Ok(handle)
    }

    /// Sends `quit` to the engine and removes it from the pool.
    ///
    /// # Errors
    ///
    /// - [`PoolError::NotFound`] if `id` is absent from the pool.
    pub fn remove(&mut self, id: &str) -> Result<(), PoolError> {
        let engine = self
            .engines
            .remove(id)
            .ok_or_else(|| PoolError::NotFound(id.to_string()))?;
        engine.quit();
        Ok(())
    }

    /// Returns an immutable reference to the engine identified by `id`.
    ///
    /// # Errors
    ///
    /// - [`PoolError::NotFound`] if `id` is absent from the pool.
    pub fn get(&self, id: &str) -> Result<&UciEngine, PoolError> {
        self.engines
            .get(id)
            .ok_or_else(|| PoolError::NotFound(id.to_string()))
    }

    /// Returns a mutable reference to the engine identified by `id`.
    ///
    /// # Errors
    ///
    /// - [`PoolError::NotFound`] if `id` is absent from the pool.
    pub fn get_mut(&mut self, id: &str) -> Result<&mut UciEngine, PoolError> {
        self.engines
            .get_mut(id)
            .ok_or_else(|| PoolError::NotFound(id.to_string()))
    }

    /// Returns `true` if an engine with this identifier is present.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.engines.contains_key(id)
    }

    /// Returns the sorted list of identifiers of the active engines.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.engines.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    /// Number of active engines in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.engines.len()
    }

    /// Returns `true` if the pool contains no engines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    /// Sends `quit` to all engines and empties the pool.
    ///
    /// Individual errors are ignored — we try to cleanly close
    /// each engine even if one of them does not respond.
    ///
    /// Each engine is closed in its own thread rather than in
    /// sequence: `UciEngine::quit()` can block for up to ~3 s if the
    /// engine does not respond (`QUIT_WAIT_TIMEOUT`), so closing N slow
    /// engines in series would cost up to N×3 s instead of ~3 s total in
    /// parallel (perf audit 02/07/2026, point 3).
    pub fn quit_all(&mut self) {
        // quit() takes self by value → we empty the map to reclaim
        // ownership of each UciEngine.
        let engines = std::mem::take(&mut self.engines);
        let handles: Vec<_> = engines
            .into_values()
            .map(|engine| std::thread::spawn(move || engine.quit()))
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}

impl Drop for EnginePool {
    /// Closes all engines when the pool is destroyed (in parallel, see [`Self::quit_all`]).
    fn drop(&mut self) {
        self.quit_all();
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
    // Tests without a real engine (pool structure)
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_is_empty() {
        let pool = EnginePool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_contains_false_when_empty() {
        let pool = EnginePool::new();
        assert!(!pool.contains("v1"));
    }

    #[test]
    fn test_get_not_found() {
        let pool = EnginePool::new();
        assert!(matches!(pool.get("v1"), Err(PoolError::NotFound(_))));
    }

    #[test]
    fn test_get_mut_not_found() {
        let mut pool = EnginePool::new();
        assert!(matches!(pool.get_mut("v1"), Err(PoolError::NotFound(_))));
    }

    #[test]
    fn test_remove_not_found() {
        let mut pool = EnginePool::new();
        assert!(matches!(pool.remove("v1"), Err(PoolError::NotFound(_))));
    }

    #[test]
    fn test_add_invalid_path() {
        let mut pool = EnginePool::new();
        let cfg = EngineConfig::new("Bad", "/nonexistent/engine");
        assert!(matches!(pool.add("bad", &cfg), Err(PoolError::Config(_))));
        assert!(pool.is_empty());
    }

    #[test]
    fn test_ids_empty() {
        let pool = EnginePool::new();
        assert!(pool.ids().is_empty());
    }

    #[test]
    fn test_pool_error_display() {
        let e1 = PoolError::DuplicateId("v1".to_string());
        assert!(e1.to_string().contains("v1"));
        let e2 = PoolError::NotFound("v2".to_string());
        assert!(e2.to_string().contains("v2"));
    }

    // -----------------------------------------------------------------------
    // Tests with the real engine (skipped if the binary is absent)
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_and_contains() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        assert!(pool.contains("v1"));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_add_duplicate_fails() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        let result = pool.add("v1", &vendetta_config("Vendetta2"));
        assert!(matches!(result, Err(PoolError::DuplicateId(_))));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_add_two_engines() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta-1")).unwrap();
        pool.add("v2", &vendetta_config("Vendetta-2")).unwrap();
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.ids(), vec!["v1", "v2"]);
    }

    #[test]
    fn test_remove_existing() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        pool.remove("v1").unwrap();
        assert!(!pool.contains("v1"));
        assert!(pool.is_empty());
    }

    #[test]
    fn test_get_after_add() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        assert!(pool.get("v1").is_ok());
    }

    #[test]
    fn test_quit_all() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("v1", &vendetta_config("Vendetta-1")).unwrap();
        pool.add("v2", &vendetta_config("Vendetta-2")).unwrap();
        pool.quit_all();
        assert!(pool.is_empty());
    }

    #[test]
    fn test_ids_sorted() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        pool.add("zebra", &vendetta_config("Z")).unwrap();
        pool.add("alpha", &vendetta_config("A")).unwrap();
        assert_eq!(pool.ids(), vec!["alpha", "zebra"]);
    }
}
