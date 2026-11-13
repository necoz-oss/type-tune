//! TypeScript project discovery shared by CLI commands.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use type_tune_core::model::{Candidate, ImportStrategy};

/// Loads TypeScript project configuration for CLI commands.
pub(crate) fn load_project_config(
    project: &Path,
) -> Result<type_tune_core::config::ProjectConfig, String> {
    type_tune_core::config::load_project_config(project).map_err(|error| error.to_string())
}

/// Resolves the root directory configured for a TypeScript project.
pub(crate) fn project_root(project: &Path) -> Result<PathBuf, String> {
    load_project_config(project).map(|config| config.project_root)
}

/// Resolves the configured TypeScript project before compiler validation.
pub(crate) fn compiler_project(project: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(project)
        .map_err(|error| format!("Could not resolve {}: {error}", project.display()))
}

/// Normalizes a user path and rejects paths outside the selected project.
pub(crate) fn normalize_project_input_path(
    project_root: &Path,
    value: &str,
) -> Result<String, String> {
    let project_root = fs::canonicalize(project_root).map_err(|error| {
        format!(
            "Could not resolve project root {}: {error}",
            project_root.display()
        )
    })?;
    let relative = if Path::new(value).is_absolute() {
        project_relative_from_absolute(&project_root, Path::new(value), value)?
    } else {
        PathBuf::from(value)
    };
    type_tune_core::path::normalize_project_path(&relative.to_string_lossy())
        .map_err(|_| format!("Path must be inside the TypeScript project: {value}."))
}

/// Loads declaration sources for plan construction.
pub(crate) fn read_candidate_sources(
    project: &Path,
    candidates: &[Candidate],
) -> Result<BTreeMap<String, String>, String> {
    let root = project_root(project)?;
    candidates
        .iter()
        .map(|candidate| {
            let path = root.join(PathBuf::from(&candidate.source_path));
            fs::read_to_string(&path)
                .map(|source| (candidate.source_path.clone(), source))
                .map_err(|error| format!("Could not read {}: {error}", path.display()))
        })
        .collect()
}

/// Loads every source potentially changed by a rewrite, including barrel exports.
pub(crate) fn read_rewrite_sources(
    project: &Path,
    candidates: &[Candidate],
) -> Result<BTreeMap<String, String>, String> {
    let root = project_root(project)?;
    let discovered = type_tune_core::source::discover_typescript_resolution_files(&root, true)
        .map_err(|error| format!("Could not discover rewrite sources: {error}"))?
        .into_iter()
        .map(|path| {
            path.strip_prefix(&root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(|error| {
                    format!(
                        "Could not make rewrite source project-relative ({}): {error}",
                        path.display()
                    )
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    discovered
        .into_iter()
        .chain(candidates.iter().flat_map(|candidate| {
            std::iter::once(candidate.source_path.clone())
                .chain(candidate.referrer_strategies.iter().flat_map(|strategy| {
                    std::iter::once(strategy.referrer_path.clone()).chain(
                        match &strategy.strategy {
                            ImportStrategy::Barrel {
                                barrel_file_path, ..
                            } => Some(barrel_file_path.clone()),
                            ImportStrategy::Alias { .. } | ImportStrategy::Relative { .. } => None,
                        },
                    )
                }))
                .chain(
                    candidate
                        .vue_script_strategies
                        .iter()
                        .map(|strategy| strategy.referrer_path.clone()),
                )
                .chain(
                    candidate
                        .vue_rewrite_blockers
                        .iter()
                        .map(|blocker| blocker.path.clone()),
                )
        }))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|source_path| {
            let path = root.join(PathBuf::from(&source_path));
            match type_tune_core::source::read_typescript_source_text(&path) {
                Ok(Some(source)) => Some(Ok((source_path, source))),
                Ok(None) => None,
                Err(error) => Some(Err(format!("Could not read {}: {error}", path.display()))),
            }
        })
        .collect()
}

/// Adds package aliases needed to resolve the supplied project-relative sources.
pub(crate) fn add_source_package_aliases(
    config: &mut type_tune_core::config::ProjectConfig,
    sources: &BTreeMap<String, String>,
) -> Result<(), String> {
    let source_paths = sources
        .keys()
        .map(|path| config.project_root.join(path))
        .collect::<Vec<_>>();
    type_tune_core::config::add_package_module_aliases_for_sources(config, &source_paths)
        .map_err(|error| error.to_string())
}

/// Plans a rewrite using the module-resolution settings loaded for the project.
pub(crate) fn plan_extraction_rewrite(
    config: &type_tune_core::config::ProjectConfig,
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    compatibility_quote_style: type_tune_core::rewrite::CompatibilityReexportQuoteStyle,
) -> type_tune_core::Result<BTreeMap<String, String>> {
    type_tune_core::rewrite::plan_extraction_rewrite_with_options(
        candidates,
        sources,
        type_tune_core::rewrite::RewriteOptions {
            base_url: config.base_url.as_deref(),
            path_aliases: &config.explicit_paths,
            package_aliases: &config.package_aliases,
            root_dirs: &config.root_dirs,
            module_suffixes: &config.module_suffixes,
            default_module_specifier_extension: if config.package_node_condition {
                ".js"
            } else {
                ""
            },
            compatibility_quote_style,
        },
    )
}

fn project_relative_from_absolute(
    project_root: &Path,
    absolute: &Path,
    original_value: &str,
) -> Result<PathBuf, String> {
    if let Ok(canonical) = fs::canonicalize(absolute) {
        return canonical
            .strip_prefix(project_root)
            .map(|path| path.to_path_buf())
            .map_err(|_| format!("Path must be inside the TypeScript project: {original_value}."));
    }
    let project_components = clean_path_components(project_root);
    let absolute_components = clean_path_components(absolute);
    let shared = common_component_prefix_len(&project_components, &absolute_components);
    if shared != project_components.len() {
        return Err(format!(
            "Path must be inside the TypeScript project: {original_value}."
        ));
    }
    Ok(absolute_components[shared..].iter().collect())
}

fn clean_path_components(path: &Path) -> Vec<Component<'_>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components
}
fn common_component_prefix_len(left: &[Component<'_>], right: &[Component<'_>]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{compiler_project, normalize_project_input_path};

    #[test]
    fn reports_the_unresolvable_compiler_project_path() {
        let missing = format!(
            "type-tune-missing-project-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let error = compiler_project(Path::new(&missing)).expect_err("missing project fails");

        assert!(error.contains(&missing));
        assert!(error.starts_with("Could not resolve "));
    }

    #[test]
    fn normalizes_absolute_paths_inside_the_project() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-project-path-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src")).expect("creates fixture");
        std::fs::write(root.join("tsconfig.json"), "{}\n").expect("writes config");
        let source_path = root.join("src/shared.ts");
        std::fs::write(&source_path, "export type Shared = {};\n").expect("writes source");

        let normalized = normalize_project_input_path(&root, source_path.to_str().expect("utf8"))
            .expect("normalizes source");
        assert_eq!(normalized, "src/shared.ts");

        let destination_path = root.join("src/common/shared.ts");
        let normalized =
            normalize_project_input_path(&root, destination_path.to_str().expect("utf8"))
                .expect("normalizes destination");
        assert_eq!(normalized, "src/common/shared.ts");
        std::fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn rejects_absolute_paths_outside_the_project() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-project-boundary-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let outside = std::env::temp_dir().join(format!(
            "type-tune-outside-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("creates project");
        std::fs::write(root.join("tsconfig.json"), "{}\n").expect("writes config");
        std::fs::write(&outside, "export type Outside = {};\n").expect("writes outside file");

        let error = normalize_project_input_path(&root, outside.to_str().expect("utf8"))
            .expect_err("rejects outside path");

        assert_eq!(
            error,
            format!(
                "Path must be inside the TypeScript project: {}.",
                outside.display()
            )
        );
        std::fs::remove_dir_all(root).expect("removes project");
        std::fs::remove_file(outside).expect("removes outside file");
    }
}
