//! Project-relative POSIX path helpers.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Result, TypeTuneError,
    model::{Candidate, ImportStrategy},
};

const TYPESCRIPT_SOURCE_EXTENSIONS: [&str; 4] = [".ts", ".tsx", ".mts", ".cts"];

/// Validates a project-relative TypeScript source path from untrusted plan input.
///
/// # Errors
///
/// Returns an error when the path is empty, absolute, non-canonical, or uses traversal segments.
pub fn validate_project_relative_typescript_path(
    file_path: &str,
    field_path: &str,
) -> Result<String> {
    if file_path.is_empty()
        || file_path.contains('\0')
        || file_path.contains('\\')
        || file_path.starts_with('/')
        || is_windows_drive_path(file_path)
        || posix_normalize(file_path) != file_path
        || file_path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return Err(TypeTuneError::new(format!(
            "Invalid {field_path} \"{file_path}\". Expected a normalized project-relative POSIX path without \".\" or \"..\" segments."
        )));
    }

    let vue_analysis_path = field_path.contains("readonlyReferrers")
        || field_path.contains("analysisLimitations")
        || field_path.contains("vueScriptStrategies")
        || field_path.contains("vueRewriteBlockers");
    if crate::source_role::is_vue_sfc_path(file_path) && vue_analysis_path {
        return Ok(file_path.to_owned());
    }
    if is_typescript_declaration_file(file_path)
        || !TYPESCRIPT_SOURCE_EXTENSIONS.contains(&extension(file_path))
    {
        return Err(TypeTuneError::new(format!(
            "Invalid {field_path} \"{file_path}\". Expected a non-declaration TypeScript source path ending in .ts, .tsx, .mts, or .cts."
        )));
    }

    Ok(file_path.to_owned())
}

fn is_windows_drive_path(file_path: &str) -> bool {
    file_path.len() >= 2
        && file_path.as_bytes()[1] == b':'
        && file_path.as_bytes()[0].is_ascii_alphabetic()
}

fn posix_normalize(file_path: &str) -> String {
    let mut parts = Vec::new();
    for segment in file_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join("/")
    }
}

fn extension(file_path: &str) -> &str {
    file_path
        .rfind('.')
        .map(|index| file_path[index..].as_ref())
        .unwrap_or("")
}

fn is_typescript_declaration_file(file_path: &str) -> bool {
    file_path.ends_with(".d.ts") || file_path.ends_with(".d.mts") || file_path.ends_with(".d.cts")
}

/// Normalizes a project-relative path to POSIX separators.
///
/// # Errors
///
/// Returns an error for absolute, empty, NUL-containing, or traversal paths.
pub fn normalize_project_path(value: &str) -> Result<String> {
    if value.is_empty()
        || value.contains('\0')
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(TypeTuneError::new("Expected a safe project-relative path."));
    }

    let parts = value
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(TypeTuneError::new("Expected a safe project-relative path."));
    }
    let normalized = parts.join("/");
    if is_windows_drive_path(&normalized) {
        return Err(TypeTuneError::new("Expected a safe project-relative path."));
    }
    Ok(normalized)
}

/// Validates project-relative paths on a candidate before apply rewrites run.
///
/// # Errors
///
/// Returns an error when a path escapes the project root, including through symbolic links.
pub fn validate_candidate_path_boundaries(
    project_root: &Path,
    candidate: &Candidate,
    candidate_path: &str,
) -> Result<()> {
    visit_candidate_paths(candidate, candidate_path, |file_path, field_path| {
        validate_project_path_boundary(project_root, file_path, field_path)
    })
}

pub(crate) fn visit_candidate_paths(
    candidate: &Candidate,
    candidate_path: &str,
    mut visit: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    visit(
        &candidate.source_path,
        &format!("{candidate_path}.sourcePath"),
    )?;
    visit(
        &candidate.destination_path,
        &format!("{candidate_path}.destinationPath"),
    )?;
    if let Some(destination_index_path) = &candidate.destination_index_path {
        visit(
            destination_index_path,
            &format!("{candidate_path}.destinationIndexPath"),
        )?;
    }
    for (index, referrer_path) in candidate.referrer_paths.iter().enumerate() {
        visit(
            referrer_path,
            &format!("{candidate_path}.referrerPaths[{index}]"),
        )?;
    }
    for (index, referrer) in candidate.readonly_referrers.iter().enumerate() {
        visit(
            &referrer.path,
            &format!("{candidate_path}.readonlyReferrers[{index}].path"),
        )?;
    }
    for (index, referrer_strategy) in candidate.vue_script_strategies.iter().enumerate() {
        visit(
            &referrer_strategy.referrer_path,
            &format!("{candidate_path}.vueScriptStrategies[{index}].referrerPath"),
        )?;
    }
    for (index, blocker) in candidate.vue_rewrite_blockers.iter().enumerate() {
        visit(
            &blocker.path,
            &format!("{candidate_path}.vueRewriteBlockers[{index}].path"),
        )?;
    }
    for (index, limitation) in candidate.analysis_limitations.iter().enumerate() {
        visit(
            &limitation.path,
            &format!("{candidate_path}.analysisLimitations[{index}].path"),
        )?;
    }
    for (index, referrer_strategy) in candidate.referrer_strategies.iter().enumerate() {
        let strategy_path = format!("{candidate_path}.referrerStrategies[{index}]");
        visit(
            &referrer_strategy.referrer_path,
            &format!("{strategy_path}.referrerPath"),
        )?;
        if let ImportStrategy::Barrel {
            barrel_file_path, ..
        } = &referrer_strategy.strategy
        {
            visit(
                barrel_file_path,
                &format!("{strategy_path}.strategy.barrelFilePath"),
            )?;
        }
    }
    Ok(())
}

fn validate_project_path_boundary(
    project_root: &Path,
    file_path: &str,
    field_path: &str,
) -> Result<()> {
    validate_project_relative_typescript_path(file_path, field_path)?;
    let absolute_file_path = project_root.join(file_path);
    validate_absolute_project_path_boundary(project_root, &absolute_file_path, field_path)
}

fn validate_absolute_project_path_boundary(
    project_root: &Path,
    file_path: &Path,
    field_path: &str,
) -> Result<()> {
    let absolute_project_root = project_root
        .canonicalize()
        .map_err(|error| TypeTuneError::new(format!("Could not resolve project root: {error}")))?;
    let absolute_file_path = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    if !is_path_inside(&absolute_project_root, &absolute_file_path) {
        return Err(TypeTuneError::new(format!(
            "Invalid {field_path} \"{}\". Path escapes project root \"{}\".",
            file_path.display(),
            absolute_project_root.display(),
        )));
    }

    let real_project_root = fs::canonicalize(project_root)
        .map_err(|error| TypeTuneError::new(format!("Could not resolve project root: {error}")))?;
    let existing_ancestor = find_existing_ancestor(file_path)?;
    let real_existing_ancestor = match fs::canonicalize(&existing_ancestor) {
        Ok(path) => path,
        Err(_) => {
            return Err(TypeTuneError::new(format!(
                "Invalid {field_path} \"{}\". Path contains a symbolic link that cannot be resolved safely.",
                file_path.display(),
            )));
        }
    };
    if !is_path_inside(&real_project_root, &real_existing_ancestor) {
        return Err(TypeTuneError::new(format!(
            "Invalid {field_path} \"{}\". Path escapes project root through a symbolic link.",
            file_path.display(),
        )));
    }
    Ok(())
}

fn is_path_inside(root: &Path, candidate: &Path) -> bool {
    candidate
        .strip_prefix(root)
        .is_ok_and(|relative| relative != Path::new("..") && !relative.starts_with(".."))
}

fn find_existing_ancestor(file_path: &Path) -> Result<PathBuf> {
    let mut current = file_path.to_path_buf();
    while !path_entry_exists(&current) {
        let parent = current.parent().ok_or_else(|| {
            TypeTuneError::new(format!(
                "No existing ancestor found for path \"{}\".",
                file_path.display()
            ))
        })?;
        if parent == current {
            return Err(TypeTuneError::new(format!(
                "No existing ancestor found for path \"{}\".",
                file_path.display()
            )));
        }
        current = parent.to_path_buf();
    }
    Ok(current)
}

fn path_entry_exists(file_path: &Path) -> bool {
    fs::symlink_metadata(file_path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{normalize_project_path, validate_project_relative_typescript_path};

    #[test]
    fn rejects_traversal_in_plan_paths() {
        let error = validate_project_relative_typescript_path(
            "../outside.ts",
            "plan.entries[0].candidate.destinationPath",
        )
        .expect_err("rejects traversal");
        assert!(error.to_string().contains("destinationPath"));
        assert!(
            error
                .to_string()
                .contains("normalized project-relative POSIX path")
        );
    }

    #[test]
    fn normalizes_separators_and_dot_segments() {
        assert_eq!(
            normalize_project_path("src\\./domain//order.ts"),
            Ok("src/domain/order.ts".into())
        );
    }

    #[test]
    fn rejects_unsafe_paths() {
        for value in [
            "",
            "/src/order.ts",
            "src/../secret.ts",
            "src/\0order.ts",
            "C:/outside.ts",
            "C:outside.ts",
            "./c:\\outside.ts",
        ] {
            assert!(normalize_project_path(value).is_err(), "{value}");
        }
    }
}
