//! Load and merge TypeScript configuration inheritance chains.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::{Result, TypeTuneError};

use super::{LoadedCompilerOptions, LoadedConfig, TsConfig, jsonc::normalize_jsonc};

pub(super) fn load_extending_config(
    path: &Path,
    seen: &mut BTreeSet<PathBuf>,
    contributes_to_project_root: bool,
) -> Result<LoadedConfig> {
    let selected = seen.is_empty();
    let canonical_path = fs::canonicalize(path).map_err(|error| {
        TypeTuneError::new(format!("Could not read {}: {error}", path.display()))
    })?;
    if !seen.insert(canonical_path.clone()) {
        return Err(TypeTuneError::new(format!(
            "Circular tsconfig extends detected at {}.",
            canonical_path.display()
        )));
    }
    let content = fs::read_to_string(&canonical_path).map_err(|error| {
        TypeTuneError::new(format!(
            "Could not read {}: {error}",
            canonical_path.display()
        ))
    })?;
    let content = normalize_jsonc(&content)?;
    if selected {
        let value: serde_json::Value = serde_json::from_str(&content).map_err(|error| {
            TypeTuneError::new(format!(
                "Could not parse {}: {error}",
                canonical_path.display()
            ))
        })?;
        if let Some(references) = value.get("references") {
            let invalid = || {
                TypeTuneError::new(format!(
                    "Invalid project references in {}: expected an array of objects with non-empty string paths.",
                    canonical_path.display()
                ))
            };
            let entries = references.as_array().ok_or_else(invalid)?;
            for entry in entries {
                let path = entry
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(invalid)?;
                if path.trim().is_empty() {
                    return Err(invalid());
                }
            }
            if !entries.is_empty() {
                return Err(TypeTuneError::new(format!(
                    "TypeScript configuration {} contains unsupported project references ({}). Analysis cannot guarantee complete source and referrer coverage. Select an independently analyzable configuration without references if appropriate; this does not guarantee workspace-wide coverage.",
                    canonical_path.display(),
                    entries.len()
                )));
            }
        }
    }
    let config: TsConfig = serde_json::from_str(&content).map_err(|error| {
        TypeTuneError::new(format!(
            "Could not parse {}: {error}",
            canonical_path.display()
        ))
    })?;
    let directory = canonical_path
        .parent()
        .expect("canonical tsconfig paths have a parent")
        .to_path_buf();
    let current = LoadedConfig {
        directories: if contributes_to_project_root {
            BTreeSet::from([directory.clone()])
        } else {
            BTreeSet::new()
        },
        files: config.files.map(|values| (directory.clone(), values)),
        include: config.include.map(|values| (directory.clone(), values)),
        exclude: config.exclude.map(|values| (directory.clone(), values)),
        compiler_options: config
            .compiler_options
            .map(|options| LoadedCompilerOptions {
                module: options.module,
                base_url: options.base_url.map(|value| (directory.clone(), value)),
                out_dir: options.out_dir.map(|value| (directory.clone(), value)),
                declaration_dir: options
                    .declaration_dir
                    .map(|value| (directory.clone(), value)),
                paths: options.paths.map(|paths| (directory.clone(), paths)),
                root_dirs: options.root_dirs.map(|values| (directory.clone(), values)),
                module_suffixes: options.module_suffixes,
                custom_conditions: options.custom_conditions,
            }),
    };
    let Some(extends_paths) = config.extends_paths else {
        seen.remove(&canonical_path);
        return Ok(current);
    };
    let mut parent = None;
    for extends_path in extends_paths.into_vec() {
        let (parent_path, parent_contributes_to_project_root) =
            resolve_extends_path(&directory, &extends_path)?;
        let next = load_extending_config(
            &parent_path,
            seen,
            contributes_to_project_root && parent_contributes_to_project_root,
        )?;
        parent = Some(match parent {
            Some(parent) => merge_configs(parent, next),
            None => next,
        });
    }
    seen.remove(&canonical_path);
    Ok(match parent {
        Some(parent) => merge_configs(parent, current),
        None => current,
    })
}

fn resolve_extends_path(directory: &Path, extends_path: &str) -> Result<(PathBuf, bool)> {
    let extends_path = extends_path.replace('\\', "/");
    if extends_path.starts_with('.') || Path::new(&extends_path).is_absolute() {
        return resolve_config_file_path(directory.join(&extends_path)).map(|path| (path, true));
    }
    resolve_package_extends_path(directory, &extends_path).map(|path| (path, false))
}

fn resolve_config_file_path(mut path: PathBuf) -> Result<PathBuf> {
    if !path.is_file() && path.extension().is_none_or(|extension| extension != "json") {
        let mut candidate = path.into_os_string();
        candidate.push(".json");
        path = candidate.into();
    }
    if path.is_file() {
        Ok(path)
    } else {
        Err(TypeTuneError::new(format!(
            "Could not resolve tsconfig extends path {}.",
            path.display()
        )))
    }
}

fn resolve_package_extends_path(directory: &Path, extends_path: &str) -> Result<PathBuf> {
    let mut current = Some(directory);
    while let Some(search_directory) = current {
        let node_modules = search_directory.join("node_modules");
        let (package_name, subpath) = split_package_specifier(extends_path);
        let package_root = node_modules.join(package_name);
        if let Some(exports) = package_exports(&package_root)? {
            if let Some(path) = resolve_export(&package_root, &exports, subpath) {
                return Ok(path);
            }
            current = search_directory.parent();
            continue;
        }
        let package_path = node_modules.join(extends_path);
        if package_path.is_file() {
            return Ok(package_path);
        }
        if package_path
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            let mut json_path = package_path.as_os_str().to_os_string();
            json_path.push(".json");
            let json_path = PathBuf::from(json_path);
            if json_path.is_file() {
                return Ok(json_path);
            }
        }
        if package_path.is_dir() {
            if let Some(path) = package_tsconfig_path(&package_path)? {
                return Ok(path);
            }
            let tsconfig_path = package_path.join("tsconfig.json");
            if tsconfig_path.is_file() {
                return Ok(tsconfig_path);
            }
        }
        current = search_directory.parent();
    }
    Err(TypeTuneError::new(format!(
        "Could not resolve package tsconfig extends path {extends_path:?} from {}.",
        directory.display()
    )))
}

fn split_package_specifier(specifier: &str) -> (&str, &str) {
    let start = usize::from(specifier.starts_with('@'));
    let split = specifier[start..].find('/').map(|index| start + index);
    let split = if start == 1 {
        split.and_then(|index| {
            specifier[index + 1..]
                .find('/')
                .map(|next| index + 1 + next)
        })
    } else {
        split
    };
    split.map_or((specifier, ""), |index| {
        (&specifier[..index], &specifier[index + 1..])
    })
}

fn package_exports(package_path: &Path) -> Result<Option<serde_json::Value>> {
    let package_json = package_path.join("package.json");
    if !package_json.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&package_json).map_err(|error| {
        TypeTuneError::new(format!(
            "Could not read {}: {error}",
            package_json.display()
        ))
    })?;
    let package: serde_json::Value = serde_json::from_str(
        content.strip_prefix('\u{feff}').unwrap_or(&content),
    )
    .map_err(|error| {
        TypeTuneError::new(format!(
            "Could not parse {}: {error}",
            package_json.display()
        ))
    })?;
    Ok(package
        .get("exports")
        .filter(|value| !value.is_null())
        .cloned())
}

fn resolve_export(
    package_root: &Path,
    exports: &serde_json::Value,
    subpath: &str,
) -> Option<PathBuf> {
    let key = if subpath.is_empty() {
        ".".to_owned()
    } else {
        format!("./{subpath}")
    };
    let mut wildcard = None;
    let target = if let Some(map) = exports
        .as_object()
        .filter(|map| map.keys().any(|key| key.starts_with('.')))
    {
        if let Some(target) = map.get(&key) {
            target
        } else {
            map.iter()
                .filter_map(|(pattern, target)| {
                    let (prefix, suffix) = pattern.split_once('*')?;
                    if suffix.contains('*')
                        || !key.starts_with(prefix)
                        || !key.ends_with(suffix)
                        || key.len() < prefix.len() + suffix.len()
                    {
                        return None;
                    }
                    Some((pattern, target, prefix.len(), suffix.len()))
                })
                .max_by_key(|(_, _, prefix, suffix)| (*prefix, *suffix))
                .map(|(_, target, prefix, suffix)| {
                    wildcard = Some(&key[prefix..key.len() - suffix]);
                    target
                })?
        }
    } else if subpath.is_empty() {
        exports
    } else {
        return None;
    };
    resolve_export_target(package_root, target, wildcard)
}

fn resolve_export_target(
    package_root: &Path,
    target: &serde_json::Value,
    wildcard: Option<&str>,
) -> Option<PathBuf> {
    match target {
        serde_json::Value::String(target) => {
            let target = target.replace('*', wildcard.unwrap_or("*"));
            let relative = target.strip_prefix("./")?;
            if relative
                .split('/')
                .any(|part| matches!(part, ".." | "." | "node_modules"))
            {
                return None;
            }
            let mut path = package_root.join(relative);
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("json") => {}
                Some("js") => {
                    path.set_extension("json");
                }
                _ => return None,
            }
            path.is_file().then_some(path)
        }
        serde_json::Value::Array(targets) => targets
            .iter()
            .find_map(|target| resolve_export_target(package_root, target, wildcard)),
        serde_json::Value::Object(conditions) => conditions
            .iter()
            .filter(|(condition, _)| {
                matches!(condition.as_str(), "types" | "node" | "require" | "default")
            })
            .find_map(|(_, target)| resolve_export_target(package_root, target, wildcard)),
        _ => None,
    }
}

fn package_tsconfig_path(package_path: &Path) -> Result<Option<PathBuf>> {
    let package_json = package_path.join("package.json");
    if !package_json.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&package_json).map_err(|error| {
        TypeTuneError::new(format!(
            "Could not read {}: {error}",
            package_json.display()
        ))
    })?;
    let package: serde_json::Value = serde_json::from_str(
        content.strip_prefix('\u{feff}').unwrap_or(&content),
    )
    .map_err(|error| {
        TypeTuneError::new(format!(
            "Could not parse {}: {error}",
            package_json.display()
        ))
    })?;
    let Some(path) = package.get("tsconfig").and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    resolve_config_file_path(package_path.join(path)).map(Some)
}

fn merge_configs(parent: LoadedConfig, child: LoadedConfig) -> LoadedConfig {
    let mut directories = parent.directories;
    directories.extend(child.directories);
    LoadedConfig {
        directories,
        files: child.files.or(parent.files),
        include: child.include.or(parent.include),
        exclude: child.exclude.or(parent.exclude),
        compiler_options: match (parent.compiler_options, child.compiler_options) {
            (Some(parent), Some(child)) => Some(LoadedCompilerOptions {
                module: child.module.or(parent.module),
                base_url: child.base_url.or(parent.base_url),
                out_dir: child.out_dir.or(parent.out_dir),
                declaration_dir: child.declaration_dir.or(parent.declaration_dir),
                paths: child.paths.or(parent.paths),
                root_dirs: child.root_dirs.or(parent.root_dirs),
                module_suffixes: child.module_suffixes.or(parent.module_suffixes),
                custom_conditions: child.custom_conditions.or(parent.custom_conditions),
            }),
            (Some(parent), None) => Some(parent),
            (None, Some(child)) => Some(child),
            (None, None) => None,
        },
    }
}
