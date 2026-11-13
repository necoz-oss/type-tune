//! Fast, deterministic analysis primitives for type-tune.

pub mod ai;
pub mod compiler;
pub mod config;
pub mod destination;
pub mod edit;
pub mod error;
pub mod glob;
pub mod hash;
pub mod model;
pub mod module_path;
pub mod module_specifier;
pub mod options;
pub mod parser;
pub mod path;
pub mod plan;
pub mod render;
pub mod review;
pub mod rewrite;
pub mod scan;
pub mod sort;
pub mod source;
mod source_role;
pub mod transaction;
mod vue;

pub use error::{Result, TypeTuneError};

/// Returns the crate's semantic-version string.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn matches_the_root_package_version() {
        let manifest = include_str!("../../../package.json");
        let parsed: serde_json::Value =
            serde_json::from_str(manifest).expect("root package.json must be valid JSON");
        let package_version = parsed
            .get("version")
            .and_then(serde_json::Value::as_str)
            .expect("root package.json must contain a string version");
        assert_eq!(version(), package_version);
    }
}
