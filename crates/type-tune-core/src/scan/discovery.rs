//! TypeScript project source discovery for scans.

use crate::{
    Result, TypeTuneError,
    config::{add_package_module_aliases_for_sources, load_project_config},
    glob,
    module_path::{
        resolve_relative_module_with_options,
        resolve_scoped_path_alias_with_base_url_and_module_suffixes,
    },
    path::normalize_project_path,
    source::{
        discover_analysis_files, filter_project_paths, path_is_included,
        read_typescript_source_text,
    },
    vue::{self, SourceParse},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::Path,
};

fn read_project_source_text(path: &Path) -> Result<Option<String>> {
    read_typescript_source_text(path)
        .map_err(|error| TypeTuneError::new(format!("Could not read {}: {error}", path.display())))
}

pub(super) fn read_project_source_pair(
    root: &Path,
    path: &Path,
) -> Result<Option<(String, String)>> {
    let source_path = path
        .strip_prefix(root)
        .map_err(|error| {
            TypeTuneError::new(format!("Could not make project-relative path: {error}"))
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let Some(source) = read_project_source_text(path)? else {
        return Ok(None);
    };
    Ok(Some((source_path, source)))
}

pub(super) fn load_tsconfig_source_files(
    tsconfig_path: &Path,
    include_generated: bool,
) -> Result<(Vec<(String, String)>, crate::config::ProjectConfig)> {
    let mut config = load_project_config(tsconfig_path)?;
    let root = config.project_root.clone();
    let mut discovered = discover_analysis_files(&root, include_generated)
        .map_err(|error| TypeTuneError::new(format!("Could not discover sources: {error}")))?;
    let (explicit_packages, package_sources) =
        discover_explicit_package_sources(&root, config.files.as_deref(), include_generated)?;
    discovered.extend(package_sources);
    discovered.extend(discover_included_package_sources(
        &root,
        config.include.as_deref(),
        include_generated,
    )?);
    discovered.sort();
    discovered.dedup();
    let mut paths = configured_source_paths(&root, &discovered, &config);
    paths.extend(explicit_packages);
    paths.sort();
    paths.dedup();
    let universe = paths_by_relative(&root, &discovered);
    loop {
        add_package_module_aliases_for_sources(&mut config, &paths)?;
        let expanded = expand_imported_project_files(&root, paths.clone(), &universe, &config)?;
        if expanded == paths {
            break;
        }
        paths = expanded;
    }
    let files = paths
        .into_iter()
        .filter_map(|path| read_project_source_pair(&root, &path).transpose())
        .collect::<Result<Vec<_>>>()?;
    Ok((files, config))
}

fn discover_included_package_sources(
    root: &Path,
    includes: Option<&[String]>,
    include_generated: bool,
) -> Result<Vec<std::path::PathBuf>> {
    let mut package_roots = BTreeSet::new();
    for pattern in includes.into_iter().flatten() {
        let segments: Vec<_> = pattern.split('/').collect();
        let Some(index) = segments
            .iter()
            .rposition(|segment| *segment == "node_modules")
        else {
            continue;
        };
        let mut directories = BTreeSet::new();
        let canonical = root.canonicalize().map_err(|error| {
            TypeTuneError::new(format!("Could not resolve {}: {error}", root.display()))
        })?;
        collect_matching_directories(
            root,
            &segments[..=index],
            include_generated,
            &mut HashSet::from([canonical]),
            &mut directories,
        )?;
        for directory in directories {
            let suffix = &segments[index + 1..];
            let mut package_root = directory;
            if let Some(name) = suffix.first().filter(|name| !name.contains(['*', '?'])) {
                if *name == ".git"
                    || (!include_generated && matches!(*name, "generated" | "__generated__"))
                {
                    continue;
                }
                let child = package_root.join(name);
                if !child.exists() {
                    continue;
                }
                if child.is_dir() {
                    package_root = child;
                    if name.starts_with('@')
                        && let Some(package) =
                            suffix.get(1).filter(|name| !name.contains(['*', '?']))
                    {
                        if *package == ".git"
                            || (!include_generated
                                && matches!(*package, "generated" | "__generated__"))
                        {
                            continue;
                        }
                        let child = package_root.join(package);
                        if !child.exists() {
                            continue;
                        }
                        if child.is_dir() {
                            package_root = child;
                        }
                    }
                }
            }
            package_roots.insert(package_root);
        }
    }
    let mut sources = BTreeSet::new();
    for package_root in package_roots {
        sources.extend(
            discover_analysis_files(&package_root, include_generated).map_err(|error| {
                TypeTuneError::new(format!("Could not discover sources: {error}"))
            })?,
        );
    }
    Ok(sources.into_iter().collect())
}

fn collect_matching_directories(
    directory: &Path,
    pattern: &[&str],
    include_generated: bool,
    ancestors: &mut HashSet<std::path::PathBuf>,
    found: &mut BTreeSet<std::path::PathBuf>,
) -> Result<()> {
    let Some((part, rest)) = pattern.split_first() else {
        found.insert(directory.to_path_buf());
        return Ok(());
    };
    if *part == "**" {
        collect_matching_directories(directory, rest, include_generated, ancestors, found)?;
    }
    let entries = std::fs::read_dir(directory).map_err(|error| {
        TypeTuneError::new(format!("Could not read {}: {error}", directory.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            TypeTuneError::new(format!("Could not read {}: {error}", directory.display()))
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let wildcard = part.contains(['*', '?']);
        if name == ".git"
            || (!include_generated && matches!(name.as_ref(), "generated" | "__generated__"))
            || (wildcard
                && ((name.starts_with('.') && !part.starts_with('.'))
                    || matches!(
                        name.as_ref(),
                        "node_modules" | "bower_components" | "jspm_packages"
                    )))
            || (*part != "**" && !glob::matches(&name, part))
        {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let real = path.canonicalize().map_err(|error| {
            TypeTuneError::new(format!("Could not resolve {}: {error}", path.display()))
        })?;
        if !ancestors.insert(real.clone()) {
            continue;
        }
        let next = if *part == "**" { pattern } else { rest };
        let result = collect_matching_directories(&path, next, include_generated, ancestors, found);
        ancestors.remove(&real);
        result?;
    }
    Ok(())
}

fn discover_explicit_package_sources(
    root: &Path,
    files: Option<&[String]>,
    include_generated: bool,
) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> {
    let mut explicit = BTreeSet::new();
    let mut package_roots = BTreeSet::new();
    for file in files.into_iter().flatten() {
        let Ok(relative) = normalize_project_path(file) else {
            continue;
        };
        let segments: Vec<_> = relative.split('/').collect();
        let Some(index) = segments
            .iter()
            .rposition(|segment| *segment == "node_modules")
        else {
            continue;
        };
        let name = segments.last().copied().unwrap_or("");
        if segments.contains(&".git")
            || !matches!(
                Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str()),
                Some("ts" | "tsx" | "vue")
            )
            || name.ends_with(".d.ts")
            || (!include_generated
                && (segments
                    .iter()
                    .any(|segment| matches!(*segment, "generated" | "__generated__"))
                    || name.ends_with(".generated.ts")
                    || name.ends_with(".generated.tsx")
                    || name.ends_with(".generated.vue")))
        {
            continue;
        }
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        explicit.insert(path);
        let package_end = index
            + if segments
                .get(index + 1)
                .is_some_and(|part| part.starts_with('@'))
            {
                3
            } else {
                2
            };
        if package_end < segments.len() {
            package_roots.insert(root.join(segments[..package_end].join("/")));
        }
    }
    let mut sources = explicit.clone();
    for package_root in package_roots {
        let files = discover_analysis_files(&package_root, include_generated)
            .map_err(|error| TypeTuneError::new(format!("Could not discover sources: {error}")))?;
        sources.extend(files);
    }
    Ok((
        explicit.into_iter().collect(),
        sources.into_iter().collect(),
    ))
}

fn configured_source_paths(
    root: &Path,
    discovered: &[std::path::PathBuf],
    config: &crate::config::ProjectConfig,
) -> Vec<std::path::PathBuf> {
    let mut selected = BTreeSet::new();
    if let Some(files) = &config.files {
        let configured_files = files
            .iter()
            .map(|path| path.replace('\\', "/").trim_start_matches("./").to_owned())
            .collect::<BTreeSet<_>>();
        for path in discovered {
            let Some(relative) = path.strip_prefix(root).ok() else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if configured_files.contains(&relative) {
                selected.insert(path.clone());
            }
        }
    }
    let includes = match &config.include {
        Some(patterns) => patterns.clone(),
        None if config.files.is_none() => {
            vec!["**/*.ts".into(), "**/*.tsx".into(), "**/*.vue".into()]
        }
        None => Vec::new(),
    };
    if !includes.is_empty() {
        selected.extend(
            filter_project_paths(discovered.to_vec(), root, &includes, &config.exclude)
                .into_iter()
                .filter(|path| {
                    path.strip_prefix(root).is_ok_and(|relative| {
                        let relative = relative.to_string_lossy().replace('\\', "/");
                        include_matches_discovery_rules(&relative, &includes)
                    })
                }),
        );
    }
    selected.into_iter().collect()
}

fn include_matches_discovery_rules(path: &str, includes: &[String]) -> bool {
    let directories = ["node_modules", "bower_components", "jspm_packages"].map(|name| {
        (
            name,
            path.split('/').filter(|segment| *segment == name).count(),
        )
    });
    let hidden_count = path.split('/').filter(|part| part.starts_with('.')).count();
    (hidden_count == 0 && directories.iter().all(|(_, count)| *count == 0))
        || includes.iter().any(|pattern| {
            path_is_included(path, std::slice::from_ref(pattern))
                && pattern
                    .split('/')
                    .filter(|part| part.starts_with('.'))
                    .count()
                    >= hidden_count
                && directories.iter().all(|(name, count)| {
                    pattern.split('/').filter(|part| part == name).count() >= *count
                })
        })
}

fn paths_by_relative(
    root: &Path,
    paths: &[std::path::PathBuf],
) -> BTreeMap<String, std::path::PathBuf> {
    paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix(root)
                .ok()
                .map(|relative| (relative.to_string_lossy().replace('\\', "/"), path.clone()))
        })
        .collect()
}

fn expand_imported_project_files(
    root: &Path,
    selected: Vec<std::path::PathBuf>,
    universe: &BTreeMap<String, std::path::PathBuf>,
    config: &crate::config::ProjectConfig,
) -> Result<Vec<std::path::PathBuf>> {
    let known_paths = universe.keys().cloned().collect::<HashSet<_>>();
    let mut included = selected
        .iter()
        .filter_map(|path| {
            path.strip_prefix(root)
                .ok()
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect::<BTreeSet<_>>();
    let mut queue = included.iter().cloned().collect::<Vec<_>>();
    while let Some(source_path) = queue.pop() {
        let Some(path) = universe.get(&source_path) else {
            continue;
        };
        let Some(source) = read_project_source_text(path)? else {
            continue;
        };
        let SourceParse::Analyzed(analyzed) = vue::parse_analysis_source(&source_path, &source)?
        else {
            continue;
        };
        for specifier in analyzed.parsed.module_specifiers() {
            let resolved = resolve_relative_module_with_options(
                source_path.as_str(),
                specifier,
                &known_paths,
                &config.root_dirs,
                &config.module_suffixes,
            )
            .or_else(|| {
                resolve_scoped_path_alias_with_base_url_and_module_suffixes(
                    source_path.as_str(),
                    specifier,
                    config.base_url.as_deref(),
                    &config.explicit_paths,
                    &config.package_aliases,
                    &known_paths,
                    &config.module_suffixes,
                )
            });
            if let Some(resolved) = resolved
                && included.insert(resolved.clone())
            {
                queue.push(resolved);
            }
        }
    }
    let mut paths = included
        .into_iter()
        .filter_map(|path| universe.get(&path).cloned())
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}
