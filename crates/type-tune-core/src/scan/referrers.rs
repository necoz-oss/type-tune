//! Referrer analysis and import-strategy assignment.

use super::{
    barrels::resolve_barrel_exports, declarations::scan_parsed_declarations_with_unsupported,
    destination_index_path,
};
use crate::{
    Result,
    model::{
        ANALYSIS_LIMITATION_UNRESOLVED_VUE, AnalysisLimitation, Candidate, ImportStrategy,
        READONLY_REFERRER_VUE_SFC, ReadOnlyReferrer, ReferrerStrategy,
    },
    module_path::{
        PackageAliasScopes, is_configured_path_alias,
        resolve_alias_module_specifier_with_known_paths, resolve_relative_module_with_options,
        resolve_scoped_path_alias_with_base_url_and_module_suffixes,
    },
    module_specifier::{
        is_relative_module_specifier, preserved_module_specifier_extension,
        relative_module_specifier, relative_module_specifier_with_extension,
    },
    source_role::SourceRole,
    vue::AnalyzedSource,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
enum ReferrerImportStyle {
    Alias {
        original_specifier: String,
    },
    Barrel {
        path: String,
        extension: String,
        original_specifier: String,
    },
    NamespaceRelative,
    Relative(String),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn scan_parsed_project_files_with_aliases(
    parsed_files: Vec<AnalyzedSource>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    package_aliases: &PackageAliasScopes,
    root_dirs: &[String],
    module_suffixes: &[String],
    include_unsupported: bool,
    preserve_path_aliases: bool,
    default_module_specifier_extension: &str,
    limitations: &[AnalysisLimitation],
) -> Result<Vec<Candidate>> {
    let known_paths = parsed_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>();
    let writable_files = parsed_files
        .iter()
        .filter(|file| file.role == SourceRole::WritableTypeScript)
        .map(|file| (file.path.clone(), file.parsed.clone()))
        .collect::<Vec<_>>();
    let mut candidates =
        scan_parsed_declarations_with_unsupported(&writable_files, include_unsupported);
    let barrel_exports = resolve_barrel_exports(
        &writable_files,
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
        &known_paths,
    )?;
    let mut referrer_styles = HashMap::new();
    for file in &parsed_files {
        let referrer_path = &file.path;
        let parsed_source = &file.parsed;
        let imports = &parsed_source.import_analysis.imports;
        let inline_type_imports = &parsed_source.import_analysis.inline_type_imports;
        let import_usage = &parsed_source.import_analysis.usage;
        for import in imports {
            let Some(source_path) = resolve_relative_module_with_options(
                referrer_path,
                &import.module_specifier,
                &known_paths,
                root_dirs,
                module_suffixes,
            )
            .or_else(|| {
                resolve_scoped_path_alias_with_base_url_and_module_suffixes(
                    referrer_path,
                    &import.module_specifier,
                    base_url,
                    path_aliases,
                    package_aliases,
                    &known_paths,
                    module_suffixes,
                )
            }) else {
                continue;
            };
            if !import_usage.binding_names.contains(&import.local_name) {
                continue;
            }
            let namespace_members = import_usage.namespace_members.get(&import.local_name);
            let barrel_candidates = [
                barrel_exports.get(&(source_path.clone(), import.imported_name.clone())),
                barrel_exports.get(&(source_path.clone(), "*".into())),
            ];
            for candidate in &mut candidates {
                let imported_candidate = candidate.symbol_name == import.imported_name
                    || (import.is_namespace
                        && namespace_members
                            .is_some_and(|members| members.contains(&candidate.symbol_name)));
                let direct_import = candidate.source_path == source_path && imported_candidate;
                let matching_barrel_exports = barrel_candidates
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|export| {
                        export.source_path == candidate.source_path
                            && ((!export.is_wildcard
                                && export.source_name == candidate.symbol_name)
                                || (export.is_wildcard && imported_candidate)
                                || (export.is_namespace_wildcard
                                    && import.imported_name == export.exported_name
                                    && namespace_members.is_some_and(|members| {
                                        members.contains(&candidate.symbol_name)
                                    })))
                    })
                    .collect::<Vec<_>>();
                let barrel_import = !matching_barrel_exports.is_empty();
                if (direct_import || barrel_import) && referrer_path != &candidate.source_path {
                    if file.role == SourceRole::ReadOnlyVue {
                        candidate.readonly_referrers.push(ReadOnlyReferrer {
                            path: referrer_path.clone(),
                            reason: READONLY_REFERRER_VUE_SFC.to_owned(),
                        });
                        continue;
                    }
                    candidate.referrer_paths.push(referrer_path.clone());
                    let style = if let Some(export) =
                        matching_barrel_exports.iter().find(|export| {
                            !export.is_namespace_wildcard
                                && (export.is_wildcard
                                    || import.imported_name == candidate.symbol_name)
                        }) {
                        ReferrerImportStyle::Barrel {
                            path: export.barrel_path.clone(),
                            extension: module_specifier_extension(
                                &export.module_specifier,
                                default_module_specifier_extension,
                            ),
                            original_specifier: export.module_specifier.clone(),
                        }
                    } else if matching_barrel_exports
                        .iter()
                        .any(|export| export.is_namespace_wildcard)
                    {
                        ReferrerImportStyle::NamespaceRelative
                    } else if !is_relative_module_specifier(&import.module_specifier)
                        && preserve_path_aliases
                        && is_configured_path_alias(&import.module_specifier, path_aliases)
                    {
                        ReferrerImportStyle::Alias {
                            original_specifier: import.module_specifier.clone(),
                        }
                    } else {
                        ReferrerImportStyle::Relative(module_specifier_extension(
                            &import.module_specifier,
                            default_module_specifier_extension,
                        ))
                    };
                    referrer_styles.insert(
                        (
                            candidate.source_path.clone(),
                            candidate.symbol_name.clone(),
                            referrer_path.clone(),
                        ),
                        style,
                    );
                }
            }
        }
        for import in inline_type_imports {
            let Some(source_path) = resolve_relative_module_with_options(
                referrer_path,
                &import.module_specifier,
                &known_paths,
                root_dirs,
                module_suffixes,
            )
            .or_else(|| {
                resolve_scoped_path_alias_with_base_url_and_module_suffixes(
                    referrer_path,
                    &import.module_specifier,
                    base_url,
                    path_aliases,
                    package_aliases,
                    &known_paths,
                    module_suffixes,
                )
            }) else {
                continue;
            };
            let barrel_candidates = [
                barrel_exports.get(&(source_path.clone(), import.imported_name.clone())),
                barrel_exports.get(&(source_path.clone(), "*".into())),
            ];
            for candidate in &mut candidates {
                let direct_import = candidate.source_path == source_path
                    && candidate.symbol_name == import.imported_name;
                let barrel_import =
                    barrel_candidates
                        .into_iter()
                        .flatten()
                        .flatten()
                        .any(|export| {
                            export.source_path == candidate.source_path
                                && ((!export.is_wildcard
                                    && export.source_name == candidate.symbol_name)
                                    || (export.is_wildcard
                                        && import.imported_name == candidate.symbol_name))
                        });
                if !(direct_import || barrel_import) || referrer_path == &candidate.source_path {
                    continue;
                }
                if file.role == SourceRole::ReadOnlyVue {
                    candidate.readonly_referrers.push(ReadOnlyReferrer {
                        path: referrer_path.clone(),
                        reason: READONLY_REFERRER_VUE_SFC.to_owned(),
                    });
                    continue;
                }
                candidate.referrer_paths.push(referrer_path.clone());
                let style = ReferrerImportStyle::Relative(module_specifier_extension(
                    &import.module_specifier,
                    default_module_specifier_extension,
                ));
                referrer_styles
                    .entry((
                        candidate.source_path.clone(),
                        candidate.symbol_name.clone(),
                        referrer_path.clone(),
                    ))
                    .or_insert(style);
            }
        }
    }
    for candidate in &mut candidates {
        candidate.referrer_paths.sort();
        candidate.referrer_paths.dedup();
        candidate.readonly_referrers.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.reason.cmp(&right.reason))
        });
        candidate.readonly_referrers.dedup();
        let mut unique_referrers = BTreeSet::new();
        unique_referrers.extend(candidate.referrer_paths.iter().cloned());
        unique_referrers.extend(
            candidate
                .readonly_referrers
                .iter()
                .map(|referrer| referrer.path.clone()),
        );
        candidate.referrer_count = unique_referrers.len();
        candidate.analysis_limitations = limitations
            .iter()
            .filter(|limitation| limitation.reason == ANALYSIS_LIMITATION_UNRESOLVED_VUE)
            .cloned()
            .collect();
        candidate.referrer_strategies = candidate
            .referrer_paths
            .iter()
            .map(|referrer_path| ReferrerStrategy {
                referrer_path: referrer_path.clone(),
                strategy: match referrer_styles.get(&(
                    candidate.source_path.clone(),
                    candidate.symbol_name.clone(),
                    referrer_path.clone(),
                )) {
                    Some(ReferrerImportStyle::Barrel {
                        path,
                        extension,
                        original_specifier,
                    }) => {
                        let specifier = if preserve_path_aliases
                            && !is_relative_module_specifier(original_specifier)
                            && is_configured_path_alias(original_specifier, path_aliases)
                        {
                            resolve_alias_module_specifier_with_known_paths(
                                &candidate.destination_path,
                                base_url,
                                path_aliases,
                                Some(original_specifier),
                                Some(&known_paths),
                                module_suffixes,
                                extension,
                            )
                            .map(|alias| format!("{alias}{extension}"))
                        } else {
                            None
                        }
                        .unwrap_or_else(|| {
                            relative_module_specifier_with_extension(
                                path,
                                &candidate.destination_path,
                                extension,
                            )
                        });
                        ImportStrategy::Barrel {
                            barrel_file_path: path.clone(),
                            export_module_specifier: specifier,
                        }
                    }
                    Some(ReferrerImportStyle::Alias { original_specifier }) => {
                        let extension = module_specifier_extension(
                            original_specifier,
                            default_module_specifier_extension,
                        );
                        resolve_alias_module_specifier_with_known_paths(
                            &candidate.destination_path,
                            base_url,
                            path_aliases,
                            Some(original_specifier),
                            Some(&known_paths),
                            module_suffixes,
                            &extension,
                        )
                        .map(|module_specifier| ImportStrategy::Alias {
                            module_specifier: format!("{module_specifier}{extension}"),
                        })
                        .unwrap_or_else(|| ImportStrategy::Relative {
                            module_specifier: relative_module_specifier_with_extension(
                                referrer_path,
                                &candidate.destination_path,
                                &extension,
                            ),
                        })
                    }
                    Some(ReferrerImportStyle::NamespaceRelative) => ImportStrategy::Relative {
                        module_specifier: relative_module_specifier(
                            referrer_path,
                            &candidate.destination_path,
                        ),
                    },
                    Some(ReferrerImportStyle::Relative(extension)) => ImportStrategy::Relative {
                        module_specifier: relative_module_specifier_with_extension(
                            referrer_path,
                            &candidate.destination_path,
                            extension,
                        ),
                    },
                    None => ImportStrategy::Relative {
                        module_specifier: relative_module_specifier_with_extension(
                            referrer_path,
                            &candidate.destination_path,
                            default_module_specifier_extension,
                        ),
                    },
                },
            })
            .collect();
        candidate.destination_index_path = destination_index_path(&candidate.referrer_strategies);
    }
    Ok(candidates)
}

fn module_specifier_extension(specifier: &str, default_extension: &str) -> String {
    preserved_module_specifier_extension(specifier)
        .unwrap_or(default_extension)
        .to_owned()
}
