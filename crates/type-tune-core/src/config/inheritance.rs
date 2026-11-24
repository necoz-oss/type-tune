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
            return match resolve_export(&package_root, &exports, subpath) {
                ExportResolution::Resolved(path) => Ok(path),
                ExportResolution::Rejected(reason) => Err(TypeTuneError::new(format!(
                    "Could not resolve package tsconfig extends path {extends_path:?}: {reason} in {}.",
                    package_root.display()
                ))),
                ExportResolution::Missing => Err(TypeTuneError::new(format!(
                    "Export target for package tsconfig extends path {extends_path:?} was not found in {}.",
                    package_root.display()
                ))),
                ExportResolution::NoMatch => Err(TypeTuneError::new(format!(
                    "Package {} does not export tsconfig subpath {subpath:?}.",
                    package_root.display()
                ))),
            };
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
    Ok(package.get("exports").cloned())
}

#[derive(Debug)]
enum ExportResolution {
    Resolved(PathBuf),
    NoMatch,
    Missing,
    Rejected(String),
}

fn resolve_export(
    package_root: &Path,
    exports: &serde_json::Value,
    subpath: &str,
) -> ExportResolution {
    if !subpath.is_empty() && !safe_export_components(subpath) {
        return ExportResolution::Rejected("invalid package subpath".into());
    }
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
        if map.keys().any(|key| !key.starts_with('.')) {
            return ExportResolution::Rejected("mixed export subpaths and conditions".into());
        }
        if let Some(target) = map.get(&key) {
            target
        } else {
            let found = map
                .iter()
                .filter_map(|(pattern, target)| {
                    let (prefix, suffix) = pattern.split_once('*')?;
                    if suffix.contains('*')
                        || !key.starts_with(prefix)
                        || !key.ends_with(suffix)
                        || key.len() < prefix.len() + suffix.len()
                    {
                        return None;
                    }
                    Some((target, prefix.len(), suffix.len()))
                })
                .max_by_key(|(_, prefix, suffix)| (*prefix, *suffix));
            let Some((target, prefix, suffix)) = found else {
                return ExportResolution::NoMatch;
            };
            wildcard = Some(&key[prefix..key.len() - suffix]);
            target
        }
    } else if subpath.is_empty() {
        exports
    } else {
        return ExportResolution::NoMatch;
    };
    resolve_export_target(package_root, target, wildcard)
}

// Package export targets use URL-style forward slashes on every platform. Reject
// ambiguous encodings and Windows path syntax before passing anything to Path.
fn safe_export_components(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '%', '\0'])
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.eq_ignore_ascii_case("node_modules")
        })
}

fn resolve_export_target(
    package_root: &Path,
    target: &serde_json::Value,
    wildcard: Option<&str>,
) -> ExportResolution {
    match target {
        serde_json::Value::String(target) => {
            let target = target.replace('*', wildcard.unwrap_or("*"));
            let Some(relative) = target.strip_prefix("./") else {
                return ExportResolution::Rejected(format!("invalid export target {target:?}"));
            };
            if !safe_export_components(relative) || relative.contains('*') {
                return ExportResolution::Rejected(format!("invalid export target {target:?}"));
            }
            let mut path = package_root.join(relative);
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("json") => {}
                Some("js") => {
                    path.set_extension("json");
                }
                _ => {
                    return ExportResolution::Rejected(format!(
                        "unsupported export target {target:?}"
                    ));
                }
            }
            if !path.is_file() {
                return ExportResolution::Missing;
            }
            let Ok(root) = fs::canonicalize(package_root) else {
                return ExportResolution::Rejected("cannot canonicalize package root".into());
            };
            let Ok(resolved) = fs::canonicalize(&path) else {
                return ExportResolution::Rejected(format!(
                    "cannot canonicalize export target {target:?}"
                ));
            };
            if !resolved.starts_with(&root) {
                return ExportResolution::Rejected(format!(
                    "export target {target:?} escapes the package root"
                ));
            }
            ExportResolution::Resolved(resolved)
        }
        serde_json::Value::Array(targets) => {
            let mut missing = false;
            for target in targets {
                match resolve_export_target(package_root, target, wildcard) {
                    ExportResolution::NoMatch => continue,
                    ExportResolution::Missing => missing = true,
                    result => return result,
                }
            }
            if missing {
                ExportResolution::Missing
            } else {
                ExportResolution::NoMatch
            }
        }
        serde_json::Value::Object(conditions) => {
            let mut missing = false;
            for (condition, target) in conditions {
                if condition.starts_with("types@") {
                    return ExportResolution::Rejected(format!(
                        "unsupported versioned export condition {condition:?}"
                    ));
                }
                if matches!(condition.as_str(), "types" | "node" | "require" | "default") {
                    match resolve_export_target(package_root, target, wildcard) {
                        ExportResolution::NoMatch => continue,
                        ExportResolution::Missing => missing = true,
                        result => return result,
                    }
                }
            }
            if missing {
                ExportResolution::Missing
            } else {
                ExportResolution::NoMatch
            }
        }
        serde_json::Value::Null => ExportResolution::Rejected("export target is null".into()),
        _ => ExportResolution::Rejected("invalid export target".into()),
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

#[cfg(test)]
mod export_tests {
    use super::*;
    use serde_json::json;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn package_exports_shadow_ancestors_and_preserve_legacy_lookup() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        let local = app.join("node_modules/my-config");
        let parent = root.path().join("node_modules/my-config");
        write(
            &parent.join("package.json"),
            r#"{"exports":{"./private":"./private.json"}}"#,
        );
        write(&parent.join("private.json"), "{}");
        fs::create_dir_all(&app).unwrap();
        assert_eq!(
            resolve_package_extends_path(&app, "my-config/private").unwrap(),
            parent.join("private.json").canonicalize().unwrap()
        );

        write(
            &local.join("package.json"),
            r#"{"exports":{"./public":"./public.json","./null":null,"./missing":"./missing.json"}}"#,
        );
        write(&local.join("public.json"), "{}");
        assert_eq!(
            resolve_package_extends_path(&app, "my-config/public").unwrap(),
            local.join("public.json").canonicalize().unwrap()
        );
        for (subpath, message) in [
            ("private", "does not export"),
            ("null", "null"),
            ("missing", "not found"),
        ] {
            let error = resolve_package_extends_path(&app, &format!("my-config/{subpath}"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{error}");
        }
        write(&local.join("package.json"), r#"{"exports":null}"#);
        assert!(resolve_package_extends_path(&app, "my-config/private").is_err());
        write(&local.join("package.json"), "{}");
        write(&local.join("private.json"), "{}");
        assert_eq!(
            resolve_package_extends_path(&app, "my-config/private").unwrap(),
            local.join("private.json")
        );
    }

    #[test]
    fn export_target_validation_rejects_cross_platform_escapes() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("pkg");
        write(&package.join("nested/空 白/config.json"), "{}");
        write(&root.path().join("outside.json"), "not a configuration");
        let valid = json!("./nested/空 白/config.json");
        assert!(matches!(
            resolve_export_target(&package, &valid, None),
            ExportResolution::Resolved(_)
        ));
        for target in [
            "./../outside.json",
            "./..\\\\outside.json",
            "./nested/..\\\\..\\\\outside.json",
            "/outside.json",
            "C:\\\\outside.json",
            "./C:/outside.json",
            "\\\\\\\\server\\\\share\\\\outside.json",
            "\\\\\\\\?\\\\C:\\\\outside.json",
            "./node_modules/config.json",
            "./nested/./config.json",
            "./%2e%2e/outside.json",
            "./.. /outside.json",
            "./.../outside.json",
        ] {
            assert!(
                matches!(
                    resolve_export_target(&package, &json!(target), None),
                    ExportResolution::Rejected(_)
                ),
                "accepted {target:?}"
            );
        }
        for unsafe_subpath in [
            "../outside",
            "..\\\\outside",
            "nested/../../outside",
            "C:/outside",
            "node_modules/outside",
        ] {
            let exports = json!({"./*":"./nested/*.json"});
            assert!(
                matches!(
                    resolve_export(&package, &exports, unsafe_subpath),
                    ExportResolution::Rejected(_)
                ),
                "accepted {unsafe_subpath:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn export_target_accepts_canonical_windows_package_root() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package with spaces/設定");
        write(&package.join("nested/base.json"), "{}");
        let canonical_package = package.canonicalize().unwrap();
        let ExportResolution::Resolved(path) =
            resolve_export_target(&canonical_package, &json!("./nested/base.json"), None)
        else {
            panic!("canonical Windows package root must resolve");
        };
        assert_eq!(
            path,
            package.join("nested/base.json").canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_target_rejects_symlink_outside_package() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("pkg");
        write(&root.path().join("outside.json"), "{}");
        fs::create_dir_all(&package).unwrap();
        symlink(root.path().join("outside.json"), package.join("link.json")).unwrap();
        assert!(matches!(
            resolve_export_target(&package, &json!("./link.json"), None),
            ExportResolution::Rejected(_)
        ));
    }

    #[test]
    fn export_conditions_arrays_and_pattern_precedence() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path();
        for name in ["exact", "specific", "fallback", "types", "default"] {
            write(&package.join(format!("{name}.json")), "{}");
        }
        let exports = json!({
            "./features/exact": "./exact.json",
            "./features/*": "./specific.json",
            "./*": "./default.json",
            "./conditional": {"import":"./default.json", "require": {"types":"./types.json"}, "default":"./default.json"},
            "./array": ["./missing.json", "./fallback.json"],
            "./blocked": [null, "./fallback.json"]
        });
        for (key, expected) in [
            ("features/exact", "exact"),
            ("features/other", "specific"),
            ("conditional", "types"),
            ("array", "fallback"),
        ] {
            let ExportResolution::Resolved(path) = resolve_export(package, &exports, key) else {
                panic!("did not resolve {key}")
            };
            assert_eq!(
                path,
                package
                    .join(format!("{expected}.json"))
                    .canonicalize()
                    .unwrap()
            );
        }
        assert!(matches!(
            resolve_export(package, &exports, "blocked"),
            ExportResolution::Rejected(_)
        ));
    }
    #[test]
    #[ignore = "requires the repository's installed TypeScript compilers and Node.js"]
    fn typescript_package_exports_matrix() {
        use std::process::Command;
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let compilers = [
            (
                "4.9.5",
                "tests/compiler-versions/node_modules/typescript49/bin/tsc",
            ),
            (
                "5.5.4",
                "tests/compiler-versions/node_modules/typescript55/bin/tsc",
            ),
            (
                "5.8.3",
                "tests/compiler-versions/node_modules/typescript58/bin/tsc",
            ),
            ("6.0.3", "node_modules/typescript/bin/tsc"),
        ];
        let cases = [
            "scoped",
            "exact",
            "wildcard",
            "conditional",
            "array",
            "null",
            "pattern_priority",
            "exact_priority",
            "unexported",
            "shadow",
            "missing",
            "unsafe",
        ];
        for case in cases {
            let root = tempfile::tempdir().unwrap();
            let app = root.path().join("app");
            let name = if case == "scoped" {
                "@scope/config"
            } else {
                "my-config"
            };
            let package = app.join("node_modules").join(name);
            let subpath = if matches!(case, "wildcard" | "pattern_priority" | "exact_priority") {
                "configs/chosen"
            } else if case == "shadow" || case == "unexported" {
                "private"
            } else {
                "chosen"
            };
            let exports = match case {
                "wildcard" => json!({"./configs/*":"./mapped/*.json"}),
                "array" => json!({"./chosen":["./missing.json", "./mapped/chosen.json"]}),
                "null" => json!({"./chosen":[null, "./mapped/chosen.json"]}),
                "pattern_priority" => {
                    json!({"./*":"./wrong.json", "./configs/*":"./mapped/*.json"})
                }
                "exact_priority" => {
                    json!({"./configs/*":"./wrong.json", "./configs/chosen":"./mapped/chosen.json"})
                }
                "conditional" => {
                    json!({"./chosen":{"import":"./wrong.json","require":{"types":"./mapped/chosen.json"},"default":"./wrong.json"}})
                }
                "unexported" | "shadow" => json!({"./public":"./chosen.json"}),
                "missing" => json!({"./chosen":"./missing.json"}),
                "unsafe" => json!({"./chosen":"./../outside.json"}),
                _ => json!({"./chosen":"./mapped/chosen.json"}),
            };
            write(
                &package.join("package.json"),
                &json!({"exports":exports}).to_string(),
            );
            for (file, marker) in [
                ("mapped/chosen.json", ".chosen"),
                ("wrong.json", ".wrong"),
                ("private.json", ".local"),
            ] {
                if case == "shadow" && file == "private.json" {
                    continue;
                }
                write(
                    &package.join(file),
                    &json!({"compilerOptions":{"moduleSuffixes":[marker]}}).to_string(),
                );
            }
            if case == "shadow" {
                let parent = root.path().join("node_modules/my-config");
                write(
                    &parent.join("package.json"),
                    &json!({"exports":{"./private":"./private.json"}}).to_string(),
                );
                write(
                    &parent.join("private.json"),
                    &json!({"compilerOptions":{"moduleSuffixes":[".ancestor"]}}).to_string(),
                );
            }
            write(&app.join("index.ts"), "export {};");
            let project = app.join("tsconfig.json");
            write(
                &project,
                &json!({"extends":format!("{name}/{subpath}"),"files":["index.ts"]}).to_string(),
            );
            let native = load_extending_config(&project, &mut BTreeSet::new(), true)
                .ok()
                .map(|config| {
                    config
                        .compiler_options
                        .expect("inherited options")
                        .module_suffixes
                        .expect("inherited marker")
                        .into_iter()
                        .next()
                        .expect("marker value")
                });
            for (version, compiler) in compilers {
                let output = Command::new("node")
                    .arg(repo.join(compiler))
                    .args(["--showConfig", "-p"])
                    .arg(&project)
                    .output()
                    .expect("runs TypeScript");
                let actual = if output.status.success() {
                    let config: serde_json::Value =
                        serde_json::from_slice(&output.stdout).expect("compiler JSON");
                    Some(
                        config["compilerOptions"]["moduleSuffixes"][0]
                            .as_str()
                            .expect("compiler inherited marker")
                            .to_owned(),
                    )
                } else {
                    None
                };
                println!("{case}: TypeScript {version}={actual:?}; type-tune={native:?}");
                let expected_compiler = match (case, version) {
                    ("shadow", _) => Some(".ancestor"),
                    ("unexported", "4.9.5") => Some(".local"),
                    ("null", "5.5.4" | "5.8.3") => Some(".chosen"),
                    (
                        "scoped" | "exact" | "wildcard" | "conditional" | "array"
                        | "pattern_priority" | "exact_priority",
                        "5.5.4" | "5.8.3" | "6.0.3",
                    ) => Some(".chosen"),
                    _ => None,
                };
                assert_eq!(
                    actual.as_deref(),
                    expected_compiler,
                    "{case} on TypeScript {version}"
                );
                let expected_native = if matches!(
                    case,
                    "scoped"
                        | "exact"
                        | "wildcard"
                        | "conditional"
                        | "array"
                        | "pattern_priority"
                        | "exact_priority"
                ) {
                    Some(".chosen")
                } else {
                    None
                };
                assert_eq!(native.as_deref(), expected_native, "native {case}");
            }
        }
    }
}
