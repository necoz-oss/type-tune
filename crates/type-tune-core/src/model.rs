//! Serializable native scan and plan data structures.

use serde::{Deserialize, Serialize};

/// One type declaration selected by native scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Stable identifier derived from source path, name, and kind.
    pub id: String,
    /// Full SHA-256 key for this concrete declaration.
    pub declaration_key: String,
    /// Candidate identity before hashing.
    pub stable_key: String,
    /// Concrete declaration identity before hashing.
    pub declaration_stable_key: String,
    /// Exported declaration name.
    pub symbol_name: String,
    /// `typeAlias` or `interface`.
    pub kind: String,
    /// Project-relative POSIX source path.
    pub source_path: String,
    /// Project-relative destination path for the extracted declaration.
    pub destination_path: String,
    /// Project-relative referrer paths in deterministic order.
    pub referrer_paths: Vec<String>,
    /// Total number of unique referrer files.
    pub referrer_count: usize,
    /// Number of referrers included in eligibility when exclusions were applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counted_referrer_count: Option<usize>,
    /// Same-file type declarations that must move with this candidate.
    pub local_dependency_names: Vec<String>,
    /// Declaration keys for same-file type declarations that must move.
    pub local_dependency_declaration_keys: Vec<String>,
    /// Import update strategy for every writable referrer path.
    pub referrer_strategies: Vec<ReferrerStrategy>,
    /// Referrers discovered by read-only analysis. These files are counted and never rewritten.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readonly_referrers: Vec<ReadOnlyReferrer>,
    /// Project-wide analysis gaps that make rewriting any of these candidates unsafe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis_limitations: Vec<AnalysisLimitation>,
    /// Barrel index path to update when a barrel strategy is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_index_path: Option<String>,
}

impl Candidate {
    pub(crate) fn is_ignored(&self, ignore_entries: &[String]) -> bool {
        if ignore_entries.is_empty() {
            return false;
        }
        let source_symbol = format!("{}#{}", self.source_path, self.symbol_name);
        let source_kind_symbol = format!("{}#{}#{}", self.source_path, self.kind, self.symbol_name);
        ignore_entries.iter().any(|entry| {
            entry == &self.id
                || entry == &self.stable_key
                || entry == &source_symbol
                || entry == &source_kind_symbol
        })
    }
}

/// Explanation for one exported declaration omitted from a scan result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDiagnostic {
    /// Informational severity used for omitted declarations.
    pub severity: String,
    /// Machine-readable omission reason.
    pub reason: String,
    /// Human-readable explanation.
    pub message: String,
    /// Stable candidate id for the omitted declaration.
    pub candidate_id: String,
    /// Exported symbol name.
    pub symbol_name: String,
    /// Declaration kind.
    pub kind: String,
    /// Project-relative declaring source path.
    pub source_path: String,
    /// Unique referrer count when the reason depends on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer_count: Option<usize>,
    /// Required referrer count when the reason depends on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_referrers: Option<usize>,
}

/// Machine-readable reason for a Vue SFC counted during read-only referrer analysis.
pub const READONLY_REFERRER_VUE_SFC: &str = "vueSfc";

/// Machine-readable reason for a Vue SFC that could not be analyzed safely.
pub const ANALYSIS_LIMITATION_UNRESOLVED_VUE: &str = "unresolvedVueSfc";

/// A referrer that contributes to scan counts and cannot be edited by the current rewrite pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadOnlyReferrer {
    /// Project-relative referrer path.
    pub path: String,
    /// Stable reason code. Vue script usage uses [`READONLY_REFERRER_VUE_SFC`].
    pub reason: String,
}

/// A source that could not be analyzed reliably enough to treat as a non-referrer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisLimitation {
    /// Project-relative source path.
    pub path: String,
    /// Stable reason code.
    pub reason: String,
    /// Actionable explanation.
    pub message: String,
}

/// Native import update strategy for one referrer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferrerStrategy {
    /// Project-relative referrer path.
    pub referrer_path: String,
    /// Import strategy selected for this referrer.
    pub strategy: ImportStrategy,
}

/// Native import strategy shape compatible with scan JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ImportStrategy {
    /// Path alias module specifier to the destination type file.
    Alias {
        /// Alias module specifier.
        module_specifier: String,
    },
    /// Barrel file that re-exports the moved declaration.
    Barrel {
        /// Project-relative barrel file path.
        barrel_file_path: String,
        /// Module specifier from the barrel to the destination type file.
        export_module_specifier: String,
    },
    /// Relative module specifier to the destination type file.
    Relative {
        /// Relative module specifier.
        module_specifier: String,
    },
}

/// Native scan response compatible with the JSON shape used by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOutput {
    /// Candidates in deterministic order.
    pub candidates: Vec<Candidate>,
    /// Omitted declaration diagnostics requested with `--explain`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<ScanDiagnostic>>,
    /// Vue files in the selected project that could not be analyzed reliably.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis_limitations: Vec<AnalysisLimitation>,
}

#[cfg(test)]
mod tests {
    use super::{Candidate, ImportStrategy, ScanOutput};

    #[test]
    fn omits_unchanged_count_metadata_from_json() {
        let output = ScanOutput {
            candidates: vec![Candidate {
                id: "candidate".into(),
                declaration_key: "declaration".into(),
                stable_key: "src/shared.ts#Shared#typeAlias".into(),
                declaration_stable_key: "src/shared.ts#typeAlias#Shared#0".into(),
                symbol_name: "Shared".into(),
                kind: "typeAlias".into(),
                source_path: "src/shared.ts".into(),
                destination_path: "src/types/shared.ts".into(),
                referrer_paths: vec!["src/app.ts".into()],
                referrer_count: 1,
                counted_referrer_count: None,
                local_dependency_names: vec![],
                local_dependency_declaration_keys: vec![],
                referrer_strategies: vec![],
                readonly_referrers: vec![],
                analysis_limitations: vec![],
                destination_index_path: None,
            }],
            diagnostics: None,
            analysis_limitations: vec![],
        };
        let json = serde_json::to_value(&output).expect("serializes");
        assert!(json["candidates"][0].get("countedReferrerCount").is_none());

        let mut output = output;
        output.candidates[0].counted_referrer_count = Some(0);
        let json = serde_json::to_value(&output).expect("serializes");
        assert_eq!(json["candidates"][0]["countedReferrerCount"], 0);
    }

    #[test]
    fn serializes_import_strategy_fields_in_camel_case() {
        let strategy = ImportStrategy::Barrel {
            barrel_file_path: "src/index.ts".into(),
            export_module_specifier: "./types/src".into(),
        };
        assert_eq!(
            serde_json::to_value(strategy).expect("serializes"),
            serde_json::json!({
                "kind": "barrel",
                "barrelFilePath": "src/index.ts",
                "exportModuleSpecifier": "./types/src",
            })
        );
    }
}
