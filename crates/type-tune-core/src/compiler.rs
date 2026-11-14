//! TypeScript compiler validation for native rewrites.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::{Result, TypeTuneError};

/// Validates one TypeScript project after a native rewrite.
///
/// A project-local `node_modules/.bin/tsc` is preferred so validation uses the project's pinned
/// TypeScript version. When unavailable, `tsc` is resolved through `PATH`.
///
/// # Errors
///
/// Returns an error when TypeScript cannot be started or reports compilation failures.
pub fn validate_typescript_project(project_root: &Path, project_path: &Path) -> Result<()> {
    match run_typescript_compiler(project_root, project_path) {
        Ok(()) => Ok(()),
        Err(CompilerFailure::Launch { compiler, error }) => Err(TypeTuneError::new(format!(
            "Could not run TypeScript compiler {}: {error}",
            compiler.display()
        ))),
        Err(CompilerFailure::Diagnostics { status, output }) => Err(TypeTuneError::new(format!(
            "TypeScript compiler validation failed ({}): {}",
            status, output
        ))),
    }
}

/// Returns TypeScript compiler diagnostics when validation fails.
///
/// # Errors
///
/// Returns launch failures or trimmed compiler diagnostics without an additional wrapper message.
pub fn validate_typescript_project_diagnostics(
    project_root: &Path,
    project_path: &Path,
) -> std::result::Result<(), String> {
    match run_typescript_compiler(project_root, project_path) {
        Ok(()) => Ok(()),
        Err(CompilerFailure::Launch { compiler, error }) => Err(format!(
            "Could not run TypeScript compiler {}: {error}",
            compiler.display()
        )),
        Err(CompilerFailure::Diagnostics { output, .. }) => Err(output),
    }
}

enum CompilerFailure {
    Launch {
        compiler: PathBuf,
        error: std::io::Error,
    },
    Diagnostics {
        status: std::process::ExitStatus,
        output: String,
    },
}

fn run_typescript_compiler(
    project_root: &Path,
    project_path: &Path,
) -> std::result::Result<(), CompilerFailure> {
    let compiler = resolve_typescript_compiler(project_root);
    let output = Command::new(&compiler)
        .current_dir(project_root)
        .args(["--noEmit", "--project"])
        .arg(project_path)
        .output()
        .map_err(|error| CompilerFailure::Launch {
            compiler: compiler.clone(),
            error,
        })?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let diagnostics = if diagnostics.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        diagnostics.into_owned()
    };
    Err(CompilerFailure::Diagnostics {
        status: output.status,
        output: diagnostics.trim().to_string(),
    })
}

/// Validates Vue script edits with a project-local `vue-tsc` when a rewrite changes `.vue` files.
///
/// `tsc` does not typecheck single-file components. Supported script specifier edits therefore
/// require `vue-tsc`. When no `.vue` path changed, this check does nothing. A missing compiler or
/// a failed typecheck is a validation error so the caller can roll the transaction back.
///
/// # Errors
///
/// Returns an error when `vue-tsc` cannot be started or reports diagnostics.
pub fn validate_vue_script_edits(
    project_root: &Path,
    project_path: &Path,
    changed_paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> std::result::Result<(), String> {
    let touches_vue = changed_paths
        .into_iter()
        .any(|path| crate::source_role::is_vue_sfc_path(path.as_ref()));
    if !touches_vue {
        return Ok(());
    }
    match run_vue_compiler(project_root, project_path) {
        Ok(()) => Ok(()),
        Err(CompilerFailure::Launch { compiler, error }) => Err(format!(
            "Vue script edits require vue-tsc, but {} could not be started: {error}. Install vue-tsc to validate .vue files.",
            compiler.display()
        )),
        Err(CompilerFailure::Diagnostics { output, .. }) => {
            Err(format!("vue-tsc validation failed: {output}"))
        }
    }
}

fn run_vue_compiler(
    project_root: &Path,
    project_path: &Path,
) -> std::result::Result<(), CompilerFailure> {
    let compiler = resolve_vue_compiler(project_root);
    let output = Command::new(&compiler)
        .current_dir(project_root)
        .args(["--noEmit", "--project"])
        .arg(project_path)
        .output()
        .map_err(|error| CompilerFailure::Launch {
            compiler: compiler.clone(),
            error,
        })?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let diagnostics = if diagnostics.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        diagnostics.into_owned()
    };
    Err(CompilerFailure::Diagnostics {
        status: output.status,
        output: diagnostics.trim().to_string(),
    })
}

fn resolve_vue_compiler(project_root: &Path) -> PathBuf {
    let mut directory = Some(project_root);
    while let Some(current) = directory {
        let local = current
            .join("node_modules/.bin")
            .join(vue_tsc_binary_name());
        if local.is_file() {
            return local;
        }
        directory = current.parent();
    }
    PathBuf::from(vue_tsc_binary_name())
}

fn vue_tsc_binary_name() -> &'static str {
    if cfg!(windows) {
        "vue-tsc.cmd"
    } else {
        "vue-tsc"
    }
}

/// Resolves the compiler executable preferred for a project.
#[must_use]
pub fn resolve_typescript_compiler(project_root: &Path) -> PathBuf {
    let mut directory = Some(project_root);
    while let Some(current) = directory {
        let local = current.join("node_modules/.bin").join(tsc_binary_name());
        if local.is_file() {
            return local;
        }
        directory = current.parent();
    }
    PathBuf::from(tsc_binary_name())
}

fn tsc_binary_name() -> &'static str {
    if cfg!(windows) { "tsc.cmd" } else { "tsc" }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{resolve_typescript_compiler, tsc_binary_name};

    #[test]
    fn prefers_a_project_local_typescript_compiler() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-compiler-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let compiler = root.join("node_modules/.bin").join(tsc_binary_name());
        fs::create_dir_all(compiler.parent().expect("parent")).expect("creates fixture");
        fs::write(&compiler, "placeholder").expect("writes fixture");
        assert_eq!(resolve_typescript_compiler(&root), compiler);
        fs::remove_dir_all(root).expect("removes fixture");
    }

    #[test]
    fn falls_back_to_path_compiler() {
        let root = std::env::temp_dir().join("type-tune-missing-compiler");
        assert_eq!(
            resolve_typescript_compiler(&root),
            std::path::PathBuf::from(tsc_binary_name())
        );
    }

    #[test]
    fn finds_a_compiler_in_a_parent_workspace() {
        let root = std::env::temp_dir().join(format!(
            "type-tune-parent-compiler-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let compiler = root.join("node_modules/.bin").join(tsc_binary_name());
        let project = root.join("packages/app");
        fs::create_dir_all(&project).expect("creates project");
        fs::create_dir_all(compiler.parent().expect("parent")).expect("creates compiler path");
        fs::write(&compiler, "placeholder").expect("writes fixture");
        assert_eq!(resolve_typescript_compiler(&project), compiler);
        fs::remove_dir_all(root).expect("removes fixture");
    }
}
