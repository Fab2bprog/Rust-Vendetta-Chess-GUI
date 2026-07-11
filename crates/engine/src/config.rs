//! Configuration of a UCI engine.
//!
//! [`EngineConfig`] groups everything needed to launch and configure an
//! engine: binary path, UCI options to send at startup, and default
//! search limits.

use std::{collections::HashMap, path::{Path, PathBuf}, time::Duration};

use uci::protocol::GoLimits;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Full configuration of a UCI engine.
///
/// Built via [`EngineConfigBuilder`], or directly if the default values
/// are suitable.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Name displayed in the interface (free-form, not necessarily the UCI name).
    pub name:           String,
    /// Absolute path to the engine binary.
    pub path:           PathBuf,
    /// UCI options to send after initialization (`setoption`).
    /// Key = option name, value = value as a string.
    pub options:        HashMap<String, String>,
    /// Search limits used when no limit is explicitly specified
    /// during an analysis.
    pub default_limits: GoLimits,
    /// Maximum delay to receive `uciok` during initialization.
    pub init_timeout:   Duration,
}

impl EngineConfig {
    /// Creates a minimal configuration with default values.
    ///
    /// - No pre-configured UCI options.
    /// - Default limit: `movetime = 1 000 ms`.
    /// - Initialization timeout: 5 s.
    ///
    /// Aligned with [`uci::engine::UciEngine::connect`] (also 5 s) —
    /// both entry points for connecting to a UCI engine now share
    /// the same default value, to avoid any confusion.
    /// A real engine responds within a few milliseconds; 5 s leaves a
    /// comfortable margin (loading NNUE tables, large hash) while still
    /// avoiding making the user wait ~20 s (uciok + readyok)
    /// before reporting a misconfigured engine (perf audit 02/07/2026,
    /// point 7).
    #[must_use]
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name:           name.into(),
            path:           path.into(),
            options:        HashMap::new(),
            default_limits: GoLimits {
                movetime: Some(1_000),
                ..GoLimits::default()
            },
            init_timeout: Duration::from_secs(5),
        }
    }

    /// Returns a builder for fluent construction.
    #[must_use]
    pub fn builder(name: impl Into<String>, path: impl Into<PathBuf>) -> EngineConfigBuilder {
        EngineConfigBuilder::new(name, path)
    }

    /// Checks that the binary exists and is executable.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::PathNotFound`] if the file does not exist,
    /// or [`ConfigError::NotExecutable`] if it is not executable.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let path = &self.path;
        if !path.exists() {
            return Err(ConfigError::PathNotFound(path.clone()));
        }
        if !is_executable(path) {
            return Err(ConfigError::NotExecutable(path.clone()));
        }
        Ok(())
    }

    /// Adds or replaces a UCI option.
    pub fn set_option(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.options.insert(name.into(), value.into());
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Fluent builder for [`EngineConfig`].
#[derive(Debug)]
pub struct EngineConfigBuilder {
    config: EngineConfig,
}

impl EngineConfigBuilder {
    /// Creates a new builder with default values.
    #[must_use]
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            config: EngineConfig::new(name, path),
        }
    }

    /// Sets a UCI option (can be called multiple times).
    #[must_use]
    pub fn option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.options.insert(name.into(), value.into());
        self
    }

    /// Sets the default search limits.
    #[must_use]
    pub fn default_limits(mut self, limits: GoLimits) -> Self {
        self.config.default_limits = limits;
        self
    }

    /// Sets the UCI initialization timeout.
    #[must_use]
    pub fn init_timeout(mut self, timeout: Duration) -> Self {
        self.config.init_timeout = timeout;
        self
    }

    /// Builds the final configuration.
    #[must_use]
    pub fn build(self) -> EngineConfig {
        self.config
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Configuration validation error.
#[derive(Debug)]
pub enum ConfigError {
    /// The binary does not exist at this path.
    PathNotFound(PathBuf),
    /// The file exists but is not executable.
    NotExecutable(PathBuf),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathNotFound(p)  =>
                write!(f, "Moteur introuvable : {}", p.display()),
            Self::NotExecutable(p) =>
                write!(f, "Fichier non exécutable : {}", p.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Checks whether a file is executable (Unix) or simply exists (Windows).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        // On Windows, existence is enough (.exe files are inherently executable)
        path.exists()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn engine_path() -> PathBuf {
        // Goes up from crates/engine to the workspace root
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("../../engines/vendetta_chess_motor");
        p
    }

    #[test]
    fn test_new_defaults() {
        let cfg = EngineConfig::new("Vendetta", "/tmp/fake");
        assert_eq!(cfg.name, "Vendetta");
        assert!(cfg.options.is_empty());
        assert_eq!(cfg.default_limits.movetime, Some(1_000));
        assert_eq!(cfg.init_timeout, Duration::from_secs(5));
    }

    #[test]
    fn test_set_option() {
        let mut cfg = EngineConfig::new("Test", "/tmp/fake");
        cfg.set_option("Hash", "128");
        assert_eq!(cfg.options.get("Hash").map(String::as_str), Some("128"));
    }

    #[test]
    fn test_set_option_overwrite() {
        let mut cfg = EngineConfig::new("Test", "/tmp/fake");
        cfg.set_option("Hash", "64");
        cfg.set_option("Hash", "128");
        assert_eq!(cfg.options.get("Hash").map(String::as_str), Some("128"));
    }

    #[test]
    fn test_builder_options() {
        let cfg = EngineConfig::builder("Test", "/tmp/fake")
            .option("Hash", "256")
            .option("Threads", "4")
            .build();
        assert_eq!(cfg.options.get("Hash").map(String::as_str), Some("256"));
        assert_eq!(cfg.options.get("Threads").map(String::as_str), Some("4"));
    }

    #[test]
    fn test_builder_limits() {
        let limits = GoLimits {
            depth: Some(20),
            ..GoLimits::default()
        };
        let cfg = EngineConfig::builder("Test", "/tmp/fake")
            .default_limits(limits)
            .build();
        assert_eq!(cfg.default_limits.depth, Some(20));
    }

    #[test]
    fn test_builder_timeout() {
        let cfg = EngineConfig::builder("Test", "/tmp/fake")
            .init_timeout(Duration::from_secs(10))
            .build();
        assert_eq!(cfg.init_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_validate_path_not_found() {
        let cfg = EngineConfig::new("Test", "/nonexistent/path/engine");
        assert!(matches!(cfg.validate(), Err(ConfigError::PathNotFound(_))));
    }

    #[test]
    fn test_validate_vendetta_motor() {
        let path = engine_path();
        if !path.exists() {
            // Binary absent from CI — skip
            return;
        }
        let cfg = EngineConfig::new("Vendetta", path);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_not_executable() {
        // Creates a temporary file without the executable bit
        let tmp = env::temp_dir().join("vendetta_not_exec_test");
        std::fs::write(&tmp, b"not an engine").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
            let cfg = EngineConfig::new("Test", &tmp);
            let result = cfg.validate();
            std::fs::remove_file(&tmp).ok();
            assert!(matches!(result, Err(ConfigError::NotExecutable(_))));
        }
        #[cfg(not(unix))]
        {
            std::fs::remove_file(&tmp).ok();
            // On Windows, any existing file is considered executable
        }
    }

    #[test]
    fn test_config_clone() {
        let cfg = EngineConfig::builder("Test", "/tmp/fake")
            .option("Hash", "64")
            .build();
        let cloned = cfg.clone();
        assert_eq!(cloned.name, cfg.name);
        assert_eq!(cloned.options, cfg.options);
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::PathNotFound(PathBuf::from("/tmp/fake"));
        assert!(err.to_string().contains("Moteur introuvable"));
        let err2 = ConfigError::NotExecutable(PathBuf::from("/tmp/fake"));
        assert!(err2.to_string().contains("non exécutable"));
    }
}
