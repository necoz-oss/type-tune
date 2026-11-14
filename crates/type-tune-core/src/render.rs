//! Human-readable native scan rendering.

use crate::model::{Candidate, ImportStrategy, ScanDiagnostic};

/// Renders deterministic scan candidates in the CLI text format.
#[must_use]
pub fn render_scan_text(candidates: &[Candidate], verbose: bool) -> String {
    render_scan_text_with_diagnostics(candidates, verbose, None)
}

/// Renders scan candidates and optional explain diagnostics in TypeScript-compatible text order.
#[must_use]
pub fn render_scan_text_with_diagnostics(
    candidates: &[Candidate],
    verbose: bool,
    diagnostics: Option<&[ScanDiagnostic]>,
) -> String {
    let mut candidates = candidates.to_vec();
    candidates.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
    if candidates.is_empty() {
        return match diagnostics {
            Some(diagnostics) => {
                format!("No candidates found.{}\n", render_diagnostics(diagnostics))
            }
            None => "No candidates found.\n".into(),
        };
    }
    let headers = if verbose {
        vec![
            "Symbol",
            "Kind",
            "Source",
            "Destination",
            "Local deps",
            "Refs",
            "Imports",
        ]
    } else {
        vec!["Symbol", "Kind", "Source", "Destination", "Local deps"]
    };
    let rows = candidates
        .iter()
        .map(|candidate| render_candidate_row(candidate, verbose))
        .collect::<Vec<_>>();
    format!(
        "Found {} candidate{}.\n\n{}{}\n\n{}\n",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "s" },
        render_table(&headers, &rows),
        diagnostics.map_or_else(String::new, render_diagnostics),
        render_next_steps().trim_end(),
    )
}

fn render_diagnostics(diagnostics: &[ScanDiagnostic]) -> String {
    let mut diagnostics = diagnostics.to_vec();
    diagnostics.sort_by(|left, right| {
        [
            left.source_path.as_str(),
            left.symbol_name.as_str(),
            left.reason.as_str(),
        ]
        .cmp(&[
            right.source_path.as_str(),
            right.symbol_name.as_str(),
            right.reason.as_str(),
        ])
    });
    if diagnostics.is_empty() {
        return "\n\nExcluded declarations: none".into();
    }
    let mut lines = vec![
        String::new(),
        String::new(),
        "Excluded declarations:".into(),
    ];
    lines.extend(diagnostics.iter().map(|diagnostic| {
        format!(
            "  - {}#{}: {} - {}",
            diagnostic.source_path, diagnostic.symbol_name, diagnostic.reason, diagnostic.message
        )
    }));
    lines.join("\n")
}

fn render_candidate_row(candidate: &Candidate, verbose: bool) -> Vec<String> {
    let mut row = vec![
        candidate.symbol_name.clone(),
        render_kind(&candidate.kind).into(),
        candidate.source_path.clone(),
        candidate.destination_path.clone(),
        render_local_dependencies(&candidate.local_dependency_names),
    ];
    if verbose {
        row.push(render_referrers(candidate));
        row.push(render_import_strategies(candidate));
    }
    row
}

fn render_kind(kind: &str) -> &str {
    if kind == "typeAlias" {
        "type"
    } else {
        "interface"
    }
}

fn render_local_dependencies(names: &[String]) -> String {
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

fn render_referrers(candidate: &Candidate) -> String {
    let count = candidate.counted_referrer_count.map_or_else(
        || candidate.referrer_count.to_string(),
        |counted| format!("{counted} counted/{} total", candidate.referrer_count),
    );
    let mut paths = candidate
        .referrer_paths
        .iter()
        .map(String::as_str)
        .chain(
            candidate
                .readonly_referrers
                .iter()
                .map(|referrer| referrer.path.as_str()),
        )
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths.dedup();
    format!("{count}:{}", paths.join(","))
}

fn render_import_strategies(candidate: &Candidate) -> String {
    let mut rendered = Vec::new();
    for referrer in &candidate.referrer_strategies {
        let strategy = match &referrer.strategy {
            ImportStrategy::Alias { module_specifier } => format!("alias:{module_specifier}"),
            ImportStrategy::Barrel {
                export_module_specifier,
                ..
            } => format!("barrel:{export_module_specifier}"),
            ImportStrategy::Relative { module_specifier } => {
                format!("relative:{module_specifier}")
            }
        };
        if !rendered.contains(&strategy) {
            rendered.push(strategy);
        }
    }
    rendered.join(", ")
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let widths = headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|cell| cell.len())
                .fold(header.len(), usize::max)
        })
        .collect::<Vec<_>>();
    let render_row = |row: Vec<&str>| {
        format!(
            "| {} |",
            row.iter()
                .enumerate()
                .map(|(index, cell)| format!("{cell:width$}", width = widths[index]))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    };
    let header = render_row(headers.to_vec());
    let separator = format!(
        "| {} |",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let rendered_rows = rows
        .iter()
        .map(|row| render_row(row.iter().map(String::as_str).collect()))
        .collect::<Vec<_>>();
    std::iter::once(header)
        .chain(std::iter::once(separator))
        .chain(rendered_rows)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Renders apply dry-run text.
#[must_use]
pub fn render_apply_dry_run(
    apply_count: usize,
    ignored_count: usize,
    candidates: &[Candidate],
    changed_paths: &[String],
    created_paths: &[String],
    deleted_paths: &[String],
) -> String {
    let mut lines = vec![format!(
        "Dry run: {apply_count} candidate{} would be applied; {ignored_count} ignored.",
        if apply_count == 1 { "" } else { "s" }
    )];
    append_candidate_section(&mut lines, candidates);
    append_import_target_section(&mut lines, candidates);
    append_file_sections(&mut lines, changed_paths, created_paths, deleted_paths);
    if lines.len() == 1 {
        lines.push("No file changes.".into());
    }
    format!("{}\n", lines.join("\n"))
}

/// Renders extract dry-run text.
#[must_use]
pub fn render_extract_dry_run(
    symbol_name: &str,
    source_path: &str,
    destination_path: &str,
    changed_paths: &[String],
    created_paths: &[String],
    deleted_paths: &[String],
) -> String {
    let mut lines = vec![format!(
        "Dry run: {symbol_name} would be extracted from {source_path} to {destination_path}."
    )];
    append_file_sections(&mut lines, changed_paths, created_paths, deleted_paths);
    if lines.len() == 1 {
        lines.push("No file changes.".into());
    }
    format!("{}\n", lines.join("\n"))
}

fn append_candidate_section(lines: &mut Vec<String>, candidates: &[Candidate]) {
    if candidates.is_empty() {
        return;
    }
    lines.push("Candidates:".into());
    for candidate in candidates {
        lines.push(format!(
            "- {} {} -> {}",
            candidate.symbol_name, candidate.source_path, candidate.destination_path
        ));
    }
}

fn append_import_target_section(lines: &mut Vec<String>, candidates: &[Candidate]) {
    let entries = candidates
        .iter()
        .flat_map(|candidate| {
            candidate.referrer_strategies.iter().map(move |referrer| {
                (
                    candidate.symbol_name.as_str(),
                    referrer.referrer_path.as_str(),
                    &referrer.strategy,
                )
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return;
    }
    lines.push("Import/export targets:".into());
    for (symbol_name, referrer_path, strategy) in entries {
        lines.push(format!(
            "- {symbol_name} {referrer_path}: {}",
            render_import_target(strategy)
        ));
    }
}

fn render_import_target(strategy: &ImportStrategy) -> String {
    match strategy {
        ImportStrategy::Barrel {
            barrel_file_path,
            export_module_specifier,
        } => format!(
            "keep barrel import; update {barrel_file_path} export {export_module_specifier}"
        ),
        ImportStrategy::Alias { module_specifier } => format!("alias import {module_specifier}"),
        ImportStrategy::Relative { module_specifier } => {
            format!("relative import {module_specifier}")
        }
    }
}

fn append_file_sections(
    lines: &mut Vec<String>,
    changed_paths: &[String],
    created_paths: &[String],
    deleted_paths: &[String],
) {
    append_file_section(lines, "Changed files", changed_paths);
    append_file_section(lines, "Created files", created_paths);
    append_file_section(lines, "Deleted files", deleted_paths);
}

fn append_file_section(lines: &mut Vec<String>, title: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    lines.push(format!("{title}:"));
    for path in paths {
        lines.push(format!("- {path}"));
    }
}

fn render_next_steps() -> &'static str {
    "Next:\n  Review the candidates above.\n  Run `type-tune plan --project <tsconfig> --out .type-tune/plan.json --markdown .type-tune/plan.md` to create a reviewable plan."
}

#[cfg(test)]
mod tests {
    use crate::model::{Candidate, ImportStrategy, ReferrerStrategy};

    use super::{
        render_apply_dry_run, render_extract_dry_run, render_scan_text,
        render_scan_text_with_diagnostics,
    };

    #[test]
    fn reports_empty_scans() {
        assert_eq!(render_scan_text(&[], false), "No candidates found.\n");
    }

    #[test]
    fn renders_a_text_table_in_stable_key_order() {
        let candidate = |symbol_name: &str, stable_key: &str| Candidate {
            id: symbol_name.into(),
            declaration_key: "key".into(),
            stable_key: stable_key.into(),
            declaration_stable_key: "stable".into(),
            symbol_name: symbol_name.into(),
            kind: "typeAlias".into(),
            source_path: "src/shared.ts".into(),
            destination_path: "src/types/src.ts".into(),
            referrer_paths: vec![],
            referrer_count: 0,
            counted_referrer_count: None,
            local_dependency_names: vec![],
            local_dependency_declaration_keys: vec![],
            referrer_strategies: vec![],
            readonly_referrers: vec![],
            vue_script_strategies: vec![],
            vue_rewrite_blockers: vec![],
            analysis_limitations: vec![],
            destination_index_path: None,
        };
        let output = render_scan_text(&[candidate("Later", "z"), candidate("First", "a")], false);
        assert!(output.contains("Found 2 candidates."));
        assert!(output.find("First").expect("first") < output.find("Later").expect("later"));
        assert!(
            output.contains("| Symbol | Kind | Source        | Destination      | Local deps |")
        );
    }

    #[test]
    fn places_explain_diagnostics_before_next_steps() {
        let candidate = Candidate {
            id: "shared".into(),
            declaration_key: "key".into(),
            stable_key: "src/shared.ts#Shared#typeAlias".into(),
            declaration_stable_key: "stable".into(),
            symbol_name: "Shared".into(),
            kind: "typeAlias".into(),
            source_path: "src/shared.ts".into(),
            destination_path: "src/types/src.ts".into(),
            referrer_paths: vec!["src/app.ts".into()],
            referrer_count: 1,
            counted_referrer_count: None,
            local_dependency_names: vec![],
            local_dependency_declaration_keys: vec![],
            referrer_strategies: vec![],
            readonly_referrers: vec![],
            vue_script_strategies: vec![],
            vue_rewrite_blockers: vec![],
            analysis_limitations: vec![],
            destination_index_path: None,
        };
        let diagnostics = [crate::model::ScanDiagnostic {
            severity: "info".into(),
            reason: "insufficientReferrers".into(),
            message: "below threshold".into(),
            candidate_id: "app".into(),
            symbol_name: "App".into(),
            kind: "typeAlias".into(),
            source_path: "src/app.ts".into(),
            referrer_count: Some(0),
            min_referrers: Some(1),
        }];
        let output = render_scan_text_with_diagnostics(&[candidate], false, Some(&diagnostics));
        let excluded = output.find("Excluded declarations:").expect("diagnostics");
        let next = output.find("Next:").expect("next steps");
        assert!(excluded < next);
    }

    #[test]
    fn renders_apply_dry_run_with_import_targets_and_file_sections() {
        let candidate = Candidate {
            id: "shared".into(),
            declaration_key: "key".into(),
            stable_key: "src/shared.ts#Shared#typeAlias".into(),
            declaration_stable_key: "stable".into(),
            symbol_name: "Shared".into(),
            kind: "typeAlias".into(),
            source_path: "src/shared.ts".into(),
            destination_path: "src/types/src.ts".into(),
            referrer_paths: vec!["src/app.ts".into()],
            referrer_count: 1,
            counted_referrer_count: None,
            local_dependency_names: vec![],
            local_dependency_declaration_keys: vec![],
            referrer_strategies: vec![ReferrerStrategy {
                referrer_path: "src/app.ts".into(),
                strategy: ImportStrategy::Relative {
                    module_specifier: "./types/src".into(),
                },
            }],
            readonly_referrers: vec![],
            vue_script_strategies: vec![],
            vue_rewrite_blockers: vec![],
            analysis_limitations: vec![],
            destination_index_path: None,
        };
        let output = render_apply_dry_run(
            1,
            0,
            &[candidate],
            &["src/app.ts".into(), "src/shared.ts".into()],
            &["src/types/src.ts".into()],
            &[],
        );
        assert_eq!(
            output,
            "\
Dry run: 1 candidate would be applied; 0 ignored.
Candidates:
- Shared src/shared.ts -> src/types/src.ts
Import/export targets:
- Shared src/app.ts: relative import ./types/src
Changed files:
- src/app.ts
- src/shared.ts
Created files:
- src/types/src.ts
"
        );
    }

    #[test]
    fn renders_empty_apply_and_extract_dry_runs() {
        assert_eq!(
            render_apply_dry_run(0, 1, &[], &[], &[], &[]),
            "Dry run: 0 candidates would be applied; 1 ignored.\nNo file changes.\n"
        );
        assert_eq!(
            render_extract_dry_run("Shared", "src/shared.ts", "src/types/src.ts", &[], &[], &[]),
            "Dry run: Shared would be extracted from src/shared.ts to src/types/src.ts.\nNo file changes.\n"
        );
    }
}
