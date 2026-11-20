//! Command module registry and shared tests.

mod apply;
mod extract;
mod plan;
mod scan;

pub(crate) use apply::run_apply;
pub(crate) use extract::run_extract;
pub(crate) use plan::run_plan;
pub(crate) use scan::run_scan;

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{project::read_rewrite_sources, scan::remove_unsupported_candidates};

    #[test]
    fn scan_and_plan_agree_on_types_file_eligibility() {
        use super::{
            plan::{PlanCandidateOptions, scan_plan_candidates},
            scan::{ScanCandidateOptions, render_scan_result, scan_candidates},
        };
        use type_tune_core::destination::PlacementStyle;

        let root = std::env::temp_dir().join(format!(
            "type-tune-types-file-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/feature")).unwrap();
        fs::write(root.join("tsconfig.json"), r#"{"include":["src/**/*.ts"]}"#).unwrap();
        fs::write(
            root.join("src/feature/types.ts"),
            "export type Shared = { id: string };\n",
        )
        .unwrap();
        fs::write(
            root.join("src/feature/model.ts"),
            "export type WidgetProps = { title: string };\n",
        )
        .unwrap();
        for name in ["one", "two"] {
            fs::write(root.join(format!("src/{name}.ts")), "import type { Shared } from './feature/types';\nimport type { WidgetProps } from './feature/model';\nexport type View = [Shared, WidgetProps];\n").unwrap();
        }
        let project = root.join("tsconfig.json");
        let project = project.to_str().unwrap();
        let kinds = vec!["typeAlias".into(), "interface".into()];
        let mut shared_id = None;
        for include_existing_types in [false, true] {
            let scan = scan_candidates(
                project,
                ScanCandidateOptions {
                    minimum_referrers: 2,
                    exclusions: &[],
                    include_kinds: &kinds,
                    ignore_entries: &[],
                    include_existing_types,
                    include_generated: false,
                    placement_style: PlacementStyle::Collocation,
                    explain: true,
                    compiler_resolution_bridge: false,
                },
            )
            .expect("scan");
            let scan_ids = scan
                .candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect::<Vec<_>>();
            let shared = scan
                .candidates
                .iter()
                .find(|candidate| candidate.symbol_name == "Shared")
                .map(|candidate| candidate.id.clone());
            assert_eq!(shared.is_some(), include_existing_types);
            assert!(
                scan.candidates
                    .iter()
                    .any(|candidate| candidate.symbol_name == "WidgetProps")
            );
            if let Some(shared) = &shared {
                assert_eq!(Some(shared), shared_id.as_ref());
            } else {
                let json =
                    render_scan_result(scan, "json", false, true).expect("renders explain JSON");
                let json: serde_json::Value = serde_json::from_str(&json).unwrap();
                let diagnostic = json["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|diagnostic| diagnostic["symbolName"] == "Shared")
                    .unwrap();
                assert_eq!(diagnostic["reason"], "existingTypesFile");
                assert!(
                    diagnostic["message"]
                        .as_str()
                        .unwrap()
                        .contains("--include-existing-types")
                );
                shared_id = Some(diagnostic["candidateId"].as_str().unwrap().to_owned());
            }
            let planned = scan_plan_candidates(
                project,
                PlanCandidateOptions {
                    include_generated: false,
                    placement_style: PlacementStyle::Collocation,
                    include_kinds: &kinds,
                    include_existing_types,
                    exclusions: &[],
                    minimum_referrers: 2,
                    ignore_entries: &[],
                    compiler_resolution_bridge: false,
                },
            )
            .expect("plan candidates");
            assert_eq!(
                scan_ids,
                planned
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect::<Vec<_>>()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_filter_diagnostic_precedence_over_unsupported_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-explain-precedence-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).expect("creates fixture");
        fs::write(root.join("tsconfig.json"), r#"{"include":["src/**/*.ts"]}"#)
            .expect("writes config");
        fs::write(
            root.join("src/shared.ts"),
            "const runtime = 'value'; export type RuntimeDependent = typeof runtime;",
        )
        .expect("writes declaration");
        fs::write(
            root.join("src/app.ts"),
            "import type { RuntimeDependent } from './shared'; export type App = RuntimeDependent;",
        )
        .expect("writes referrer");
        let (candidates, diagnostics) =
            type_tune_core::scan::scan_tsconfig_with_generated_and_diagnostics(
                &root.join("tsconfig.json"),
                false,
            )
            .expect("scans config");
        fs::remove_dir_all(root).expect("removes fixture");

        let (eligible, _) = type_tune_core::scan::filter_candidates_with_diagnostics(
            candidates.clone(),
            type_tune_core::scan::ScanFilterOptions {
                minimum_referrers: 1,
                referrer_count_exclusions: &[],
                include_kinds: &["typeAlias".into(), "interface".into()],
                ignore_entries: &[],
                include_existing_types: false,
                placement_style: type_tune_core::destination::PlacementStyle::Collocation,
                base_url: None,
                path_aliases: &BTreeMap::new(),
            },
        );
        let (eligible, unsupported) = remove_unsupported_candidates(eligible, diagnostics.clone());
        assert!(eligible.is_empty());
        assert_eq!(unsupported.len(), 1);
        assert_eq!(unsupported[0].reason, "unsupportedDependency");

        let (eligible, filter_diagnostics) =
            type_tune_core::scan::filter_candidates_with_diagnostics(
                candidates,
                type_tune_core::scan::ScanFilterOptions {
                    minimum_referrers: 2,
                    referrer_count_exclusions: &[],
                    include_kinds: &["typeAlias".into(), "interface".into()],
                    ignore_entries: &[],
                    include_existing_types: false,
                    placement_style: type_tune_core::destination::PlacementStyle::Collocation,
                    base_url: None,
                    path_aliases: &BTreeMap::new(),
                },
            );
        let (_, unsupported) = remove_unsupported_candidates(eligible, diagnostics);
        assert!(unsupported.is_empty());
        assert!(
            filter_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.symbol_name == "RuntimeDependent"
                    && diagnostic.reason == "insufficientReferrers")
        );
    }

    #[test]
    fn reads_type_dependencies_needed_for_a_native_rewrite() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-rewrite-sources-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).expect("creates fixture");
        fs::write(root.join("tsconfig.json"), "{}").expect("writes config");
        fs::write(root.join("src/external.d.ts"), "export type External = {};")
            .expect("writes external");
        fs::write(
            root.join("src/shared.ts"),
            "import type { External } from './external'; export type Shared = External;",
        )
        .expect("writes shared");
        fs::write(
            root.join("src/app.ts"),
            "import type { Shared } from './shared'; type App = Shared;",
        )
        .expect("writes app");
        let candidate = type_tune_core::scan::scan_project_files(vec![
            (
                "src/external.d.ts".into(),
                "export type External = {};".into(),
            ),
            (
                "src/shared.ts".into(),
                "import type { External } from './external'; export type Shared = External;".into(),
            ),
            (
                "src/app.ts".into(),
                "import type { Shared } from './shared'; type App = Shared;".into(),
            ),
        ])
        .expect("scans")
        .into_iter()
        .find(|candidate| candidate.symbol_name == "Shared")
        .expect("finds shared");
        let sources = read_rewrite_sources(
            &root.join("tsconfig.json"),
            std::slice::from_ref(&candidate),
        )
        .expect("reads rewrite sources");
        let changes = type_tune_core::rewrite::plan_extraction_rewrite(&[candidate], &sources)
            .expect("plans rewrite");
        assert!(changes["src/types/src.ts"].starts_with("import type { External }"));
        fs::remove_dir_all(root).expect("removes fixture");
    }
}
