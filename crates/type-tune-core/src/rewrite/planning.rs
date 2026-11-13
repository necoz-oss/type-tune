//! Ordered phases for one extraction rewrite.
//!
//! Phases stay in this order: prepare parsed inputs, record merged referrer imports, plan each
//! source move, render destination files, plan the remaining referrer edits, then apply edits.
//! Edit ranges and destination text are unchanged by splitting the phases.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::{
    Result, TypeTuneError,
    edit::{TextEdit, apply_text_edits},
    model::Candidate,
    parser::{
        ParsedDeclaration, ParsedSource, parse_import_usage_for_cleanup, parse_source,
        source_has_type_usage,
    },
};

use super::{
    RewriteOptions,
    compatibility::{SourceCompatibility, plan_source_compatibility},
    destination::DestinationPlan,
    format::{match_line_endings, module_specifier_extension_for_source, statement_semicolon},
    imports::collect_imported_type_dependencies,
    referrer_edits::{ReferrerEditContext, plan_referrer_edits},
    referrers::{MergedReferrerImports, plan_merged_referrer_imports},
    resolution::has_inline_type_referrers,
    source::{declaration_removal_edit, plan_source_cleanup},
    validation::{CandidateGroups, declaration_key, group_candidates, moved_declarations},
};

struct RewriteInputs<'a> {
    candidates: &'a [Candidate],
    sources: &'a BTreeMap<String, String>,
    options: RewriteOptions<'a>,
    known_paths: HashSet<String>,
    groups: CandidateGroups<'a>,
    parsed_sources: BTreeMap<String, ParsedSource>,
    module_specifier_extension: String,
}

pub(super) fn plan_rewrite(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    options: RewriteOptions<'_>,
) -> Result<BTreeMap<String, String>> {
    crate::source_role::reject_unsupported_rewrite(candidates)?;
    if let Some(path) = sources.keys().find(|path| {
        crate::source_role::source_role(path) != crate::source_role::SourceRole::WritableTypeScript
    }) {
        return Err(TypeTuneError::new(format!(
            "Vue rewriting is not supported yet. Refusing to treat {path} as a writable TypeScript source. Apply was rejected before any files were changed."
        )));
    }
    let inputs = prepare_rewrite_inputs(candidates, sources, options)?;
    let mut edits_by_path = BTreeMap::<String, Vec<TextEdit>>::new();
    let merged_referrer_import_ranges =
        record_merged_referrer_imports(&inputs, &mut edits_by_path)?;
    let mut destinations = DestinationPlan::default();
    plan_source_moves(&inputs, &mut destinations, &mut edits_by_path)?;
    let mut output = BTreeMap::new();
    destinations.render(inputs.sources, &mut output);
    plan_remaining_referrer_edits(&inputs, &merged_referrer_import_ranges, &mut edits_by_path)?;
    apply_planned_edits(&inputs, edits_by_path, &mut output)?;
    Ok(output)
}

fn prepare_rewrite_inputs<'a>(
    candidates: &'a [Candidate],
    sources: &'a BTreeMap<String, String>,
    options: RewriteOptions<'a>,
) -> Result<RewriteInputs<'a>> {
    let known_paths = sources.keys().cloned().collect::<HashSet<_>>();
    let groups = group_candidates(candidates)?;
    let parsed_sources = sources
        .keys()
        .cloned()
        .map(|path| {
            let source = sources
                .get(&path)
                .ok_or_else(|| TypeTuneError::new(format!("Could not find source for {path}.")))?;
            Ok((path, parse_source(source)?))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let module_specifier_extension = options.default_module_specifier_extension.to_owned();
    Ok(RewriteInputs {
        candidates,
        sources,
        options,
        known_paths,
        groups,
        parsed_sources,
        module_specifier_extension,
    })
}

fn record_merged_referrer_imports(
    inputs: &RewriteInputs<'_>,
    edits_by_path: &mut BTreeMap<String, Vec<TextEdit>>,
) -> Result<MergedReferrerImports> {
    let RewriteOptions {
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
        ..
    } = inputs.options;
    let merged = plan_merged_referrer_imports(
        inputs.candidates,
        &inputs.groups.by_source,
        inputs.sources,
        &inputs.parsed_sources,
        &inputs.known_paths,
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
    )?;
    let edits = merged.edits;
    for (path, start, end, replacement) in edits {
        edits_by_path.entry(path).or_default().push(TextEdit {
            start,
            end,
            replacement,
        });
    }
    Ok(MergedReferrerImports {
        edits: Vec::new(),
        ranges: merged.ranges,
        absorbed_partials: merged.absorbed_partials,
        extra_partial_names: merged.extra_partial_names,
    })
}

fn plan_source_moves(
    inputs: &RewriteInputs<'_>,
    destinations: &mut DestinationPlan,
    edits_by_path: &mut BTreeMap<String, Vec<TextEdit>>,
) -> Result<()> {
    let RewriteOptions {
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
        compatibility_quote_style,
        ..
    } = inputs.options;
    for (source_path, source_candidates) in &inputs.groups.by_source {
        let destination_path = &source_candidates[0].destination_path;
        if inputs.sources.contains_key(destination_path) {
            return Err(TypeTuneError::new(format!(
                "Destination already exists: {destination_path}."
            )));
        }
        let source = inputs.sources.get(source_path).ok_or_else(|| {
            TypeTuneError::new(format!("Could not find source for {source_path}."))
        })?;
        let parsed_source = inputs.parsed_sources.get(source_path).ok_or_else(|| {
            TypeTuneError::new(format!("Could not find source for {source_path}."))
        })?;
        let moved =
            moved_declarations(source_path, source_candidates, &parsed_source.declarations)?;
        let retained_private =
            retained_private_dependencies(source, source_path, source_candidates, &moved)?;
        let retained_private_names = retained_private
            .iter()
            .map(|declaration| declaration.symbol_name.clone())
            .collect::<BTreeSet<_>>();
        let removed = moved
            .iter()
            .copied()
            .filter(|declaration| {
                !retained_private
                    .iter()
                    .any(|retained| retained.start_byte == declaration.start_byte)
            })
            .collect::<Vec<_>>();
        let imported_dependencies = collect_imported_type_dependencies(
            parsed_source,
            &moved,
            source,
            source_path,
            destination_path,
            &inputs.known_paths,
            &inputs.groups.destinations,
        )?;
        let removed_dependencies = collect_imported_type_dependencies(
            parsed_source,
            &removed,
            source,
            source_path,
            destination_path,
            &inputs.known_paths,
            &inputs.groups.destinations,
        )?;
        // Parse the complete remaining source, not just retained type declarations: functions,
        // values, re-exports, and copied private dependencies can all still use an import.
        // Binding-name analysis is conservative: shadowed names may keep an unused import, but
        // never cause a required import to be removed.
        let kept_import_names = if removed_dependencies.statement_ranges.is_empty() {
            HashSet::new()
        } else {
            let edits = removed
                .iter()
                .map(|declaration| declaration_removal_edit(source, declaration))
                .collect::<Vec<_>>();
            parse_import_usage_for_cleanup(&apply_text_edits(source, &edits)?)?.map_or_else(
                || {
                    parsed_source
                        .import_analysis
                        .imports
                        .iter()
                        .map(|import| import.local_name.clone())
                        .collect()
                },
                |usage| usage.binding_names.into_iter().collect(),
            )
        };
        let cleanup_edits = plan_source_cleanup(
            source,
            parsed_source,
            &removed,
            &removed_dependencies.statement_ranges,
            &kept_import_names,
        );
        let has_external_referrers = source_candidates
            .iter()
            .any(|candidate| !candidate.referrer_strategies.is_empty());
        let has_inline_type_referrers = has_inline_type_referrers(
            source_candidates,
            &inputs.parsed_sources,
            &inputs.known_paths,
            base_url,
            path_aliases,
            package_aliases,
            root_dirs,
            module_suffixes,
        );
        let module_specifier_extension = module_specifier_extension_for_source(
            parsed_source,
            inputs.options.default_module_specifier_extension,
        );
        let source_edits = plan_source_compatibility(SourceCompatibility {
            source,
            source_path,
            destination_path,
            source_candidates,
            moved: &moved,
            retained_private_names: &retained_private_names,
            cleanup_edits: &cleanup_edits,
            module_specifier_extension: &module_specifier_extension,
            compatibility_quote_style,
            has_external_referrers,
            has_inline_type_referrers,
        })?;
        destinations.add_source(
            source_path,
            destination_path,
            moved,
            &imported_dependencies.imports,
            !statement_semicolon(source).is_empty(),
        );
        edits_by_path
            .entry(source_path.clone())
            .or_default()
            .extend(source_edits);
    }
    Ok(())
}

/// Private helpers that other remaining declarations still reference stay in the source and are
/// copied into the destination. Exported helpers move and are imported back when needed.
fn retained_private_dependencies<'a>(
    source: &str,
    source_path: &str,
    source_candidates: &[&Candidate],
    moved: &[&'a ParsedDeclaration],
) -> Result<Vec<&'a ParsedDeclaration>> {
    let candidate_keys = source_candidates
        .iter()
        .map(|candidate| candidate.declaration_key.as_str())
        .collect::<HashSet<_>>();
    let analysis_edits = moved
        .iter()
        .map(|declaration| declaration_removal_edit(source, declaration))
        .collect::<Vec<_>>();
    let remaining = apply_text_edits(source, &analysis_edits)?;
    let mut retained: Vec<&ParsedDeclaration> = Vec::new();
    loop {
        let mut grew = false;
        for declaration in moved {
            let Some(key) = declaration_key(source_path, declaration) else {
                continue;
            };
            if declaration.is_exported
                || candidate_keys.contains(key.as_str())
                || retained
                    .iter()
                    .any(|retained| retained.start_byte == declaration.start_byte)
            {
                continue;
            }
            let names = BTreeSet::from([declaration.symbol_name.clone()]);
            let used_by_remaining = source_has_type_usage(&remaining, &names)?;
            let used_by_retained = retained.iter().any(|retained| {
                retained
                    .referenced_type_names
                    .iter()
                    .any(|name| name == &declaration.symbol_name)
            });
            if used_by_remaining || used_by_retained {
                retained.push(*declaration);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    Ok(retained)
}

fn plan_remaining_referrer_edits(
    inputs: &RewriteInputs<'_>,
    merged: &MergedReferrerImports,
    edits_by_path: &mut BTreeMap<String, Vec<TextEdit>>,
) -> Result<()> {
    let RewriteOptions {
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
        ..
    } = inputs.options;
    plan_referrer_edits(
        ReferrerEditContext {
            candidates: inputs.candidates,
            candidates_by_source: &inputs.groups.by_source,
            sources: inputs.sources,
            parsed_sources: &inputs.parsed_sources,
            known_paths: &inputs.known_paths,
            base_url,
            path_aliases,
            package_aliases,
            root_dirs,
            module_suffixes,
            module_specifier_extension: &inputs.module_specifier_extension,
            merged_referrer_import_ranges: &merged.ranges,
            absorbed_partials: &merged.absorbed_partials,
            extra_partial_names: &merged.extra_partial_names,
        },
        edits_by_path,
    )
}

fn apply_planned_edits(
    inputs: &RewriteInputs<'_>,
    edits_by_path: BTreeMap<String, Vec<TextEdit>>,
    output: &mut BTreeMap<String, String>,
) -> Result<()> {
    for (path, mut edits) in edits_by_path {
        let source = inputs
            .sources
            .get(&path)
            .ok_or_else(|| TypeTuneError::new(format!("Could not find source for {path}.")))?;
        for edit in &mut edits {
            edit.replacement = match_line_endings(source, &edit.replacement);
        }
        output.insert(path, apply_text_edits(source, &edits)?);
    }
    Ok(())
}
