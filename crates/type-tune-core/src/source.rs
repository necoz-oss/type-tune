//! TypeScript source discovery for native scans.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use crate::glob::matches;

/// Reads a TypeScript source file, skipping dangling symbolic links.
///
/// TypeScript project loading ignores unreadable symlink targets instead of failing the scan.
pub fn read_typescript_source_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == io::ErrorKind::NotFound && is_symlink(path) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Recursively returns TypeScript source files below `root` in lexical order.
///
/// Generated directories are skipped to preserve the default CLI contract.
pub fn discover_typescript_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    discover_typescript_files_with_generated(root, false)
}

/// Recursively returns TypeScript source files, optionally including generated sources.
pub fn discover_typescript_files_with_generated(
    root: &Path,
    include_generated: bool,
) -> std::io::Result<Vec<PathBuf>> {
    discover_typescript_files_with_options(root, include_generated, false)
}

/// Recursively returns TypeScript source and declaration files for module resolution.
///
/// Declaration files remain excluded from candidate scanning but are required to resolve imports
/// while planning a native rewrite.
pub fn discover_typescript_resolution_files(
    root: &Path,
    include_generated: bool,
) -> std::io::Result<Vec<PathBuf>> {
    discover_typescript_files_with_options(root, include_generated, true)
}

fn discover_typescript_files_with_options(
    root: &Path,
    include_generated: bool,
    include_declaration_files: bool,
) -> std::io::Result<Vec<PathBuf>> {
    discover_files(root, include_generated, include_declaration_files, false)
}

/// Recursively returns TypeScript sources and Vue single-file components for referrer analysis.
///
/// Vue files stay out of declaration scanning and rewrite discovery. Callers parse their script
/// blocks separately.
pub fn discover_analysis_files(
    root: &Path,
    include_generated: bool,
) -> std::io::Result<Vec<PathBuf>> {
    discover_files(root, include_generated, false, true)
}

fn discover_files(
    root: &Path,
    include_generated: bool,
    include_declaration_files: bool,
    include_vue: bool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    visit(
        root,
        &mut files,
        include_generated,
        include_declaration_files,
        include_vue,
        &mut HashSet::new(),
    )?;
    files.sort();
    Ok(files)
}

/// Keeps source paths included by the project and not matched by exclusions.
#[must_use]
pub fn filter_project_paths(
    paths: Vec<PathBuf>,
    root: &Path,
    includes: &[String],
    excludes: &[String],
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| {
            let relative = path
                .strip_prefix(root)
                .ok()
                .map(|value| value.to_string_lossy().replace('\\', "/"));
            relative.is_some_and(|value| {
                path_is_included(&value, includes) && !path_is_excluded(&value, excludes)
            })
        })
        .collect()
}

/// Returns whether a project-relative path matches a tsconfig include entry.
///
/// Literal patterns without a file extension include that directory and everything beneath it.
/// `src` and `src/` therefore include `src/app.ts`, while `src/app.ts` stays an exact file match.
#[must_use]
pub fn path_is_included(path: &str, includes: &[String]) -> bool {
    includes
        .iter()
        .any(|pattern| include_matches(path, pattern))
}

fn include_matches(path: &str, pattern: &str) -> bool {
    let directory = pattern.trim_end_matches('/');
    if matches(path, pattern) || (!directory.is_empty() && matches(path, directory)) {
        return true;
    }
    if pattern
        .chars()
        .any(|character| matches!(character, '*' | '?'))
        || directory.is_empty()
        || last_segment_has_extension(directory)
    {
        return false;
    }
    is_beneath_directory(path, directory)
}

fn is_beneath_directory(path: &str, directory: &str) -> bool {
    path.strip_prefix(directory)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn last_segment_has_extension(pattern: &str) -> bool {
    let segment = pattern.rsplit('/').next().unwrap_or(pattern);
    match segment.rsplit_once('.') {
        Some((name, extension)) => !name.is_empty() && !extension.is_empty(),
        None => false,
    }
}

/// Returns whether a project-relative path matches a tsconfig exclude entry.
///
/// Literal directory names exclude that directory and everything beneath it, matching TypeScript's
/// `exclude` behavior for patterns such as `dist` and `node_modules`.
#[must_use]
pub fn path_is_excluded(path: &str, excludes: &[String]) -> bool {
    excludes
        .iter()
        .any(|pattern| exclude_matches(path, pattern))
}

fn exclude_matches(path: &str, pattern: &str) -> bool {
    if matches(path, pattern) {
        return true;
    }
    if pattern
        .chars()
        .any(|character| matches!(character, '*' | '?'))
    {
        let directory_pattern = pattern.trim_end_matches('/');
        return path
            .match_indices('/')
            .any(|(index, _)| matches(&path[..index], directory_pattern));
    }
    let prefix = pattern.trim_end_matches('/');
    !prefix.is_empty() && (path == prefix || is_beneath_directory(path, prefix))
}

fn visit(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    include_generated: bool,
    include_declaration_files: bool,
    include_vue: bool,
    ancestors: &mut HashSet<PathBuf>,
) -> std::io::Result<()> {
    let real_directory = fs::canonicalize(directory)?;
    if !ancestors.insert(real_directory.clone()) {
        return Ok(());
    }
    let result = visit_entries(
        directory,
        files,
        include_generated,
        include_declaration_files,
        include_vue,
        ancestors,
    );
    ancestors.remove(&real_directory);
    result
}

fn visit_entries(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    include_generated: bool,
    include_declaration_files: bool,
    include_vue: bool,
    ancestors: &mut HashSet<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if ["node_modules", ".git"].contains(&name.to_string_lossy().as_ref())
                || (!include_generated
                    && ["generated", "__generated__"].contains(&name.to_string_lossy().as_ref()))
            {
                continue;
            }
            visit(
                &path,
                files,
                include_generated,
                include_declaration_files,
                include_vue,
                ancestors,
            )?;
        } else if is_discovered_file(&path, include_declaration_files, include_vue)
            && (include_generated || !is_generated_source(&path))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn is_discovered_file(path: &Path, include_declaration_files: bool, include_vue: bool) -> bool {
    match path.extension().and_then(|value| value.to_str()) {
        Some("vue") => include_vue,
        Some("ts" | "tsx") => {
            include_declaration_files || !path.to_string_lossy().ends_with(".d.ts")
        }
        _ => false,
    }
}

fn is_generated_source(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with(".generated.ts")
                || name.ends_with(".generated.tsx")
                || name.ends_with(".generated.vue")
        })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};
    #[cfg(unix)]
    use std::{os::unix::fs::symlink, path::Path};

    #[cfg(unix)]
    use super::read_typescript_source_text;
    use super::{
        discover_typescript_files, discover_typescript_files_with_generated,
        discover_typescript_resolution_files, filter_project_paths, path_is_excluded,
        path_is_included,
    };

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "type-tune-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    #[cfg(unix)]
    fn skips_dangling_symbolic_links_when_reading_source_text() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-dangling-symlink-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("creates fixture");
        let dangling = root.join("dangling.ts");
        symlink(root.join("missing-target.ts"), &dangling).expect("creates symlink");
        assert_eq!(
            read_typescript_source_text(Path::new(&dangling)).expect("reads"),
            None
        );
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    #[cfg(unix)]
    fn reports_read_errors_for_existing_symlink_targets() {
        let root = tempfile::tempdir().expect("creates fixture");
        let link = root.path().join("directory.ts");
        symlink(root.path(), &link).expect("creates symlink");
        assert!(read_typescript_source_text(&link).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn skips_directory_symlink_cycles() {
        let root = tempfile::tempdir().expect("creates fixture");
        let source = root.path().join("source.ts");
        fs::write(&source, "export type Shared = string;").expect("writes source");
        symlink(root.path(), root.path().join("loop")).expect("creates cycle");
        assert_eq!(
            discover_typescript_files(root.path()).expect("discovers"),
            [source]
        );
    }

    #[test]
    fn literal_exclude_patterns_cover_directories() {
        assert!(path_is_excluded("dist/src/shared.ts", &["dist".into()]));
        assert!(path_is_excluded("src/skip.ts", &["src/skip.ts".into()]));
        assert!(!path_is_excluded("src/shared.ts", &["src/skip.ts".into()]));
        assert!(!path_is_excluded("dist-old/app.ts", &["dist".into()]));
        let root = std::path::Path::new("/project");
        let paths = vec![root.join("src/shared.ts"), root.join("dist/src/shared.ts")];
        let filtered = filter_project_paths(paths, root, &["**/*.ts".into()], &["dist".into()]);
        assert_eq!(filtered, vec![root.join("src/shared.ts")]);
    }

    #[test]
    fn wildcard_excludes_cover_matching_directories() {
        assert!(path_is_excluded(
            "src/dist/nested/app.ts",
            &["**/dist".into()]
        ));
        assert!(path_is_excluded(
            "src/client/cache/app.ts",
            &["src/*/cache/".into()]
        ));
        assert!(!path_is_excluded(
            "src/dist-old/app.ts",
            &["**/dist".into()]
        ));
    }

    #[test]
    fn literal_include_patterns_cover_directories() {
        assert!(path_is_included("src/nested/app.ts", &["src".into()]));
        assert!(path_is_included("src/app.ts", &["src/".into()]));
        assert!(path_is_included("src/app.ts", &["src/app.ts".into()]));
        assert!(!path_is_included("src/app.ts", &["src/app".into()]));
        assert!(!path_is_included("scripts/tool.ts", &["src".into()]));
        assert!(!path_is_included("src-old/tool.ts", &["src".into()]));
        assert!(!path_is_included("src/nested/app.ts", &["src/*.ts".into()]));
        assert!(path_is_included("src/app.ts", &["src/*.ts".into()]));
    }

    #[test]
    fn includes_d_tsx_sources_but_skips_declaration_files() {
        let root = temporary_directory("d-tsx-source");
        fs::create_dir_all(root.join("src")).expect("creates fixture");
        fs::write(
            root.join("src/shared.d.tsx"),
            "export interface Shared {}\n",
        )
        .expect("writes TypeScript source");
        fs::write(
            root.join("src/globals.d.ts"),
            "declare const value: string;\n",
        )
        .expect("writes declaration file");

        assert_eq!(
            discover_typescript_files(&root).expect("discovers source files"),
            vec![root.join("src/shared.d.tsx")]
        );
        assert_eq!(
            discover_typescript_resolution_files(&root, false).expect("discovers resolution files"),
            vec![root.join("src/globals.d.ts"), root.join("src/shared.d.tsx")]
        );
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn generated_filter_matches_source_filenames() {
        let root = tempfile::tempdir().expect("creates fixture");
        let directory = root.path().join("archive.generated.backup");
        fs::create_dir(&directory).expect("creates directory");
        let ordinary = directory.join("schema.generated.extra.ts");
        let generated = directory.join("schema.generated.ts");
        fs::write(&ordinary, "").expect("writes source");
        fs::write(&generated, "").expect("writes generated source");
        assert_eq!(
            discover_typescript_files(root.path()).expect("discovers"),
            [ordinary.clone()]
        );
        assert_eq!(
            discover_typescript_files_with_generated(root.path(), true).expect("discovers"),
            [ordinary, generated]
        );
    }
}
