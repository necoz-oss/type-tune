//! Temporary rewrite validation used by dry-run CLI commands.

use std::{collections::BTreeMap, fs, path::Path};

use crate::commit::stage_and_commit;

/// Validates changes against TypeScript and restores the original files.
pub(crate) fn preview_changes(
    project_root: &Path,
    compiler_project: &Path,
    changes: &BTreeMap<String, String>,
    compile_failure_message: &str,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut transaction =
        stage_and_commit(project_root, changes).map_err(|error| error.to_string())?;
    let validation = type_tune_core::compiler::validate_typescript_project_diagnostics(
        project_root,
        compiler_project,
    )
    .and_then(|()| {
        type_tune_core::compiler::validate_vue_script_edits(
            project_root,
            compiler_project,
            changes.keys(),
        )
    });
    let rollback = transaction.rollback();
    match (validation, rollback) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(diagnostics), Ok(())) => Err(format!("{compile_failure_message}\n{diagnostics}")),
        (Ok(()), Err(rollback)) => Err(format!("Native dry run rollback failed: {rollback}")),
        (Err(validation), Err(rollback)) => Err(format!(
            "{validation}\nNative dry run rollback failed: {rollback}"
        )),
    }
}

/// Groups dry-run changes according to their effect on the filesystem.
pub(crate) fn classify_dry_run_paths(
    project_root: &Path,
    changes: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut changed_paths = Vec::new();
    let mut created_paths = Vec::new();
    let mut deleted_paths = Vec::new();
    for (path, content) in changes {
        let absolute = project_root.join(path);
        if absolute.is_file() {
            let original = fs::read_to_string(&absolute).unwrap_or_default();
            if content.is_empty() {
                deleted_paths.push(path.clone());
            } else if original != *content {
                changed_paths.push(path.clone());
            }
        } else if !content.is_empty() {
            created_paths.push(path.clone());
        }
    }
    changed_paths.sort();
    created_paths.sort();
    deleted_paths.sort();
    (changed_paths, created_paths, deleted_paths)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::classify_dry_run_paths;

    #[test]
    fn classifies_changed_created_and_deleted_paths() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-preview-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).expect("creates fixture");
        fs::write(root.join("src/changed.ts"), "before\n").expect("writes changed file");
        fs::write(root.join("src/deleted.ts"), "before\n").expect("writes deleted file");
        let changes = BTreeMap::from([
            ("src/changed.ts".into(), "after\n".into()),
            ("src/created.ts".into(), "new\n".into()),
            ("src/deleted.ts".into(), String::new()),
            ("src/unchanged.ts".into(), String::new()),
        ]);

        let (changed, created, deleted) = classify_dry_run_paths(&root, &changes);

        assert_eq!(changed, ["src/changed.ts"]);
        assert_eq!(created, ["src/created.ts"]);
        assert_eq!(deleted, ["src/deleted.ts"]);
        fs::remove_dir_all(root).expect("removes fixture");
    }
}
