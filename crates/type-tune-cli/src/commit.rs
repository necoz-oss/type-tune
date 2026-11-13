//! Transactional rewrite commits shared by mutating CLI commands.

use std::{collections::BTreeMap, fmt, path::Path, process::ExitCode};

/// A failure that occurs while committing and validating planned file changes.
#[derive(Debug)]
pub(crate) enum CommitError {
    /// The staged transaction could not be created.
    Stage(String),
    /// The staged transaction could not be committed.
    Commit(String),
    /// TypeScript validation failed after commit and the rollback may also have failed.
    Validation {
        /// TypeScript compiler diagnostics from the committed rewrite.
        diagnostics: String,
        /// A rollback failure, when restoration did not complete successfully.
        rollback: Option<String>,
    },
}

impl fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stage(error) | Self::Commit(error) => formatter.write_str(error),
            Self::Validation {
                diagnostics,
                rollback,
            } => {
                formatter.write_str(diagnostics)?;
                if let Some(rollback) = rollback {
                    write!(formatter, "\nNative rewrite rollback failed: {rollback}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for CommitError {}

/// Stages and commits rewrite changes before command-specific validation.
pub(crate) fn stage_and_commit(
    project_root: &Path,
    changes: &BTreeMap<String, String>,
) -> Result<type_tune_core::transaction::FileTransaction, CommitError> {
    let mut transaction =
        type_tune_core::transaction::stage_file_transaction(project_root, changes)
            .map_err(|error| CommitError::Stage(error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| CommitError::Commit(error.to_string()))?;
    Ok(transaction)
}

/// Commits changes, validates the TypeScript project, and rolls back validation failures.
pub(crate) fn commit_and_validate(
    project_root: &Path,
    compiler_project: &Path,
    changes: &BTreeMap<String, String>,
) -> Result<(), CommitError> {
    let mut transaction = stage_and_commit(project_root, changes)?;
    if let Err(diagnostics) = type_tune_core::compiler::validate_typescript_project_diagnostics(
        project_root,
        compiler_project,
    ) {
        return Err(CommitError::Validation {
            diagnostics,
            rollback: transaction.rollback().err().map(|error| error.to_string()),
        });
    }
    if let Err(diagnostics) = type_tune_core::compiler::validate_vue_script_edits(
        project_root,
        compiler_project,
        changes.keys(),
    ) {
        return Err(CommitError::Validation {
            diagnostics,
            rollback: transaction.rollback().err().map(|error| error.to_string()),
        });
    }
    Ok(())
}

/// Prints a command-specific failure after a transactional rewrite attempt.
pub(crate) fn report_commit_failure(command: &str, error: CommitError) -> ExitCode {
    match error {
        CommitError::Stage(error) | CommitError::Commit(error) => eprintln!("{error}"),
        CommitError::Validation {
            diagnostics,
            rollback,
        } => match rollback {
            Some(rollback) => {
                eprintln!("Compile validation failed after {command}.\n{diagnostics}");
                eprintln!("Native {command} rollback failed: {rollback}");
            }
            None => eprintln!(
                "Compile validation failed after {command}; rolled back file changes.\n{diagnostics}"
            ),
        },
    }
    ExitCode::FAILURE
}
