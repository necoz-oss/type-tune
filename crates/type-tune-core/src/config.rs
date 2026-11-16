//! Native project configuration loading.

mod inheritance;
mod jsonc;
mod package_aliases;

use inheritance::load_extending_config;
use package_aliases::{
    load_package_module_aliases, normalize_package_module_aliases, package_scope_for_config,
    package_scope_key,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{Result, TypeTuneError};

#[derive(Debug, Clone, Deserialize)]
struct TsConfig {
    #[serde(rename = "extends")]
    extends_paths: Option<ExtendsPaths>,
    files: Option<Vec<String>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    #[serde(rename = "compilerOptions")]
    compiler_options: Option<CompilerOptions>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ExtendsPaths {
    One(String),
    Many(Vec<String>),
}

impl ExtendsPaths {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(path) => vec![path],
            Self::Many(paths) => paths,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CompilerOptions {
    module: Option<String>,
    #[serde(rename = "baseUrl")]
    base_url: Option<String>,
    #[serde(rename = "outDir")]
    out_dir: Option<String>,
    #[serde(rename = "declarationDir")]
    declaration_dir: Option<String>,
    #[serde(rename = "rootDirs")]
    root_dirs: Option<Vec<String>>,
    #[serde(rename = "moduleSuffixes")]
    module_suffixes: Option<Vec<String>>,
    #[serde(rename = "customConditions")]
    custom_conditions: Option<Vec<String>>,
    paths: Option<BTreeMap<String, Vec<String>>>,
}

/// Relevant project settings loaded from `tsconfig.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Common root containing project tsconfig directories in the extends chain.
    pub project_root: PathBuf,
    /// Explicit project-relative source files.
    ///
    /// `None` means `files` was omitted. `Some` empty list is explicit and does not fall back to
    /// the default include globs.
    pub files: Option<Vec<String>>,
    /// Include patterns declared by the TypeScript project.
    ///
    /// `None` means `include` was omitted. Default globs apply only when `files` is also omitted.
    pub include: Option<Vec<String>>,
    /// Exclude patterns declared by the TypeScript project.
    pub exclude: Vec<String>,
    /// Optional TypeScript base URL used for path aliases.
    pub base_url: Option<String>,
    /// Module alias entries in deterministic key order, including package exports and imports.
    pub paths: BTreeMap<String, Vec<String>>,
    /// Module aliases declared directly by TypeScript `paths` settings.
    pub explicit_paths: BTreeMap<String, Vec<String>>,
    /// Package module aliases grouped by their project-relative package scope.
    pub package_aliases: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// Whether the effective tsconfig declares `paths` and can preserve alias output.
    pub preserve_path_aliases: bool,
    /// TypeScript root directories used to resolve relative module specifiers.
    pub root_dirs: Vec<String>,
    /// TypeScript module suffixes applied before source extension resolution.
    pub module_suffixes: Vec<String>,
    /// TypeScript package export conditions selected in addition to standard conditions.
    pub custom_conditions: Vec<String>,
    /// Whether NodeNext-style CommonJS package resolution selects the `require` condition.
    pub package_require_condition: bool,
    /// Whether Node16, Node18, Node20, or NodeNext package resolution selects `node`.
    pub package_node_condition: bool,
}

/// Adds package module aliases for the package scopes containing source paths.
///
/// Existing TypeScript `paths` entries take precedence over package aliases. Source paths outside
/// the project root are ignored because native rewrite inputs must remain project-relative.
///
/// # Errors
///
/// Returns an error when package metadata cannot be read or parsed.
pub fn add_package_module_aliases_for_sources(
    config: &mut ProjectConfig,
    source_paths: &[PathBuf],
) -> Result<()> {
    let scopes = source_paths
        .iter()
        .filter_map(|path| path.parent())
        .filter(|directory| directory.starts_with(&config.project_root))
        .map(|directory| package_scope_for_config(directory, &config.project_root))
        .collect::<BTreeSet<_>>();
    for scope in scopes {
        let aliases = load_package_module_aliases(
            &scope,
            &config.custom_conditions,
            config.package_require_condition,
            config.package_node_condition,
        )?;
        let aliases = normalize_package_module_aliases(
            aliases,
            &scope,
            &config.project_root,
            config.base_url.as_deref(),
        );
        let scope_key = package_scope_key(&scope, &config.project_root);
        config.package_aliases.insert(scope_key, aliases.clone());
        for (specifier, targets) in aliases {
            config.paths.entry(specifier).or_insert(targets);
        }
    }
    Ok(())
}

/// Loads the subset of `tsconfig.json` consumed by native scanning.
///
/// # Errors
///
/// Returns an error when the configuration cannot be read or parsed.
pub fn load_project_config(path: &Path) -> Result<ProjectConfig> {
    let tsconfig_path = path.canonicalize().map_err(|error| {
        TypeTuneError::new(format!("Could not resolve {}: {error}", path.display()))
    })?;
    let config = load_extending_config(&tsconfig_path, &mut BTreeSet::new(), true)?;
    let config_directory = tsconfig_path.parent().ok_or_else(|| {
        TypeTuneError::new(format!(
            "Could not determine tsconfig directory for {}.",
            tsconfig_path.display()
        ))
    })?;
    let project_root = project_root_for_config(config_directory, &config)?;
    let package_scope = package_scope_for_config(config_directory, &project_root);
    let compiler_options = config.compiler_options.clone().unwrap_or_default();
    let exclude = project_exclude_patterns(&config, &compiler_options, &project_root)?;
    let preserve_path_aliases = compiler_options.paths.is_some();
    let base_url = compiler_options
        .base_url
        .map(|(directory, value)| resolve_pattern(&directory, &value, &project_root))
        .map(|value| value.map(|value| if value.is_empty() { ".".into() } else { value }))
        .transpose()?;
    let mut paths = compiler_options
        .paths
        .map(|(directory, paths)| {
            if base_url.is_some() {
                Ok(paths)
            } else {
                resolve_path_alias_targets(&directory, paths, &project_root)
            }
        })
        .transpose()?
        .unwrap_or_default();
    let explicit_paths = paths.clone();
    let custom_conditions = compiler_options.custom_conditions.unwrap_or_default();
    let package_node_condition = compiler_options.module.as_deref().is_some_and(|module| {
        ["node16", "node18", "node20", "nodenext"]
            .iter()
            .any(|node_module| module.eq_ignore_ascii_case(node_module))
    });
    let package_require_condition = package_node_condition;
    let package_module_aliases = load_package_module_aliases(
        &package_scope,
        &custom_conditions,
        package_require_condition,
        package_node_condition,
    )?;
    let package_module_aliases = normalize_package_module_aliases(
        package_module_aliases,
        &package_scope,
        &project_root,
        base_url.as_deref(),
    );
    for (specifier, targets) in &package_module_aliases {
        paths
            .entry(specifier.clone())
            .or_insert_with(|| targets.clone());
    }
    let package_aliases = BTreeMap::from([(
        package_scope_key(&package_scope, &project_root),
        package_module_aliases,
    )]);
    Ok(ProjectConfig {
        project_root: project_root.clone(),
        files: config
            .files
            .map(|patterns| resolve_patterns(patterns, &project_root))
            .transpose()?,
        include: config
            .include
            .map(|patterns| resolve_patterns(patterns, &project_root))
            .transpose()?,
        exclude,
        base_url,
        paths,
        explicit_paths,
        package_aliases,
        preserve_path_aliases,
        root_dirs: compiler_options
            .root_dirs
            .map(|values| resolve_patterns(values, &project_root))
            .transpose()?
            .unwrap_or_default(),
        module_suffixes: compiler_options.module_suffixes.unwrap_or_default(),
        custom_conditions,
        package_require_condition,
        package_node_condition,
    })
}

fn project_root_for_config(config_directory: &Path, config: &LoadedConfig) -> Result<PathBuf> {
    let mut directories = BTreeSet::from([config_directory.to_path_buf()]);
    for setting in [&config.files, &config.include].into_iter().flatten() {
        directories.insert(setting.0.clone());
    }
    common_ancestor(&directories)
}

#[derive(Debug, Clone)]
struct LoadedConfig {
    directories: BTreeSet<PathBuf>,
    files: Option<(PathBuf, Vec<String>)>,
    include: Option<(PathBuf, Vec<String>)>,
    exclude: Option<(PathBuf, Vec<String>)>,
    compiler_options: Option<LoadedCompilerOptions>,
}

#[derive(Debug, Clone, Default)]
struct LoadedCompilerOptions {
    module: Option<String>,
    base_url: Option<(PathBuf, String)>,
    out_dir: Option<(PathBuf, String)>,
    declaration_dir: Option<(PathBuf, String)>,
    paths: Option<(PathBuf, BTreeMap<String, Vec<String>>)>,
    root_dirs: Option<(PathBuf, Vec<String>)>,
    module_suffixes: Option<Vec<String>>,
    custom_conditions: Option<Vec<String>>,
}

fn project_exclude_patterns(
    config: &LoadedConfig,
    compiler_options: &LoadedCompilerOptions,
    project_root: &Path,
) -> Result<Vec<String>> {
    if config.exclude.is_some() {
        return resolve_optional_patterns(config.exclude.clone(), project_root);
    }
    let mut exclude = Vec::new();
    for (directory, output) in [&compiler_options.out_dir, &compiler_options.declaration_dir]
        .into_iter()
        .flatten()
    {
        if let Ok(pattern) = resolve_pattern(directory, output, project_root)
            && !pattern.is_empty()
            && !exclude.contains(&pattern)
        {
            exclude.push(pattern);
        }
    }
    Ok(exclude)
}

fn resolve_optional_patterns(
    patterns: Option<(PathBuf, Vec<String>)>,
    project_root: &Path,
) -> Result<Vec<String>> {
    patterns
        .map(|patterns| resolve_patterns(patterns, project_root))
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn resolve_patterns(
    (directory, values): (PathBuf, Vec<String>),
    project_root: &Path,
) -> Result<Vec<String>> {
    values
        .iter()
        .map(|value| resolve_pattern(&directory, value, project_root))
        .collect()
}

fn resolve_path_alias_targets(
    directory: &Path,
    paths: BTreeMap<String, Vec<String>>,
    project_root: &Path,
) -> Result<BTreeMap<String, Vec<String>>> {
    paths
        .into_iter()
        .map(|(alias, targets)| {
            resolve_patterns((directory.to_path_buf(), targets), project_root)
                .map(|targets| (alias, targets))
        })
        .collect()
}

fn resolve_pattern(directory: &Path, value: &str, project_root: &Path) -> Result<String> {
    let resolved = normalize_relative_path(directory, value)?;
    let relative = resolved.strip_prefix(project_root).map_err(|_| {
        TypeTuneError::new(format!(
            "tsconfig path {value:?} resolves outside native project root {}.",
            project_root.display()
        ))
    })?;
    let value = relative.to_string_lossy().replace('\\', "/");
    Ok(value.trim_start_matches("./").to_owned())
}

fn normalize_relative_path(directory: &Path, value: &str) -> Result<PathBuf> {
    let value_path = Path::new(value);
    if value.is_empty() || value_path.is_absolute() {
        return Err(TypeTuneError::new(format!(
            "Expected a non-empty relative tsconfig path, received {value:?}."
        )));
    }
    let mut output = directory.to_path_buf();
    for component in value_path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            std::path::Component::Normal(part) => output.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => unreachable!(),
        }
    }
    Ok(output)
}

fn common_ancestor(paths: &BTreeSet<PathBuf>) -> Result<PathBuf> {
    let mut paths = paths.iter();
    let Some(first) = paths.next() else {
        return Err(TypeTuneError::new(
            "Expected at least one tsconfig source directory.",
        ));
    };
    let mut ancestor = first.clone();
    for path in paths {
        while !path.starts_with(&ancestor) {
            if !ancestor.pop() {
                return Err(TypeTuneError::new(
                    "Could not determine native project root.",
                ));
            }
        }
    }
    Ok(ancestor)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{load_package_module_aliases, load_project_config};

    #[test]
    fn selected_project_references_are_checked_without_inheriting_or_traversing_them() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tsconfig.json");
        for references in [
            r#"[{"path":"./missing"}]"#,
            r#"[{"path":"../outside"},{"path":"./tsconfig.json"}]"#,
        ] {
            std::fs::write(
                &path,
                format!(r#"{{"files":[],"references":{references}}}"#),
            )
            .unwrap();
            let error = load_project_config(&path).unwrap_err().to_string();
            assert!(error.contains("project references"), "{error}");
        }
        for references in [
            "null",
            "{}",
            "[1]",
            "[{}]",
            r#"[{"path":""}]"#,
            r#"[{"path":42}]"#,
        ] {
            std::fs::write(&path, format!(r#"{{"references":{references}}}"#)).unwrap();
            let error = load_project_config(&path).unwrap_err().to_string();
            assert!(error.contains("Invalid project references"), "{error}");
        }
        std::fs::write(
            root.path().join("base.json"),
            r#"{"references":[{"path":"./missing"}]}"#,
        )
        .unwrap();
        for content in [
            r#"{"extends":"./base.json"}"#,
            r#"{"extends":"./base.json","references":[]}"#,
            r#"{"references":[]}"#,
            "{}",
        ] {
            std::fs::write(&path, content).unwrap();
            load_project_config(&path).expect("reference-free selected config");
        }
    }

    #[test]
    fn uses_explicit_include_patterns() {
        let path = std::env::temp_dir().join(format!(
            "type-tune-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::write(&path, r#"{"include":["src/**/*.ts"],"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]},"rootDirs":["src","virtual"],"moduleSuffixes":[".native",""]}}"#).expect("writes fixture");
        let config = load_project_config(&path).expect("loads");
        assert_eq!(config.project_root, path.parent().expect("has parent"));
        assert!(config.files.is_none());
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
        assert_eq!(config.base_url.as_deref(), Some("."));
        assert_eq!(config.paths.get("@/*"), Some(&vec!["src/*".into()]));
        assert_eq!(
            config.root_dirs,
            vec![String::from("src"), String::from("virtual")]
        );
        assert_eq!(
            config.module_suffixes,
            vec![String::from(".native"), String::new()]
        );
        fs::remove_file(path).expect("removes fixture");
    }

    #[test]
    fn loads_package_module_aliases() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-package-self-reference-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(
            root.join("package.json"),
            r##"{"name":"fixture-package","exports":{"./shared":{"types":"./src/shared.ts"},"./hidden":{"types":null,"default":"./src/hidden.ts"}},"imports":{"#model":"./src/model.ts"}}"##,
        )
        .expect("writes package metadata");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"baseUrl":"src"}}"#,
        )
        .expect("writes config");

        let package_references =
            load_package_module_aliases(&root, &[], false, false).expect("loads package aliases");
        assert!(
            package_references.contains_key("fixture-package/shared"),
            "unexpected references: {package_references:?}"
        );
        assert_eq!(
            package_references.get("#model"),
            Some(&vec!["src/model.ts".into()])
        );
        assert!(!package_references.contains_key("fixture-package/hidden"));
        let config = load_project_config(&root.join("tsconfig.json")).expect("loads config");
        assert_eq!(
            config.paths.get("fixture-package/shared"),
            Some(&vec!["shared.ts".into()])
        );
        assert_eq!(config.paths.get("#model"), Some(&vec!["model.ts".into()]));
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn selects_package_import_conditions_by_module_kind() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-package-import-conditions-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(
            root.join("package.json"),
            r##"{"imports":{"#shared":{"import":"./src/import.ts","require":"./src/require.ts","default":"./src/default.ts"},"#node-shared":{"node":"./src/node.ts","default":"./src/default.ts"}}}"##,
        )
        .expect("writes package metadata");

        let common_js = load_package_module_aliases(&root, &[], true, true)
            .expect("loads CommonJS package aliases");
        assert_eq!(
            common_js.get("#shared"),
            Some(&vec!["src/require.ts".into()])
        );
        assert_eq!(
            common_js.get("#node-shared"),
            Some(&vec!["src/node.ts".into()])
        );
        let esm = load_package_module_aliases(&root, &[], false, true)
            .expect("loads ESM package aliases");
        assert_eq!(esm.get("#shared"), Some(&vec!["src/import.ts".into()]));
        assert_eq!(esm.get("#node-shared"), Some(&vec!["src/node.ts".into()]));
        let non_node =
            load_package_module_aliases(&root, &[], false, false).expect("loads bundler aliases");
        assert_eq!(
            non_node.get("#node-shared"),
            Some(&vec!["src/default.ts".into()])
        );

        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn enables_node_package_conditions_for_supported_node_modules() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-node-module-conditions-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");

        for module in ["node16", "node18", "node20", "nodenext"] {
            fs::write(
                root.join("tsconfig.json"),
                format!(r#"{{"compilerOptions":{{"module":"{module}"}}}}"#),
            )
            .expect("writes config");
            let config = load_project_config(&root.join("tsconfig.json")).expect("loads config");
            assert!(
                config.package_node_condition,
                "expected {module} to enable node"
            );
        }

        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn inherits_same_directory_relative_extends() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(
            root.join("base.json"),
            r#"{"include":["src/**/*.ts"],"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
        )
        .expect("writes base config");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"extends":"./base","compilerOptions":{"baseUrl":"src"}}"#,
        )
        .expect("writes config");
        let config = load_project_config(&root.join("tsconfig.json")).expect("loads config");
        assert_eq!(config.project_root, root);
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
        assert_eq!(config.base_url.as_deref(), Some("src"));
        assert_eq!(config.paths.get("@/*"), Some(&vec!["src/*".into()]));
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn resolves_cross_directory_extends_against_each_config_directory() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("configs")).expect("creates fixture");
        fs::write(
            root.join("base.json"),
            r#"{"include":["shared/**/*.ts"],"compilerOptions":{"baseUrl":".","paths":{"@shared/*":["shared/*"]}}}"#,
        )
        .expect("writes base config");
        fs::write(
            root.join("configs/tsconfig.json"),
            r#"{"extends":"../base"}"#,
        )
        .expect("writes config");
        let config = load_project_config(&root.join("configs/tsconfig.json"))
            .expect("loads cross-directory config");
        assert_eq!(config.project_root, root);
        assert_eq!(config.include, Some(vec!["shared/**/*.ts".into()]));
        assert_eq!(config.base_url.as_deref(), Some("."));
        assert_eq!(
            config.paths.get("@shared/*"),
            Some(&vec!["shared/*".into()])
        );
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn resolves_inherited_paths_without_base_url_against_their_config_directory() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("configs")).expect("creates fixture");
        fs::write(
            root.join("configs/base.json"),
            r#"{"compilerOptions":{"paths":{"@/*":["./mapped/*"]}}}"#,
        )
        .expect("writes base config");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"extends":"./configs/base","include":["src/**/*.ts"]}"#,
        )
        .expect("writes config");

        let config = load_project_config(&root.join("tsconfig.json")).expect("loads config");
        assert!(config.base_url.is_none());
        assert_eq!(
            config.paths.get("@/*"),
            Some(&vec!["configs/mapped/*".into()])
        );
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn resolves_package_tsconfig_extends_without_widening_the_project_root() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-package-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let package = root.join("node_modules/@scope/config");
        fs::create_dir_all(&package).expect("creates package config");
        fs::write(
            package.join("package.json"),
            r#"{"name":"@scope/config","tsconfig":"base.json"}"#,
        )
        .expect("writes package metadata");
        fs::write(package.join("base.json"), r#"{"compilerOptions":{}}"#)
            .expect("writes package config");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"extends":"@scope/config","include":["src/**/*.ts"]}"#,
        )
        .expect("writes project config");

        let config =
            load_project_config(&root.join("tsconfig.json")).expect("loads package config");
        assert_eq!(config.project_root, root);
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn merges_tsconfig_extends_arrays_in_declaration_order() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-config-array-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(
            root.join("first.json"),
            r#"{"include":["first/**/*.ts"],"compilerOptions":{"baseUrl":"first","paths":{"@first/*":["*"]}}}"#,
        )
        .expect("writes first config");
        fs::write(
            root.join("second.json"),
            r#"{"include":["second/**/*.ts"],"compilerOptions":{"baseUrl":"second"}}"#,
        )
        .expect("writes second config");
        fs::write(
            root.join("tsconfig.json"),
            r#"{"extends":["./first","./second"]}"#,
        )
        .expect("writes project config");

        let config = load_project_config(&root.join("tsconfig.json")).expect("loads extends array");
        assert_eq!(config.include, Some(vec!["second/**/*.ts".into()]));
        assert_eq!(config.base_url.as_deref(), Some("second"));
        assert_eq!(config.paths.get("@first/*"), Some(&vec!["*".into()]));
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn allows_shared_extends_bases_but_rejects_cycles() {
        let root = tempfile::tempdir().expect("creates project");
        let write = |name: &str, content: &str| {
            fs::write(root.path().join(name), content).expect("writes config");
        };
        write("base.json", r#"{"include":["src/**/*.ts"]}"#);
        write("first.json", r#"{"extends":"./base.json"}"#);
        write("second.json", r#"{"extends":"./base.json"}"#);
        write(
            "tsconfig.json",
            r#"{"extends":["./first.json","./second.json"]}"#,
        );
        let config =
            load_project_config(&root.path().join("tsconfig.json")).expect("loads shared base");
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));

        write("base.json", r#"{"extends":"./tsconfig.json"}"#);
        let error =
            load_project_config(&root.path().join("tsconfig.json")).expect_err("rejects cycle");
        assert!(error.to_string().contains("Circular tsconfig extends"));
    }

    #[test]
    fn resolves_dotted_extends_names_and_prefers_exact_files() {
        let root = tempfile::tempdir().expect("creates project");
        let exact = root.path().join("tsconfig.base");
        fs::write(&exact, r#"{"include":["exact/**/*.ts"]}"#).expect("writes exact config");
        fs::write(
            root.path().join("tsconfig.base.json"),
            r#"{"include":["fallback/**/*.ts"]}"#,
        )
        .expect("writes fallback config");
        let config_path = root.path().join("tsconfig.json");
        fs::write(&config_path, r#"{"extends":"./tsconfig.base"}"#).expect("writes config");
        assert_eq!(
            load_project_config(&config_path)
                .expect("loads exact config")
                .include,
            Some(vec!["exact/**/*.ts".into()])
        );
        fs::remove_file(exact).expect("removes exact config");
        assert_eq!(
            load_project_config(&config_path)
                .expect("loads fallback config")
                .include,
            Some(vec!["fallback/**/*.ts".into()])
        );
    }

    #[test]
    fn resolves_absolute_extends_paths() {
        let root = tempfile::tempdir().expect("creates project");
        let base = root.path().join("base.json");
        fs::write(&base, r#"{"include":["src/**/*.ts"]}"#).expect("writes base config");
        let config_path = root.path().join("tsconfig.json");
        fs::write(
            &config_path,
            serde_json::json!({"extends": base}).to_string(),
        )
        .expect("writes config");
        let config = load_project_config(&config_path).expect("loads absolute extends");
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
    }

    #[test]
    fn resolves_extends_paths_with_backslashes() {
        let root = tempfile::tempdir().expect("creates project");
        let base_dir = root.path().join("config");
        fs::create_dir_all(&base_dir).expect("creates config directory");
        fs::write(base_dir.join("base.json"), r#"{"include":["src/**/*.ts"]}"#)
            .expect("writes base config");
        let config_path = root.path().join("tsconfig.json");
        fs::write(
            &config_path,
            serde_json::json!({"extends": ".\\config\\base"}).to_string(),
        )
        .expect("writes project config");
        let config = load_project_config(&config_path).expect("loads config");
        assert_eq!(config.include, Some(vec!["config/src/**/*.ts".into()]));
    }

    #[test]
    fn resolves_dotted_package_extends_subpaths() {
        let root = tempfile::tempdir().expect("creates project");
        let package = root.path().join("node_modules/config-pkg");
        fs::create_dir_all(&package).expect("creates package");
        fs::write(
            package.join("tsconfig.base.json"),
            r#"{"include":["src/**/*.ts"]}"#,
        )
        .expect("writes base config");
        let config_path = root.path().join("tsconfig.json");
        fs::write(&config_path, r#"{"extends":"config-pkg/tsconfig.base"}"#)
            .expect("writes config");
        let config = load_project_config(&config_path).expect("loads package subpath");
        assert_eq!(
            config.include,
            Some(vec!["node_modules/config-pkg/src/**/*.ts".into()])
        );
    }

    #[test]
    fn accepts_jsonc_comments_and_trailing_commas() {
        let path = std::env::temp_dir().join(format!(
            "type-tune-jsonc-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::write(
            &path,
            "{\n  // Scan source files.\n  \"include\": [\"src/**/*.ts\",],\n  /* Alias root. */\n  \"compilerOptions\": { \"baseUrl\": \".\", },\n}",
        )
        .expect("writes config");
        let config = load_project_config(&path).expect("loads JSONC config");
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
        assert_eq!(config.base_url.as_deref(), Some("."));
        fs::remove_file(path).expect("removes fixture");
    }

    #[test]
    fn loads_tsconfig_with_a_utf8_byte_order_mark() {
        let root = tempfile::tempdir().expect("creates project");
        let path = root.path().join("tsconfig.json");
        fs::write(&path, "\u{feff}{\"include\":[\"src/**/*.ts\"]}").expect("writes config");
        let config = load_project_config(&path).expect("loads config");
        assert_eq!(config.include, Some(vec!["src/**/*.ts".into()]));
    }

    #[test]
    fn loads_package_metadata_with_a_utf8_byte_order_mark() {
        let root = tempfile::tempdir().expect("creates project");
        let package = root.path().join("node_modules/config-package");
        fs::create_dir_all(&package).expect("creates package");
        fs::write(
            root.path().join("package.json"),
            "\u{feff}{\"imports\":{\"#model\":\"./src/model.ts\"}}",
        )
        .expect("writes project metadata");
        fs::write(
            package.join("package.json"),
            "\u{feff}{\"tsconfig\":\"base.json\"}",
        )
        .expect("writes package metadata");
        fs::write(package.join("base.json"), "{}").expect("writes base config");
        fs::write(
            root.path().join("tsconfig.json"),
            "{\"extends\":\"config-package\"}",
        )
        .expect("writes config");
        let config = load_project_config(&root.path().join("tsconfig.json")).expect("loads config");
        assert_eq!(
            config.paths.get("#model"),
            Some(&vec!["src/model.ts".into()])
        );
    }

    #[test]
    fn excludes_out_dir_unless_exclude_is_explicit() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-outdir-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(
            root.join("default.json"),
            r#"{"compilerOptions":{"outDir":"dist"}}"#,
        )
        .expect("writes default config");
        fs::write(
            root.join("explicit.json"),
            r#"{"compilerOptions":{"outDir":"dist"},"exclude":["src/skip.ts"]}"#,
        )
        .expect("writes explicit config");
        let default_config = load_project_config(&root.join("default.json")).expect("loads");
        assert!(
            default_config
                .exclude
                .iter()
                .any(|pattern| pattern == "dist")
        );
        let explicit_config = load_project_config(&root.join("explicit.json")).expect("loads");
        assert_eq!(explicit_config.exclude, ["src/skip.ts"]);
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn keeps_an_explicit_empty_files_list() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-empty-files-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        fs::write(root.join("tsconfig.json"), r#"{"files":[]}"#).expect("writes config");
        fs::write(root.join("base.json"), r#"{"files":[],"include":["src"]}"#)
            .expect("writes base");
        fs::write(
            root.join("child.json"),
            r#"{"extends":"./base.json","files":["src/a.ts"]}"#,
        )
        .expect("writes child");
        let config = load_project_config(&root.join("tsconfig.json")).expect("loads");
        assert_eq!(config.files, Some(vec![]));
        assert!(config.include.is_none());
        let inherited = load_project_config(&root.join("child.json")).expect("loads child");
        assert_eq!(inherited.files, Some(vec!["src/a.ts".into()]));
        assert_eq!(inherited.include, Some(vec!["src".into()]));
        fs::remove_dir_all(root).expect("removes fixture");
    }
}
