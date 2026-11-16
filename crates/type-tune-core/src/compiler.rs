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
        Err(CompilerFailure::Cleanup { message }) => Err(TypeTuneError::new(message)),
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
        Err(CompilerFailure::Cleanup { message }) => Err(message),
    }
}

enum CompilerFailure {
    Cleanup {
        message: String,
    },
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
    run_compiler(
        resolve_typescript_compiler(project_root),
        project_root,
        project_path,
    )
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
        Err(CompilerFailure::Cleanup { message }) => Err(message),
    }
}

fn run_vue_compiler(
    project_root: &Path,
    project_path: &Path,
) -> std::result::Result<(), CompilerFailure> {
    run_compiler(
        resolve_vue_compiler(project_root),
        project_root,
        project_path,
    )
}

// A fresh build-info file avoids both writes to the project's configured artifact and
// reuse of potentially stale incremental state. No other compiler side effects are sandboxed.
fn run_compiler(
    compiler: PathBuf,
    project_root: &Path,
    project_path: &Path,
) -> std::result::Result<(), CompilerFailure> {
    let temp = tempfile::Builder::new()
        .prefix("type-tune-compiler-")
        .tempdir()
        .map_err(|error| CompilerFailure::Cleanup {
            message: format!("Could not create temporary compiler validation directory: {error}"),
        })?;
    let result = inspect_build_info(&compiler, project_root, project_path, temp.path()).and_then(
        |needs_build_info| {
            let mut command = Command::new(&compiler);
            command
                .current_dir(project_root)
                .env("TMPDIR", temp.path())
                .env("TMP", temp.path())
                .env("TEMP", temp.path())
                .args(["--noEmit", "--project"])
                .arg(project_path);
            if needs_build_info {
                command
                    .arg("--tsBuildInfoFile")
                    .arg(temp.path().join("validation.tsbuildinfo"));
            }
            compiler_output(command, &compiler)
        },
    );
    if let Err(error) = temp.close() {
        let prior = match result {
            Ok(()) => String::new(),
            Err(CompilerFailure::Launch { compiler, error }) => {
                format!("Compiler {} could not start: {error}\n", compiler.display())
            }
            Err(CompilerFailure::Diagnostics { output, .. }) => format!("{output}\n"),
            Err(CompilerFailure::Cleanup { message }) => format!("{message}\n"),
        };
        return Err(CompilerFailure::Cleanup {
            message: format!(
                "{prior}Could not clean temporary compiler validation directory: {error}"
            ),
        });
    }
    result
}

// Ask the actual compiler to resolve inheritance, including package-based extends. If the
// effective options cannot be inspected, fail closed rather than running without isolation.
fn inspect_build_info(
    compiler: &Path,
    project_root: &Path,
    project_path: &Path,
    temp: &Path,
) -> std::result::Result<bool, CompilerFailure> {
    let output = Command::new(compiler)
        .current_dir(project_root)
        .env("TMPDIR", temp)
        .env("TMP", temp)
        .env("TEMP", temp)
        .args(["--showConfig", "--project"])
        .arg(project_path)
        .output()
        .map_err(|error| CompilerFailure::Launch {
            compiler: compiler.to_path_buf(),
            error,
        })?;
    if !output.status.success() {
        return Err(CompilerFailure::Diagnostics {
            status: output.status,
            output: diagnostics(&output),
        });
    }
    let config: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        CompilerFailure::Cleanup {
            message: format!("Could not inspect effective compiler configuration from {} --showConfig: {error}. Ensure the selected compiler supports --showConfig.", compiler.display()),
        }
    })?;
    let options = config.get("compilerOptions").and_then(serde_json::Value::as_object).ok_or_else(|| CompilerFailure::Cleanup {
        message: format!("Could not inspect effective compiler options from {} --showConfig; refusing unsafe validation.", compiler.display()),
    })?;
    let get_bool = |key: &str| -> std::result::Result<bool, CompilerFailure> {
        match options.get(key) {
            None => Ok(false),
            Some(value) => value.as_bool().ok_or_else(|| CompilerFailure::Cleanup {
                message: format!("Invalid effective compiler option {key} in --showConfig output; refusing unsafe validation."),
            }),
        }
    };
    let incremental = get_bool("incremental")?;
    let composite = get_bool("composite")?;
    Ok(incremental || composite)
}

fn compiler_output(
    mut command: Command,
    compiler: &Path,
) -> std::result::Result<(), CompilerFailure> {
    let output = command.output().map_err(|error| CompilerFailure::Launch {
        compiler: compiler.to_path_buf(),
        error,
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CompilerFailure::Diagnostics {
            status: output.status,
            output: diagnostics(&output),
        })
    }
}

fn diagnostics(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        stderr.trim().to_owned()
    }
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

    #[cfg(unix)]
    #[test]
    fn invalid_inspection_output_does_not_run_unisolated_validation() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let compiler = root.path().join("compiler");
        fs::write(&compiler, "#!/bin/sh\nif [ \"$1\" = --showConfig ]; then echo not-json; exit 0; fi\ntouch validation-ran\n").unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
        let result =
            super::run_compiler(compiler, root.path(), std::path::Path::new("tsconfig.json"));
        assert!(
            matches!(result, Err(super::CompilerFailure::Cleanup { message }) if message.contains("Could not inspect"))
        );
        assert!(!root.path().join("validation-ran").exists());
    }

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
