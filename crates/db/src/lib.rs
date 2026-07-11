pub mod eco_names;
pub mod import_export;
pub mod reference_import;
pub mod reference_schema;
pub mod repository;
pub mod schema;
pub mod scid_import;

/// Re-export of `rusqlite::Connection` for dependent crates (e.g. `gui`)
/// that need to name the type without adding `rusqlite` as a direct
/// dependency (guarantees that only one version of `rusqlite` is used).
pub use rusqlite::Connection;
