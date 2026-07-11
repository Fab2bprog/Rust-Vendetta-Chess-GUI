//! Typed reference to an engine in the [`EnginePool`](crate::pool::EnginePool).
//!
//! An [`EngineHandle`] is returned by [`EnginePool::add`] when an engine
//! connects. It represents a lightweight "ticket" that can be kept,
//! cloned, or passed around without borrowing the pool.
//!
//! It allows you to:
//! - find an engine in the pool (`pool.get(handle.id())`)
//! - check that it is still active (`handle.is_alive(&pool)`)
//! - inspect its metadata (name, path, options)

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{config::EngineConfig, pool::EnginePool};

// ---------------------------------------------------------------------------
// EngineHandle
// ---------------------------------------------------------------------------

/// Lightweight reference to an engine registered in the pool.
///
/// Created by [`EnginePool::add`]; can be cloned and passed around freely.
/// Does not hold the engine itself — any analysis operation must go
/// through the pool.
#[derive(Debug, Clone)]
pub struct EngineHandle {
    /// Unique identifier in the pool.
    id:           String,
    /// Displayed name (extracted from the config).
    name:         String,
    /// Binary path (extracted from the config).
    path:         PathBuf,
    /// Number of configured UCI options.
    option_count: usize,
    /// Instant when the engine was added to the pool.
    created_at:   Instant,
}

impl EngineHandle {
    /// Creates a handle from the identifier and config used
    /// in the call to [`EnginePool::add`].
    #[must_use]
    pub(crate) fn new(id: String, config: &EngineConfig) -> Self {
        Self {
            id,
            name:         config.name.clone(),
            path:         config.path.clone(),
            option_count: config.options.len(),
            created_at:   Instant::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Engine identifier in the pool.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Displayed name of the engine.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Binary path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of UCI options sent to the engine during initialization.
    #[must_use]
    pub fn option_count(&self) -> usize {
        self.option_count
    }

    /// Duration elapsed since the engine was added to the pool.
    #[must_use]
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    // -----------------------------------------------------------------------
    // Status
    // -----------------------------------------------------------------------

    /// Returns `true` if the referenced engine is still active in the pool.
    ///
    /// A handle remains valid as long as `pool.remove(id)` has not been called.
    #[must_use]
    pub fn is_alive(&self, pool: &EnginePool) -> bool {
        pool.contains(&self.id)
    }
}

impl std::fmt::Display for EngineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineHandle {{ id: '{}', name: '{}' }}", self.id, self.name)
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

    fn make_handle(id: &str, name: &str) -> EngineHandle {
        let cfg = EngineConfig::new(name, "/tmp/fake");
        EngineHandle::new(id.to_string(), &cfg)
    }

    // -----------------------------------------------------------------------
    // Tests without a real engine
    // -----------------------------------------------------------------------

    #[test]
    fn test_handle_id() {
        let h = make_handle("v1", "Vendetta");
        assert_eq!(h.id(), "v1");
    }

    #[test]
    fn test_handle_name() {
        let h = make_handle("v1", "Vendetta");
        assert_eq!(h.name(), "Vendetta");
    }

    #[test]
    fn test_handle_path() {
        let cfg = EngineConfig::new("Test", "/tmp/myengine");
        let h = EngineHandle::new("t1".to_string(), &cfg);
        assert_eq!(h.path(), Path::new("/tmp/myengine"));
    }

    #[test]
    fn test_handle_option_count_zero() {
        let h = make_handle("v1", "Vendetta");
        assert_eq!(h.option_count(), 0);
    }

    #[test]
    fn test_handle_option_count_nonzero() {
        let cfg = EngineConfig::builder("Test", "/tmp/fake")
            .option("Hash", "128")
            .option("Threads", "2")
            .build();
        let h = EngineHandle::new("t1".to_string(), &cfg);
        assert_eq!(h.option_count(), 2);
    }

    #[test]
    fn test_handle_age_nonnegative() {
        let h = make_handle("v1", "Vendetta");
        assert!(h.age() < Duration::from_secs(1));
    }

    #[test]
    fn test_handle_clone() {
        let h = make_handle("v1", "Vendetta");
        let h2 = h.clone();
        assert_eq!(h.id(), h2.id());
        assert_eq!(h.name(), h2.name());
    }

    #[test]
    fn test_handle_display() {
        let h = make_handle("v1", "Vendetta");
        let s = h.to_string();
        assert!(s.contains("v1"));
        assert!(s.contains("Vendetta"));
    }

    // -----------------------------------------------------------------------
    // Tests with a real engine
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_alive_true() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        let handle = pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        assert!(handle.is_alive(&pool));
    }

    #[test]
    fn test_is_alive_false_after_remove() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        let handle = pool.add("v1", &vendetta_config("Vendetta")).unwrap();
        pool.remove("v1").unwrap();
        assert!(!handle.is_alive(&pool));
    }

    #[test]
    fn test_is_alive_false_after_quit_all() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        let h1 = pool.add("v1", &vendetta_config("V1")).unwrap();
        let h2 = pool.add("v2", &vendetta_config("V2")).unwrap();
        pool.quit_all();
        assert!(!h1.is_alive(&pool));
        assert!(!h2.is_alive(&pool));
    }

    #[test]
    fn test_handle_from_add_has_correct_id() {
        if !vendetta_path().exists() { return; }
        let mut pool = EnginePool::new();
        let handle = pool.add("motor", &vendetta_config("Vendetta")).unwrap();
        assert_eq!(handle.id(), "motor");
        assert_eq!(handle.name(), "Vendetta");
    }
}
