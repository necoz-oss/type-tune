//! Specifier-only updates inside supported Vue script blocks.
//!
//! Edits replace quoted module specifiers whose ranges sit inside extracted script bodies. Template,
//! style, and other SFC bytes are copied through unchanged. Imports that would need a statement
//! split, a namespace import, or a barrel update are rejected before any edit is returned.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::{RewriteOptions, format::quoted_module_specifier};
use crate::{
    Result, TypeTuneError,
    edit::{TextEdit, apply_text_edits},
    model::{Candidate, ImportStrategy},
    module_path::{
        resolve_relative_module_with_options,
        resolve_scoped_path_alias_with_base_url_and_module_suffixes,
    },
    parser::{ParsedImport, ParsedReExport, ParsedSource},
    vue::{self, SourceParse},
};

pub(super) fn plan_vue_script_rewrites(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    options: RewriteOptions<'_>,
) -> Result<BTreeMap<String, String>> {
    let mut paths = BTreeSet::new();
    for candidate in candidates {
        for strategy in &candidate.vue_script_strategies {
            paths.insert(strategy.referrer_path.clone());
        }
    }
    let known_paths = sources.keys().cloned().collect::<HashSet<_>>();
    let mut changes = BTreeMap::new();
    for path in paths {
        let source = sources.get(&path).ok_or_else(|| {
            TypeTuneError::new(format!(
                "Could not find Vue referrer {path}. Apply was rejected before any files were changed."
            ))
        })?;
        let SourceParse::Analyzed(analyzed) = vue::parse_analysis_source(&path, source)? else {
            return Err(TypeTuneError::new(format!(
                "Could not analyze Vue referrer {path} while planning script edits. Apply was rejected before any files were changed."
            )));
        };
        if analyzed.script_ranges.is_empty() {
            return Err(TypeTuneError::new(format!(
                "Vue referrer {path} has no supported script block to update. Apply was rejected before any files were changed."
            )));
        }
        let mut edits = BTreeMap::<(usize, usize), String>::new();
        for candidate in candidates {
            for strategy in candidate
                .vue_script_strategies
                .iter()
                .filter(|strategy| strategy.referrer_path == path)
            {
                let specifier = strategy_specifier(&strategy.strategy).ok_or_else(|| {
                    TypeTuneError::new(format!(
                        "Cannot update Vue referrer {path} for {} with a barrel strategy.",
                        candidate.symbol_name
                    ))
                })?;
                replace_matching_specifiers(
                    candidates,
                    candidate,
                    &path,
                    specifier,
                    source,
                    &analyzed.parsed,
                    &analyzed.script_ranges,
                    &known_paths,
                    options,
                    &mut edits,
                )?;
            }
        }
        let text_edits = edits
            .into_iter()
            .map(|((start, end), replacement)| TextEdit {
                start,
                end,
                replacement,
            })
            .collect::<Vec<_>>();
        if text_edits.is_empty() {
            continue;
        }
        let updated = apply_text_edits(source, &text_edits)?;
        if updated != *source {
            changes.insert(path, updated);
        }
    }
    Ok(changes)
}

#[allow(clippy::too_many_arguments)]
fn replace_matching_specifiers(
    candidates: &[Candidate],
    candidate: &Candidate,
    referrer_path: &str,
    specifier: &str,
    source: &str,
    parsed: &ParsedSource,
    script_ranges: &[(usize, usize)],
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
    edits: &mut BTreeMap<(usize, usize), String>,
) -> Result<()> {
    let mut found = false;
    for import in &parsed.import_analysis.imports {
        if !import_targets_candidate(import, candidate, referrer_path, known_paths, options) {
            continue;
        }
        found = true;
        if import.is_namespace || import.is_default {
            return Err(rejected(
                referrer_path,
                &candidate.symbol_name,
                "namespace and default imports need more than a specifier replacement",
            ));
        }
        ensure_named_statement_moves_together(
            candidates,
            &parsed.import_analysis.imports,
            import.statement_start,
            referrer_path,
            specifier,
            known_paths,
            options,
        )?;
        queue_specifier_edit(
            source,
            import.module_specifier_start,
            import.module_specifier_end,
            specifier,
            script_ranges,
            referrer_path,
            edits,
        )?;
    }
    for import in &parsed.import_analysis.inline_type_imports {
        let Some(resolved) = resolve_specifier(
            referrer_path,
            &import.module_specifier,
            known_paths,
            options,
        ) else {
            continue;
        };
        if resolved != candidate.source_path || import.imported_name != candidate.symbol_name {
            continue;
        }
        found = true;
        queue_specifier_edit(
            source,
            import.module_specifier_start,
            import.module_specifier_end,
            specifier,
            script_ranges,
            referrer_path,
            edits,
        )?;
    }
    for export in &parsed.re_exports {
        if !reexport_targets_candidate(export, candidate, referrer_path, known_paths, options) {
            continue;
        }
        found = true;
        if export.is_wildcard || export.is_namespace_wildcard {
            return Err(rejected(
                referrer_path,
                &candidate.symbol_name,
                "wildcard re-exports need more than a specifier replacement",
            ));
        }
        ensure_reexport_statement_moves_together(
            candidates,
            &parsed.re_exports,
            export.statement_start,
            referrer_path,
            specifier,
            known_paths,
            options,
        )?;
        queue_specifier_edit(
            source,
            export.module_specifier_start,
            export.module_specifier_end,
            specifier,
            script_ranges,
            referrer_path,
            edits,
        )?;
    }
    if found {
        Ok(())
    } else {
        Err(rejected(
            referrer_path,
            &candidate.symbol_name,
            "the planned import was not found in a supported script block",
        ))
    }
}

fn ensure_named_statement_moves_together(
    candidates: &[Candidate],
    imports: &[ParsedImport],
    statement_start: usize,
    referrer_path: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> Result<()> {
    for import in imports
        .iter()
        .filter(|import| import.statement_start == statement_start)
    {
        if import.is_namespace || import.is_default {
            return Err(rejected(
                referrer_path,
                &import.imported_name,
                "the import statement is not a named type import",
            ));
        }
        ensure_binding_uses_specifier(
            candidates,
            referrer_path,
            &import.imported_name,
            &import.module_specifier,
            specifier,
            known_paths,
            options,
        )?;
    }
    Ok(())
}

fn ensure_reexport_statement_moves_together(
    candidates: &[Candidate],
    exports: &[ParsedReExport],
    statement_start: usize,
    referrer_path: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> Result<()> {
    for export in exports
        .iter()
        .filter(|export| export.statement_start == statement_start)
    {
        if export.is_wildcard || export.is_namespace_wildcard {
            return Err(rejected(
                referrer_path,
                &export.source_name,
                "the re-export statement is not a named type re-export",
            ));
        }
        ensure_binding_uses_specifier(
            candidates,
            referrer_path,
            &export.source_name,
            &export.module_specifier,
            specifier,
            known_paths,
            options,
        )?;
    }
    Ok(())
}

fn ensure_binding_uses_specifier(
    candidates: &[Candidate],
    referrer_path: &str,
    imported_name: &str,
    module_specifier: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> Result<()> {
    let Some(resolved) = resolve_specifier(referrer_path, module_specifier, known_paths, options)
    else {
        return Err(rejected(
            referrer_path,
            imported_name,
            "a binding in the same statement could not be resolved",
        ));
    };
    let Some(owner) = candidates.iter().find(|candidate| {
        candidate.source_path == resolved && candidate.symbol_name == imported_name
    }) else {
        return Err(rejected(
            referrer_path,
            imported_name,
            "the statement also binds a symbol that is not part of this move",
        ));
    };
    let Some(strategy) = owner
        .vue_script_strategies
        .iter()
        .find(|strategy| strategy.referrer_path == referrer_path)
    else {
        return Err(rejected(
            referrer_path,
            imported_name,
            "the statement also binds a symbol without a Vue script strategy",
        ));
    };
    let Some(sibling_specifier) = strategy_specifier(&strategy.strategy) else {
        return Err(rejected(
            referrer_path,
            imported_name,
            "the statement also binds a symbol with a barrel strategy",
        ));
    };
    if sibling_specifier == specifier {
        Ok(())
    } else {
        Err(rejected(
            referrer_path,
            imported_name,
            "the statement binds symbols that move to different modules",
        ))
    }
}

fn queue_specifier_edit(
    source: &str,
    start: usize,
    end: usize,
    specifier: &str,
    script_ranges: &[(usize, usize)],
    referrer_path: &str,
    edits: &mut BTreeMap<(usize, usize), String>,
) -> Result<()> {
    if !script_ranges
        .iter()
        .any(|(range_start, range_end)| start >= *range_start && end <= *range_end)
    {
        return Err(TypeTuneError::new(format!(
            "Refusing to edit {referrer_path} outside a supported script block. Apply was rejected before any files were changed."
        )));
    }
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return Err(TypeTuneError::new(format!(
            "Invalid specifier range in {referrer_path}. Apply was rejected before any files were changed."
        )));
    }
    let original = &source[start..end];
    let replacement = quoted_module_specifier(original, specifier);
    if original == replacement {
        return Ok(());
    }
    if let Some(existing) = edits.get(&(start, end))
        && existing != &replacement
    {
        return Err(TypeTuneError::new(format!(
            "Vue referrer {referrer_path} needs two different specifiers at the same import. Apply was rejected before any files were changed."
        )));
    }
    edits.insert((start, end), replacement);
    Ok(())
}

fn import_targets_candidate(
    import: &ParsedImport,
    candidate: &Candidate,
    referrer_path: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> bool {
    let Some(resolved) = resolve_specifier(
        referrer_path,
        &import.module_specifier,
        known_paths,
        options,
    ) else {
        return false;
    };
    resolved == candidate.source_path && import.imported_name == candidate.symbol_name
}

fn reexport_targets_candidate(
    export: &ParsedReExport,
    candidate: &Candidate,
    referrer_path: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> bool {
    let Some(resolved) = resolve_specifier(
        referrer_path,
        &export.module_specifier,
        known_paths,
        options,
    ) else {
        return false;
    };
    resolved == candidate.source_path && export.source_name == candidate.symbol_name
}

fn resolve_specifier(
    referrer_path: &str,
    module_specifier: &str,
    known_paths: &HashSet<String>,
    options: RewriteOptions<'_>,
) -> Option<String> {
    resolve_relative_module_with_options(
        referrer_path,
        module_specifier,
        known_paths,
        options.root_dirs,
        options.module_suffixes,
    )
    .or_else(|| {
        resolve_scoped_path_alias_with_base_url_and_module_suffixes(
            referrer_path,
            module_specifier,
            options.base_url,
            options.path_aliases,
            options.package_aliases,
            known_paths,
            options.module_suffixes,
        )
    })
}

fn strategy_specifier(strategy: &ImportStrategy) -> Option<&str> {
    match strategy {
        ImportStrategy::Alias { module_specifier }
        | ImportStrategy::Relative { module_specifier } => Some(module_specifier),
        ImportStrategy::Barrel { .. } => None,
    }
}

fn rejected(path: &str, symbol: &str, detail: &str) -> TypeTuneError {
    TypeTuneError::new(format!(
        "Cannot update Vue referrer {path} for {symbol} because {detail}. Apply was rejected before any files were changed."
    ))
}
