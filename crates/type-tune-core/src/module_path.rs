//! Project-relative module-specifier resolution.

use crate::module_specifier::{
    is_relative_module_specifier, preserved_module_specifier_extension,
    relative_module_specifier_with_extension,
};
use std::collections::{BTreeMap, HashSet};

/// Package module aliases grouped by project-relative package scope.
pub type PackageAliasScopes = BTreeMap<String, BTreeMap<String, Vec<String>>>;

/// Resolves a relative TypeScript module specifier against known project files.
#[must_use]
pub fn resolve_relative_module(
    referrer_path: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
) -> Option<String> {
    resolve_relative_module_with_options(referrer_path, specifier, known_paths, &[], &[])
}

/// Resolves a relative TypeScript module specifier with compiler `rootDirs` support.
#[must_use]
pub fn resolve_relative_module_with_root_dirs(
    referrer_path: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    root_dirs: &[String],
) -> Option<String> {
    resolve_relative_module_with_options(referrer_path, specifier, known_paths, root_dirs, &[])
}

/// Resolves a relative TypeScript module specifier with compiler `rootDirs` and `moduleSuffixes`.
#[must_use]
pub fn resolve_relative_module_with_options(
    referrer_path: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    root_dirs: &[String],
    module_suffixes: &[String],
) -> Option<String> {
    let base = relative_module_base(referrer_path, specifier)?;
    resolve_module_base_with_suffixes(&base, known_paths, module_suffixes).or_else(|| {
        root_dirs.iter().find_map(|source_root| {
            let source_root = source_root.trim_end_matches('/');
            let referrer_relative = if source_root.is_empty() {
                referrer_path
            } else {
                referrer_path.strip_prefix(&format!("{source_root}/"))?
            };
            let virtual_base = relative_module_base(referrer_relative, specifier)?;
            root_dirs.iter().find_map(|target_root| {
                let target_root = target_root.trim_end_matches('/');
                let target_base = if target_root.is_empty() {
                    virtual_base.clone()
                } else {
                    format!("{target_root}/{virtual_base}")
                };
                resolve_module_base_with_suffixes(&target_base, known_paths, module_suffixes)
            })
        })
    })
}

fn relative_module_base(referrer_path: &str, specifier: &str) -> Option<String> {
    if !is_relative_module_specifier(specifier) {
        return None;
    }
    let mut segments = referrer_path.split('/').collect::<Vec<_>>();
    segments.pop();
    for segment in specifier.split('/') {
        match segment {
            "." => {}
            ".." => {
                segments.pop()?;
            }
            value => segments.push(value),
        }
    }
    Some(segments.join("/"))
}

fn resolve_module_base(base: &str, known_paths: &HashSet<String>) -> Option<String> {
    module_candidates(base)
        .into_iter()
        .find(|candidate| known_paths.contains(candidate))
}

fn resolve_module_base_with_suffixes(
    base: &str,
    known_paths: &HashSet<String>,
    module_suffixes: &[String],
) -> Option<String> {
    if module_suffixes.is_empty() {
        return resolve_module_base(base, known_paths);
    }
    module_file_candidates(base)
        .into_iter()
        .chain(module_index_candidates(base))
        .find_map(|candidate| {
            module_suffixes.iter().find_map(|suffix| {
                let candidate = apply_module_suffix(&candidate, suffix);
                known_paths.contains(&candidate).then_some(candidate)
            })
        })
}

fn apply_module_suffix(base: &str, suffix: &str) -> String {
    for extension in [
        ".d.mts", ".d.cts", ".d.tsx", ".d.ts", ".mts", ".cts", ".tsx", ".ts", ".mjs", ".cjs",
        ".jsx", ".js",
    ] {
        if let Some(stem) = base.strip_suffix(extension) {
            return format!("{stem}{suffix}{extension}");
        }
    }
    format!("{base}{suffix}")
}

/// Resolves a TypeScript path alias against known project files.
#[must_use]
pub fn resolve_path_alias(
    specifier: &str,
    paths: &BTreeMap<String, Vec<String>>,
    known_paths: &HashSet<String>,
) -> Option<String> {
    resolve_path_alias_with_module_suffixes(specifier, paths, known_paths, &[])
}

fn resolve_path_alias_with_module_suffixes(
    specifier: &str,
    paths: &BTreeMap<String, Vec<String>>,
    known_paths: &HashSet<String>,
    module_suffixes: &[String],
) -> Option<String> {
    let (_, star_value, targets) = paths
        .iter()
        .filter_map(|(alias, targets)| {
            match_target_pattern(specifier, alias)
                .map(|star_value| (alias.find('*').unwrap_or(usize::MAX), star_value, targets))
        })
        .max_by_key(|(specificity, _, _)| *specificity)?;
    for target in targets {
        let base = apply_alias_pattern(target, star_value);
        if let Some(candidate) =
            resolve_module_base_with_suffixes(&base, known_paths, module_suffixes)
        {
            return Some(candidate);
        }
    }
    None
}

fn module_candidates(base: &str) -> Vec<String> {
    let mut candidates = module_file_candidates(base);
    candidates.extend(module_index_candidates(base));
    candidates
}

pub(crate) fn filesystem_module_candidates(base: &str, suffixes: &[String]) -> Vec<String> {
    let candidates = module_candidates(base);
    if suffixes.is_empty() {
        candidates
    } else {
        candidates
            .into_iter()
            .flat_map(|candidate| {
                suffixes
                    .iter()
                    .map(move |suffix| apply_module_suffix(&candidate, suffix))
            })
            .collect()
    }
}

pub(crate) fn filesystem_alias_bases(
    referrer: &str,
    specifier: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
    package_aliases: &PackageAliasScopes,
) -> Vec<String> {
    if is_relative_module_specifier(specifier) {
        return Vec::new();
    }
    let bases_for = |paths: &BTreeMap<String, Vec<String>>| {
        let normalized = normalize_path_aliases(base_url, paths);
        normalized
            .iter()
            .filter_map(|(alias, targets)| {
                match_target_pattern(specifier, alias)
                    .map(|star| (alias.find('*').unwrap_or(usize::MAX), star, targets))
            })
            .max_by_key(|(length, _, _)| *length)
            .map(|(_, star, targets)| {
                targets
                    .iter()
                    .map(|target| apply_alias_pattern(target, star))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut result = bases_for(paths);
    if let Some(base) = base_url {
        result.push(format!("{}/{}", base.trim_end_matches('/'), specifier));
    }
    if let Some((_, scoped)) = package_aliases
        .iter()
        .filter(|(scope, _)| {
            *scope == "."
                || referrer == scope.as_str()
                || referrer
                    .strip_prefix(scope.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|(scope, _)| scope.len())
    {
        result.extend(bases_for(scoped));
    }
    result
}

fn module_file_candidates(base: &str) -> Vec<String> {
    let jsx_candidates = base.strip_suffix(".jsx").map(|stem| {
        vec![
            format!("{stem}.tsx"),
            format!("{stem}.ts"),
            format!("{stem}.d.ts"),
        ]
    });
    let (base, extensions) = match base
        .strip_suffix(".mjs")
        .map(|base| (base, [".mts", ".d.mts"]))
        .or_else(|| {
            base.strip_suffix(".cjs")
                .map(|base| (base, [".cts", ".d.cts"]))
        }) {
        Some(value) => value,
        None => (base.strip_suffix(".js").unwrap_or(base), [".ts", ".tsx"]),
    };
    let mut candidates = jsx_candidates.unwrap_or_default();
    candidates.push(base.to_owned());
    for extension in extensions {
        candidates.push(format!("{base}{extension}"));
    }
    if extensions == [".ts", ".tsx"] {
        candidates.extend([
            format!("{base}.d.ts"),
            format!("{base}.mts"),
            format!("{base}.cts"),
            format!("{base}.d.tsx"),
            format!("{base}.d.mts"),
            format!("{base}.d.cts"),
        ]);
    }
    candidates
}

fn module_index_candidates(base: &str) -> Vec<String> {
    let (base, extensions) = match base
        .strip_suffix(".mjs")
        .map(|base| (base, [".mts", ".d.mts"]))
        .or_else(|| {
            base.strip_suffix(".cjs")
                .map(|base| (base, [".cts", ".d.cts"]))
        }) {
        Some(value) => value,
        None => (base.strip_suffix(".js").unwrap_or(base), [".ts", ".tsx"]),
    };
    let mut candidates = Vec::new();
    for extension in extensions {
        candidates.push(format!("{base}/index{extension}"));
    }
    if extensions == [".ts", ".tsx"] {
        candidates.extend([
            format!("{base}/index.d.ts"),
            format!("{base}/index.mts"),
            format!("{base}/index.cts"),
            format!("{base}/index.d.tsx"),
            format!("{base}/index.d.mts"),
            format!("{base}/index.d.cts"),
        ]);
    }
    candidates
}

/// Resolves a path alias import using `baseUrl`-relative target patterns.
#[must_use]
pub fn resolve_path_alias_with_base_url(
    specifier: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
    known_paths: &HashSet<String>,
) -> Option<String> {
    resolve_path_alias_with_base_url_and_module_suffixes(
        specifier,
        base_url,
        paths,
        known_paths,
        &[],
    )
}

/// Resolves a path alias or bare `baseUrl` specifier with compiler `moduleSuffixes`.
#[must_use]
pub fn resolve_path_alias_with_base_url_and_module_suffixes(
    specifier: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
    known_paths: &HashSet<String>,
    module_suffixes: &[String],
) -> Option<String> {
    let normalized_paths = normalize_path_aliases(base_url, paths);
    resolve_path_alias_with_module_suffixes(
        specifier,
        &normalized_paths,
        known_paths,
        module_suffixes,
    )
    .or_else(|| {
        resolve_base_url_module_specifier(specifier, base_url, known_paths, module_suffixes)
    })
}

/// Resolves a module specifier through explicit aliases and its nearest package scope aliases.
#[must_use]
pub fn resolve_scoped_path_alias_with_base_url_and_module_suffixes(
    referrer_path: &str,
    module_specifier: &str,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    package_aliases: &PackageAliasScopes,
    known_paths: &HashSet<String>,
    module_suffixes: &[String],
) -> Option<String> {
    resolve_path_alias_with_base_url_and_module_suffixes(
        module_specifier,
        base_url,
        path_aliases,
        known_paths,
        module_suffixes,
    )
    .or_else(|| {
        package_aliases
            .iter()
            .filter(|(scope, _)| {
                *scope == "."
                    || referrer_path == scope.as_str()
                    || referrer_path
                        .strip_prefix(scope.as_str())
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|(scope, _)| scope.len())
            .and_then(|(_, aliases)| {
                resolve_path_alias_with_base_url_and_module_suffixes(
                    module_specifier,
                    base_url,
                    aliases,
                    known_paths,
                    module_suffixes,
                )
            })
    })
}

fn resolve_base_url_module_specifier(
    specifier: &str,
    base_url: Option<&str>,
    known_paths: &HashSet<String>,
    module_suffixes: &[String],
) -> Option<String> {
    if is_relative_module_specifier(specifier) {
        return None;
    }
    let base_url = base_url?.trim_matches('/');
    let base = if base_url.is_empty() || base_url == "." {
        specifier.to_owned()
    } else {
        format!("{base_url}/{specifier}")
    };
    resolve_module_base_with_suffixes(&base, known_paths, module_suffixes)
}

/// Creates an alias module specifier for a project-relative destination path.
#[must_use]
pub fn resolve_alias_module_specifier(
    destination_path: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
) -> Option<String> {
    resolve_alias_module_specifier_from(destination_path, base_url, paths, None)
}

/// Creates an alias specifier, preferring the alias pattern used by `original_specifier`.
///
/// A target of `./*` is the same project-relative pattern as `*` when `baseUrl` is the project
/// root. `~/*` and `@/*` can share that target, so the original import decides which prefix stays.
#[must_use]
pub fn resolve_alias_module_specifier_from(
    destination_path: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
    original_specifier: Option<&str>,
) -> Option<String> {
    resolve_alias_module_specifier_with_known_paths(
        destination_path,
        base_url,
        paths,
        original_specifier,
        None,
        &[],
        "",
    )
}

pub(crate) fn resolve_alias_module_specifier_with_known_paths(
    destination_path: &str,
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
    original_specifier: Option<&str>,
    known_paths: Option<&HashSet<String>>,
    module_suffixes: &[String],
    extension: &str,
) -> Option<String> {
    let destination = destination_without_extension(destination_path);
    let mut known_destination = known_paths.cloned().unwrap_or_default();
    known_destination.insert(destination_path.to_owned());
    let normalized_paths = normalize_path_aliases(base_url, paths);
    let mut aliases = paths.keys().cloned().collect::<Vec<_>>();
    if let Some(original_specifier) = original_specifier
        && let Some(preferred) = preferred_alias(original_specifier, paths)
    {
        aliases.retain(|alias| alias != &preferred);
        aliases.insert(0, preferred);
    }
    for alias in aliases {
        let Some(targets) = normalized_paths.get(&alias) else {
            continue;
        };
        // Files outside the scan can shadow later targets during TypeScript resolution.
        for target in targets.iter().take(1) {
            let target = normalized_alias_target(target);
            if let Some(star_value) = match_target_pattern(&destination, &target) {
                let specifier = apply_alias_pattern(&alias, star_value);
                if resolve_path_alias_with_base_url_and_module_suffixes(
                    &format!("{specifier}{extension}"),
                    base_url,
                    paths,
                    &known_destination,
                    module_suffixes,
                )
                .as_deref()
                    == Some(destination_path)
                {
                    return Some(specifier);
                }
            }
        }
    }
    None
}

pub(crate) fn is_configured_path_alias(
    specifier: &str,
    paths: &BTreeMap<String, Vec<String>>,
) -> bool {
    preferred_alias(specifier, paths).is_some()
}

pub(crate) fn configured_alias_pattern(
    specifier: &str,
    paths: &BTreeMap<String, Vec<String>>,
) -> Option<String> {
    preferred_alias(specifier, paths)
}

/// Keeps an alias specifier when `original_specifier` already used one.
///
/// Relative barrel exports stay relative. An alias that no longer covers the destination falls
/// back to a relative specifier from `from_path`.
#[must_use]
pub fn module_specifier_preserving_alias(
    from_path: &str,
    destination_path: &str,
    extension: &str,
    original_specifier: &str,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
) -> String {
    if !is_relative_module_specifier(original_specifier)
        && is_configured_path_alias(original_specifier, path_aliases)
        && let Some(alias) = resolve_alias_module_specifier_from(
            destination_path,
            base_url,
            path_aliases,
            Some(original_specifier),
        )
    {
        let extension =
            preserved_module_specifier_extension(original_specifier).unwrap_or(extension);
        return format!("{alias}{extension}");
    }
    relative_module_specifier_with_extension(from_path, destination_path, extension)
}

fn preferred_alias(specifier: &str, paths: &BTreeMap<String, Vec<String>>) -> Option<String> {
    paths
        .keys()
        .filter(|alias| alias_covers_specifier(alias, specifier))
        .max_by(|left, right| {
            alias_prefix_len(left)
                .cmp(&alias_prefix_len(right))
                .then_with(|| right.len().cmp(&left.len()))
        })
        .cloned()
}

fn alias_covers_specifier(alias: &str, specifier: &str) -> bool {
    match alias.split_once('*') {
        Some((prefix, suffix)) => {
            specifier.len() >= prefix.len() + suffix.len()
                && specifier.starts_with(prefix)
                && specifier.ends_with(suffix)
        }
        None => specifier == alias,
    }
}

fn alias_prefix_len(alias: &str) -> usize {
    alias
        .split_once('*')
        .map_or(alias.len(), |(prefix, _)| prefix.len())
}

fn normalized_alias_target(target: &str) -> String {
    let target = target.strip_prefix("./").unwrap_or(target);
    destination_without_extension(target)
}

fn normalize_path_aliases(
    base_url: Option<&str>,
    paths: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let prefix = base_url
        .filter(|value| !value.is_empty() && *value != ".")
        .map(|value| format!("{}/", value.trim_end_matches('/')))
        .unwrap_or_default();
    paths
        .iter()
        .map(|(alias, targets)| {
            (
                alias.clone(),
                targets
                    .iter()
                    .filter_map(|target| {
                        let target = target.strip_prefix("./").unwrap_or(target);
                        normalize_module_path(&format!("{prefix}{target}"))
                    })
                    .collect(),
            )
        })
        .collect()
}

fn normalize_module_path(path: &str) -> Option<String> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(segment),
        }
    }
    Some(segments.join("/"))
}

fn destination_without_extension(path: &str) -> String {
    path.strip_suffix(".tsx")
        .or_else(|| path.strip_suffix(".mts"))
        .or_else(|| path.strip_suffix(".cts"))
        .or_else(|| path.strip_suffix(".ts"))
        .unwrap_or(path)
        .to_owned()
}

fn match_target_pattern<'a>(value: &'a str, pattern: &str) -> Option<&'a str> {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => value.strip_prefix(prefix)?.strip_suffix(suffix),
        None if value == pattern => Some(""),
        None => None,
    }
}

fn apply_alias_pattern(alias: &str, star_value: &str) -> String {
    alias.replace('*', star_value)
}

#[cfg(test)]
mod tests {
    use super::{
        PackageAliasScopes, resolve_alias_module_specifier, resolve_alias_module_specifier_from,
        resolve_path_alias, resolve_path_alias_with_base_url,
        resolve_path_alias_with_base_url_and_module_suffixes, resolve_relative_module,
        resolve_relative_module_with_options, resolve_relative_module_with_root_dirs,
        resolve_scoped_path_alias_with_base_url_and_module_suffixes,
    };
    use std::collections::{BTreeMap, HashSet};

    #[test]
    fn resolves_extensions_and_index_files() {
        let paths = HashSet::from(["src/shared.ts".into(), "src/types/index.ts".into()]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./shared", &paths),
            Some("src/shared.ts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./types", &paths),
            Some("src/types/index.ts".into())
        );
    }

    #[test]
    fn does_not_resolve_relative_imports_outside_the_project() {
        let paths = HashSet::from(["shared.ts".into()]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "../../shared", &paths),
            None
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "../shared", &paths),
            Some("shared.ts".into())
        );
    }

    #[test]
    fn resolves_declaration_files() {
        let paths = HashSet::from(["src/external.d.ts".into(), "src/types/index.d.ts".into()]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./external", &paths),
            Some("src/external.d.ts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./types", &paths),
            Some("src/types/index.d.ts".into())
        );
    }

    #[test]
    fn prefers_declaration_files_over_mts_fallbacks() {
        let paths = HashSet::from([
            "src/shared.d.ts".into(),
            "src/shared.mts".into(),
            "src/types/index.d.ts".into(),
            "src/types/index.mts".into(),
        ]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./shared.js", &paths),
            Some("src/shared.d.ts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./types", &paths),
            Some("src/types/index.d.ts".into())
        );
    }

    #[test]
    fn resolves_mts_and_cts_files() {
        let paths = HashSet::from([
            "src/schema.mts".into(),
            "src/legacy.cts".into(),
            "src/types/index.d.mts".into(),
            "src/cjs/index.d.cts".into(),
        ]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./schema", &paths),
            Some("src/schema.mts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./legacy", &paths),
            Some("src/legacy.cts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./types", &paths),
            Some("src/types/index.d.mts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./cjs", &paths),
            Some("src/cjs/index.d.cts".into())
        );
    }

    #[test]
    fn resolves_typescript_sources_from_javascript_specifiers() {
        let paths = HashSet::from([
            "src/shared.ts".into(),
            "src/schema.mts".into(),
            "src/legacy.cts".into(),
        ]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./shared.js", &paths),
            Some("src/shared.ts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./schema.mjs", &paths),
            Some("src/schema.mts".into())
        );
        assert_eq!(
            resolve_relative_module("src/app.ts", "./legacy.cjs", &paths),
            Some("src/legacy.cts".into())
        );
    }

    #[test]
    fn resolves_jsx_specifiers_to_tsx_before_ts() {
        let paths = HashSet::from(["src/component.tsx".into(), "src/component.ts".into()]);
        assert_eq!(
            resolve_relative_module("src/app.ts", "./component.jsx", &paths),
            Some("src/component.tsx".into())
        );
        assert_eq!(
            resolve_relative_module_with_options(
                "src/app.ts",
                "./component.jsx",
                &HashSet::from(["src/component.native.tsx".into()]),
                &[],
                &[".native".into()]
            ),
            Some("src/component.native.tsx".into())
        );
    }

    #[test]
    fn resolves_relative_modules_across_root_directories() {
        let paths = HashSet::from(["src/shared.ts".into(), "virtual/app.ts".into()]);
        assert_eq!(
            resolve_relative_module_with_root_dirs(
                "virtual/app.ts",
                "./shared",
                &paths,
                &["src".into(), "virtual".into()],
            ),
            Some("src/shared.ts".into())
        );
    }

    #[test]
    fn resolves_relative_modules_with_module_suffixes() {
        let paths = HashSet::from(["src/shared.native.ts".into(), "src/app.ts".into()]);
        assert_eq!(
            resolve_relative_module_with_options(
                "src/app.ts",
                "./shared",
                &paths,
                &[],
                &[".native".into(), "".into()],
            ),
            Some("src/shared.native.ts".into())
        );
    }

    #[test]
    fn prefers_file_extensions_before_module_suffixes() {
        let paths = HashSet::from([
            "src/shared.ts".into(),
            "src/shared.native.tsx".into(),
            "src/feature/index.native.ts".into(),
        ]);
        let suffixes = [".native".into(), "".into()];
        assert_eq!(
            resolve_relative_module_with_options("src/app.ts", "./shared", &paths, &[], &suffixes),
            Some("src/shared.ts".into())
        );
        let paths = HashSet::from([
            "src/feature.ts".into(),
            "src/feature/index.native.ts".into(),
        ]);
        assert_eq!(
            resolve_relative_module_with_options("src/app.ts", "./feature", &paths, &[], &suffixes),
            Some("src/feature.ts".into())
        );
    }

    #[test]
    fn resolves_explicit_typescript_extensions_with_module_suffixes() {
        let paths = HashSet::from(["src/shared.native.ts".into(), "src/app.ts".into()]);
        assert_eq!(
            resolve_relative_module_with_options(
                "src/app.ts",
                "./shared.ts",
                &paths,
                &[],
                &[".native".into(), "".into()],
            ),
            Some("src/shared.native.ts".into())
        );
    }

    #[test]
    fn resolves_index_modules_with_module_suffixes() {
        let paths = HashSet::from(["src/feature/index.native.ts".into(), "src/app.ts".into()]);
        assert_eq!(
            resolve_relative_module_with_options(
                "src/app.ts",
                "./feature",
                &paths,
                &[],
                &[".native".into(), "".into()],
            ),
            Some("src/feature/index.native.ts".into())
        );
    }

    #[test]
    fn resolves_path_aliases() {
        let paths = BTreeMap::from([("@/*".into(), vec!["src/*".into()])]);
        let known = HashSet::from(["src/shared.ts".into()]);
        assert_eq!(
            resolve_path_alias("@/shared", &paths, &known),
            Some("src/shared.ts".into())
        );

        let paths = BTreeMap::from([("@feature/*/model".into(), vec!["src/*/model".into()])]);
        let known = HashSet::from(["src/user/model.ts".into()]);
        assert_eq!(
            resolve_path_alias("@feature/user/model", &paths, &known),
            Some("src/user/model.ts".into())
        );

        let paths = BTreeMap::from([
            ("@/*".into(), vec!["src/general/*".into()]),
            ("@/feature/*".into(), vec!["src/feature/*".into()]),
        ]);
        let known = HashSet::from([
            "src/general/feature/shared.ts".into(),
            "src/feature/shared.ts".into(),
        ]);
        assert_eq!(
            resolve_path_alias("@/feature/shared", &paths, &known),
            Some("src/feature/shared.ts".into())
        );
    }

    #[test]
    fn ignores_alias_patterns_with_overlapping_prefix_and_suffix() {
        let paths = BTreeMap::from([("@a*a".into(), vec!["src/*".into()])]);
        let known = HashSet::from(["src/index.ts".into()]);
        assert_eq!(resolve_path_alias("@a", &paths, &known), None);
    }

    #[test]
    fn resolves_bare_specifiers_from_base_url() {
        let known_paths = HashSet::from(["src/shared.ts".into()]);
        assert_eq!(
            resolve_path_alias_with_base_url("shared", Some("src"), &BTreeMap::new(), &known_paths),
            Some("src/shared.ts".into())
        );
    }

    #[test]
    fn resolves_aliases_relative_to_base_url() {
        let paths = BTreeMap::from([("@/*".into(), vec!["*".into()])]);
        let known = HashSet::from(["src/shared.ts".into()]);
        assert_eq!(
            resolve_path_alias_with_base_url("@/shared", Some("src"), &paths, &known),
            Some("src/shared.ts".into())
        );
        assert_eq!(
            resolve_alias_module_specifier("src/types/shared.ts", Some("src"), &paths),
            Some("@/types/shared".into())
        );
        let dot_slash = BTreeMap::from([
            ("@/*".into(), vec!["./*".into()]),
            ("~/*".into(), vec!["./*".into()]),
        ]);
        assert_eq!(
            resolve_alias_module_specifier_from(
                "services/domains/catalog-v2/types/catalog-v2.ts",
                Some("."),
                &dot_slash,
                Some("~/services/domains/catalog-v2/types"),
            ),
            Some("~/services/domains/catalog-v2/types/catalog-v2".into())
        );

        let paths = BTreeMap::from([("@/*".into(), vec!["../shared/*".into()])]);
        let known = HashSet::from(["shared/model.ts".into()]);
        assert_eq!(
            resolve_path_alias_with_base_url("@/model", Some("src"), &paths, &known),
            Some("shared/model.ts".into())
        );
    }

    #[test]
    fn does_not_generate_alias_shadowed_by_a_more_specific_pattern() {
        let paths = BTreeMap::from([
            ("@/*".into(), vec!["src/*".into()]),
            ("@/types/*".into(), vec!["other/*".into()]),
        ]);
        assert_eq!(
            resolve_alias_module_specifier("src/types/shared.ts", Some("."), &paths),
            None
        );
    }

    #[test]
    fn does_not_generate_base_url_alias_for_an_outside_destination() {
        let paths = BTreeMap::from([("@/*".into(), vec!["*".into()])]);
        assert_eq!(
            resolve_alias_module_specifier("types/shared.ts", Some("src"), &paths),
            None
        );
    }

    #[test]
    fn generates_alias_for_target_outside_base_url() {
        let paths = BTreeMap::from([("@/*".into(), vec!["../shared/*".into()])]);
        assert_eq!(
            resolve_alias_module_specifier("shared/types/order.ts", Some("src"), &paths),
            Some("@/types/order".into())
        );
    }

    #[test]
    fn does_not_resolve_alias_targets_above_the_project() {
        let paths = BTreeMap::from([("@/*".into(), vec!["../../outside/*".into()])]);
        let known = HashSet::from(["outside/shared.ts".into()]);
        assert_eq!(
            resolve_path_alias_with_base_url("@/shared", Some("src"), &paths, &known),
            None
        );
    }

    #[test]
    fn resolves_path_aliases_with_module_suffixes() {
        let paths = BTreeMap::from([("@/*".into(), vec!["*".into()])]);
        let known = HashSet::from(["src/shared.native.ts".into()]);
        assert_eq!(
            resolve_path_alias_with_base_url_and_module_suffixes(
                "@/shared",
                Some("src"),
                &paths,
                &known,
                &[".native".into(), "".into()],
            ),
            Some("src/shared.native.ts".into())
        );
    }

    #[test]
    fn resolves_aliases_from_the_nearest_package_scope() {
        let package_aliases = PackageAliasScopes::from([
            (
                "packages/app".into(),
                BTreeMap::from([("#shared".into(), vec!["packages/app/src/shared.ts".into()])]),
            ),
            (
                "packages/lib".into(),
                BTreeMap::from([("#shared".into(), vec!["packages/lib/src/shared.ts".into()])]),
            ),
        ]);
        let known = HashSet::from([
            "packages/app/src/shared.ts".into(),
            "packages/lib/src/shared.ts".into(),
        ]);
        assert_eq!(
            resolve_scoped_path_alias_with_base_url_and_module_suffixes(
                "packages/lib/src/lib.ts",
                "#shared",
                None,
                &BTreeMap::new(),
                &package_aliases,
                &known,
                &[],
            ),
            Some("packages/lib/src/shared.ts".into())
        );
    }
}
