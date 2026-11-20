//! Limited preflight for project sources reached through relative imports.

use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use crate::{
    Result, TypeTuneError,
    config::ProjectConfig,
    module_path::{filesystem_alias_bases, filesystem_module_candidates},
    module_specifier::is_relative_module_specifier,
    source::validate_discovery_path,
};

fn lexical(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn project_source(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    ![".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        && [".ts", ".tsx", ".mts", ".cts", ".vue"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// Reject a resolved relative project source that cannot safely enter the scan graph.
pub(super) fn check_source(
    root: &Path,
    importer: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    config: &ProjectConfig,
) -> Result<()> {
    let mut bases = if is_relative_module_specifier(specifier) {
        let parent = Path::new(importer).parent().unwrap_or(Path::new(""));
        vec![lexical(&root.join(parent).join(specifier))]
    } else {
        filesystem_alias_bases(
            importer,
            specifier,
            config.base_url.as_deref(),
            &config.explicit_paths,
            &config.package_aliases,
        )
        .into_iter()
        .map(|base| lexical(&root.join(base)))
        .collect()
    };
    if !is_relative_module_specifier(specifier)
        && let Some(base) = simple_package_type_entry(root, importer, specifier)?
    {
        bases.push(base);
    }
    for base in bases {
        for candidate in
            filesystem_module_candidates(&base.to_string_lossy(), &config.module_suffixes)
        {
            let path = PathBuf::from(candidate);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let declaration = [".d.ts", ".d.mts", ".d.cts"]
                .iter()
                .any(|suffix| name.ends_with(suffix));
            if !project_source(&path) && !declaration {
                continue;
            }
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {}
                Ok(_) => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(TypeTuneError::new(format!(
                        "Import resolution cannot inspect {specifier:?} from {importer}: {}: {error}",
                        path.display()
                    )));
                }
            }
            if declaration {
                // Declarations retain their existing separate dependency handling.
                return Ok(());
            }
            let canonical = fs::canonicalize(&path).map_err(|error| {
                TypeTuneError::new(format!(
                    "Import resolution cannot resolve {specifier:?} from {importer}: {}: {error}",
                    path.display()
                ))
            })?;
            let diagnostic = |reason: &str| {
                TypeTuneError::new(format!(
                    "Import resolution cannot analyze {specifier:?} from {importer}: {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
                    path.display(),
                    canonical.display(),
                    root.display()
                ))
            };
            if !canonical.starts_with(root) {
                return Err(diagnostic(
                    "project source resolves outside the project boundary",
                ));
            }
            if validate_discovery_path(root, &path).is_err() {
                return Err(diagnostic("source symlink identity is unsupported"));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| diagnostic("logical source is outside the project boundary"))?
                .to_string_lossy()
                .replace('\\', "/");
            if !known_paths.contains(&relative) {
                return Err(diagnostic(
                    "project source is outside the discovered analysis universe",
                ));
            }
            // The existing resolver handles this supported source candidate.
            return Ok(());
        }
    }
    Ok(())
}

/// Resolve a simple package `types`/`typings` or exact export `types` target.
/// typesVersions, ambiguous conditions, and JS fallback remain unsupported.
fn simple_package_type_entry(
    root: &Path,
    importer: &str,
    specifier: &str,
) -> Result<Option<PathBuf>> {
    let parts: Vec<_> = specifier.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Ok(None);
    }
    let package_parts = if parts[0].starts_with('@') {
        if parts.len() < 2 || parts[0].len() == 1 {
            return Ok(None);
        }
        2
    } else if !parts[0].starts_with(['.', '#']) && !parts[0].contains(':') {
        1
    } else {
        return Ok(None);
    };
    let package_name = parts[..package_parts].join("/");
    let subpath = parts[package_parts..].join("/");
    let parent = root.join(Path::new(importer).parent().unwrap_or(Path::new("")));
    for directory in parent.ancestors() {
        let package = directory.join("node_modules").join(&package_name);
        let manifest = package.join("package.json");
        let content = match fs::read_to_string(&manifest) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(TypeTuneError::new(format!(
                    "Could not inspect dependency metadata {}: {error}",
                    manifest.display()
                )));
            }
        };
        let metadata: serde_json::Value =
            serde_json::from_str(content.trim_start_matches('\u{feff}')).map_err(|error| {
                TypeTuneError::new(format!(
                    "Could not parse dependency metadata {}: {error}",
                    manifest.display()
                ))
            })?;
        if let Some(exports) = metadata.get("exports") {
            let export_key = if subpath.is_empty() {
                ".".to_owned()
            } else {
                format!("./{subpath}")
            };
            let has_subpaths = exports
                .as_object()
                .is_some_and(|object| object.keys().any(|key| key.starts_with('.')));
            let (root_export, wildcard) = if has_subpaths {
                if let Some(target) = exports.get(&export_key) {
                    (target, None)
                } else {
                    let Some((_, captured, target)) = exports
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(pattern, target)| {
                            let (prefix, suffix) = pattern.split_once('*')?;
                            let captured = export_key.strip_prefix(prefix)?.strip_suffix(suffix)?;
                            Some(((prefix.len(), suffix.len()), captured, target))
                        })
                        .max_by_key(|(specificity, _, _)| *specificity)
                    else {
                        return Ok(None);
                    };
                    (target, Some(captured))
                }
            } else if subpath.is_empty() {
                (exports, None)
            } else {
                return Ok(None);
            };
            if let Some(target) = root_export.as_str() {
                let target =
                    wildcard.map_or_else(|| target.to_owned(), |value| target.replace('*', value));
                return Ok(Some(lexical(&package.join(target))));
            }
            // Inspect a nested `types` condition only when all such conditions
            // identify the same target. Multiple alternatives require a fuller
            // condition-aware resolver and are still unsupported.
            fn type_targets<'a>(value: &'a serde_json::Value, result: &mut Vec<&'a str>) {
                if let Some(object) = value.as_object() {
                    for (key, child) in object {
                        if key == "types" {
                            if let Some(target) = child.as_str() {
                                result.push(target);
                            }
                        } else {
                            type_targets(child, result);
                        }
                    }
                } else if let Some(items) = value.as_array() {
                    for child in items {
                        type_targets(child, result);
                    }
                }
            }
            let mut targets = Vec::new();
            type_targets(root_export, &mut targets);
            targets.sort_unstable();
            targets.dedup();
            if targets.len() > 1 {
                for target in &targets {
                    let target = wildcard
                        .map_or_else(|| (*target).to_owned(), |value| target.replace('*', value));
                    let path = lexical(&package.join(target));
                    if project_source(&path) && path.is_file() {
                        let canonical = fs::canonicalize(&path).map_err(|error| {
                            TypeTuneError::new(format!(
                                "Could not resolve {}: {error}",
                                path.display()
                            ))
                        })?;
                        return Err(TypeTuneError::new(format!(
                            "Import resolution cannot analyze {specifier:?} from {importer}: ambiguous package type targets; logical path: {}; canonical path: {}; project boundary: {}.",
                            path.display(),
                            canonical.display(),
                            root.display()
                        )));
                    }
                }
                return Ok(None);
            }
            return Ok((targets.len() == 1).then(|| {
                let target = wildcard.map_or_else(
                    || targets[0].to_owned(),
                    |value| targets[0].replace('*', value),
                );
                lexical(&package.join(target))
            }));
        }
        if let Some(versions) = metadata
            .get("typesVersions")
            .and_then(serde_json::Value::as_object)
        {
            // The native scanner does not select a TypeScript version range.
            // If any matching alternative points at a real source, fail closed
            // through the ordinary source identity check instead of omitting it.
            for mappings in versions.values().filter_map(serde_json::Value::as_object) {
                for (pattern, targets) in mappings {
                    let capture = if let Some((prefix, suffix)) = pattern.split_once('*') {
                        subpath
                            .strip_prefix(prefix)
                            .and_then(|value| value.strip_suffix(suffix))
                    } else if pattern == &subpath {
                        Some("")
                    } else {
                        None
                    };
                    if let Some(capture) = capture {
                        for target in targets
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(serde_json::Value::as_str)
                        {
                            let path = lexical(&package.join(target.replace('*', capture)));
                            if filesystem_module_candidates(&path.to_string_lossy(), &[])
                                .iter()
                                .map(Path::new)
                                .any(|candidate| project_source(candidate) && candidate.is_file())
                            {
                                return Ok(Some(path));
                            }
                        }
                    }
                }
            }
            return Ok(None);
        }
        if !subpath.is_empty() {
            return Ok(Some(lexical(&package.join(subpath))));
        }
        return Ok(metadata
            .get("types")
            .or_else(|| metadata.get("typings"))
            .and_then(serde_json::Value::as_str)
            .map(|target| lexical(&package.join(target))));
    }
    Ok(None)
}
