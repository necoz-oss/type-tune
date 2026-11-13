//! Declaration candidates and dependency eligibility analysis.

use crate::{
    destination::resolve_collocated_destination,
    hash::{candidate_id, sha256},
    model::{Candidate, ScanDiagnostic},
    parser::{ParsedDeclaration, ParsedSource},
    sort::sort_candidates,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(super) fn scan_parsed_declarations(files: &[(String, ParsedSource)]) -> Vec<Candidate> {
    scan_parsed_declarations_with_unsupported(files, false)
}

pub(super) fn scan_parsed_declarations_with_unsupported(
    files: &[(String, ParsedSource)],
    include_unsupported: bool,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (source_path, parsed_source) in files {
        let declarations = &parsed_source.declarations;
        let runtime_import_names = parsed_source
            .import_analysis
            .imports
            .iter()
            .filter(|import| !import.is_type_only)
            .map(|import| import.local_name.as_str())
            .collect::<HashSet<_>>();
        let declarations_by_name = index_declarations_by_name(declarations);
        let local_dependencies = collect_local_dependencies(source_path, declarations);
        for declaration in declarations.iter().filter(|declaration| {
            declaration.is_exported
                && !declaration.is_default_export
                && !has_unsupported_declaration_merge(declaration, declarations)
                && !shares_symbol_with_unsupported_declaration(
                    declaration,
                    &parsed_source.unsupported_type_names,
                )
                && (include_unsupported
                    || !has_unsupported_dependency(
                        declaration,
                        &declarations_by_name,
                        &runtime_import_names,
                        &parsed_source.unsupported_type_names,
                    ))
        }) {
            let stable_key = format!(
                "{source_path}#{}#{}",
                declaration.symbol_name, declaration.kind
            );
            let declaration_stable_key = declaration.stable_key(source_path);
            let dependencies = local_dependencies
                .get(&declaration_stable_key)
                .cloned()
                .unwrap_or_default();
            candidates.push(Candidate {
                id: candidate_id(&stable_key),
                declaration_key: sha256(&declaration_stable_key),
                stable_key,
                declaration_stable_key,
                symbol_name: declaration.symbol_name.clone(),
                kind: declaration.kind.clone(),
                source_path: source_path.clone(),
                destination_path: resolve_collocated_destination(source_path),
                referrer_paths: Vec::new(),
                referrer_count: 0,
                counted_referrer_count: None,
                local_dependency_names: dependencies
                    .iter()
                    .map(|dependency| dependency.symbol_name.clone())
                    .collect(),
                local_dependency_declaration_keys: dependencies
                    .iter()
                    .map(|dependency| dependency.declaration_key.clone())
                    .collect(),
                referrer_strategies: Vec::new(),
                readonly_referrers: Vec::new(),
                vue_script_strategies: Vec::new(),
                vue_rewrite_blockers: Vec::new(),
                analysis_limitations: Vec::new(),
                destination_index_path: None,
            });
        }
    }
    sort_candidates(&mut candidates);
    candidates
}

fn has_unsupported_declaration_merge(
    declaration: &ParsedDeclaration,
    declarations: &[ParsedDeclaration],
) -> bool {
    declarations.iter().any(|other| {
        other.symbol_name == declaration.symbol_name
            && other.start_position != declaration.start_position
            && is_unsupported_declaration_merge_pair(declaration, other)
    })
}

fn is_unsupported_declaration_merge_pair(
    left: &ParsedDeclaration,
    right: &ParsedDeclaration,
) -> bool {
    let left_kind = left.kind.as_str();
    let right_kind = right.kind.as_str();
    if (left_kind == "typeAlias" && right_kind == "interface")
        || (left_kind == "interface" && right_kind == "typeAlias")
    {
        return false;
    }
    true
}

fn is_allowed_ambiguous_same_file_export(matches: &[&ParsedDeclaration]) -> bool {
    matches.len() == 2
        && matches
            .iter()
            .any(|declaration| declaration.kind == "typeAlias")
        && matches
            .iter()
            .any(|declaration| declaration.kind == "interface")
}

fn shares_symbol_with_unsupported_declaration(
    declaration: &ParsedDeclaration,
    unsupported_type_names: &BTreeSet<String>,
) -> bool {
    matches!(declaration.kind.as_str(), "interface" | "typeAlias")
        && unsupported_type_names.contains(&declaration.symbol_name)
}

pub(super) fn unsupported_dependency_diagnostics(
    files: &[(String, ParsedSource)],
) -> Vec<ScanDiagnostic> {
    let mut diagnostics = Vec::new();
    for (source_path, parsed_source) in files {
        let declarations = &parsed_source.declarations;
        let declarations_by_name = index_declarations_by_name(declarations);
        let runtime_import_names = parsed_source
            .import_analysis
            .imports
            .iter()
            .filter(|import| !import.is_type_only)
            .map(|import| import.local_name.as_str())
            .collect::<HashSet<_>>();
        for declaration in declarations.iter().filter(|declaration| {
            declaration.is_exported
                && !declaration.is_default_export
                && declarations_by_name
                    .get(declaration.symbol_name.as_str())
                    .is_some_and(|matches| matches.len() == 1)
                && !shares_symbol_with_unsupported_declaration(
                    declaration,
                    &parsed_source.unsupported_type_names,
                )
                && has_unsupported_dependency(
                    declaration,
                    &declarations_by_name,
                    &runtime_import_names,
                    &parsed_source.unsupported_type_names,
                )
        }) {
            let stable_key = format!(
                "{source_path}#{}#{}",
                declaration.symbol_name, declaration.kind
            );
            diagnostics.push(ScanDiagnostic {
                severity: "info".into(),
                reason: "unsupportedDependency".into(),
                message: "Declaration depends on a runtime value, type query, or unsupported same-file declaration.".into(),
                candidate_id: candidate_id(&stable_key),
                symbol_name: declaration.symbol_name.clone(),
                kind: declaration.kind.clone(),
                source_path: source_path.clone(),
                referrer_count: None,
                min_referrers: None,
            });
        }
    }
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
    diagnostics
}

fn has_unsupported_dependency(
    declaration: &ParsedDeclaration,
    declarations_by_name: &BTreeMap<&str, Vec<&ParsedDeclaration>>,
    runtime_import_names: &HashSet<&str>,
    unsupported_type_names: &BTreeSet<String>,
) -> bool {
    let mut pending = vec![declaration];
    let mut visited = HashSet::new();
    while let Some(current) = pending.pop() {
        if current.has_type_query {
            return true;
        }
        for reference in &current.referenced_type_names {
            if runtime_import_names.contains(reference.as_str())
                || unsupported_type_names.contains(reference)
            {
                return true;
            }
            let Some(matches) = declarations_by_name.get(reference.as_str()) else {
                continue;
            };
            if matches.len() != 1 {
                if is_allowed_ambiguous_same_file_export(matches) {
                    continue;
                }
                return true;
            }
            let dependency = matches[0];
            if visited.insert(dependency.start_position) {
                pending.push(dependency);
            }
        }
    }
    false
}

#[derive(Debug, Clone)]
struct LocalDependency {
    symbol_name: String,
    declaration_key: String,
}

fn index_declarations_by_name(
    declarations: &[ParsedDeclaration],
) -> BTreeMap<&str, Vec<&ParsedDeclaration>> {
    let mut by_name = BTreeMap::<&str, Vec<&ParsedDeclaration>>::new();
    for declaration in declarations {
        by_name
            .entry(&declaration.symbol_name)
            .or_default()
            .push(declaration);
    }
    by_name
}

fn collect_local_dependencies(
    source_path: &str,
    declarations: &[ParsedDeclaration],
) -> BTreeMap<String, Vec<LocalDependency>> {
    let declarations_by_name = index_declarations_by_name(declarations);
    declarations
        .iter()
        .filter(|declaration| declaration.is_exported && !declaration.is_default_export)
        .map(|root| {
            let root_key = root.stable_key(source_path);
            let mut seen = HashSet::new();
            let mut pending = vec![root];
            let mut dependencies = Vec::new();
            while let Some(current) = pending.pop() {
                for reference in &current.referenced_type_names {
                    let Some(matches) = declarations_by_name.get(reference.as_str()) else {
                        continue;
                    };
                    if matches.len() != 1 {
                        continue;
                    }
                    let dependency = matches[0];
                    let dependency_stable_key = dependency.stable_key(source_path);
                    if dependency_stable_key == root_key
                        || !seen.insert(dependency_stable_key.clone())
                    {
                        continue;
                    }
                    dependencies.push(LocalDependency {
                        symbol_name: dependency.symbol_name.clone(),
                        declaration_key: sha256(&dependency_stable_key),
                    });
                    pending.push(dependency);
                }
            }
            dependencies.sort_by(|left, right| left.declaration_key.cmp(&right.declaration_key));
            (root_key, dependencies)
        })
        .collect()
}
