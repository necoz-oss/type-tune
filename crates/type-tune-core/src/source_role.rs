//! Separates writable TypeScript sources from read-only referrer analysis.
//!
//! Rewrite planning calls [`reject_unsupported_rewrite`] before it builds edits. Vue single-file
//! components are read-only until a later phase can update their script blocks safely.

use std::collections::BTreeSet;

use crate::{
    Result, TypeTuneError,
    model::{ANALYSIS_LIMITATION_UNRESOLVED_VUE, Candidate, READONLY_REFERRER_VUE_SFC},
};

/// Whether the rewrite pipeline may edit a project file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceRole {
    /// TypeScript source the existing rewrite pipeline may edit.
    WritableTypeScript,
    /// Vue SFC analyzed for referrers and never edited in this phase.
    ReadOnlyVue,
}

/// Classifies a project-relative path.
///
/// This is the only file-kind decision the rewrite pipeline should consult. Callers store the
/// result on analyzed sources instead of repeating extension checks at each edit site.
#[must_use]
pub(crate) fn source_role(path: &str) -> SourceRole {
    if is_vue_sfc_path(path) {
        SourceRole::ReadOnlyVue
    } else {
        SourceRole::WritableTypeScript
    }
}

/// Returns whether `path` names a Vue single-file component.
#[must_use]
pub(crate) fn is_vue_sfc_path(path: &str) -> bool {
    file_name(path)
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension == "vue")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Rejects a rewrite that would edit a Vue file or ignore an unanalyzed Vue component.
///
/// # Errors
///
/// Returns an error before any rewrite edit is constructed when a selected candidate has a Vue
/// referrer, a Vue source path, a Vue strategy path, or an unresolved Vue analysis limitation.
pub(crate) fn reject_unsupported_rewrite(candidates: &[Candidate]) -> Result<()> {
    let mut messages = BTreeSet::new();
    for candidate in candidates {
        if source_role(&candidate.source_path) != SourceRole::WritableTypeScript {
            messages.insert(format!(
                "Cannot move {} from {} because Vue single-file components are not writable sources.",
                candidate.symbol_name, candidate.source_path
            ));
        }
        for referrer in &candidate.readonly_referrers {
            if referrer.reason == READONLY_REFERRER_VUE_SFC
                || source_role(&referrer.path) == SourceRole::ReadOnlyVue
            {
                messages.insert(format!(
                    "Cannot update read-only Vue referrer {} for {}. The component uses this type in a TypeScript script block, and Vue import rewriting is not supported yet.",
                    referrer.path, candidate.symbol_name
                ));
            } else {
                messages.insert(format!(
                    "Cannot update read-only referrer {} for {} ({}).",
                    referrer.path, candidate.symbol_name, referrer.reason
                ));
            }
        }
        for strategy in &candidate.referrer_strategies {
            if source_role(&strategy.referrer_path) != SourceRole::WritableTypeScript {
                messages.insert(format!(
                    "Cannot rewrite {} for {} because Vue import rewriting is not supported yet.",
                    strategy.referrer_path, candidate.symbol_name
                ));
            }
            if let crate::model::ImportStrategy::Barrel {
                barrel_file_path, ..
            } = &strategy.strategy
                && source_role(barrel_file_path) != SourceRole::WritableTypeScript
            {
                messages.insert(format!(
                    "Cannot rewrite Vue barrel {barrel_file_path} for {} because Vue import rewriting is not supported yet.",
                    candidate.symbol_name
                ));
            }
        }
        for limitation in &candidate.analysis_limitations {
            if limitation.reason == ANALYSIS_LIMITATION_UNRESOLVED_VUE {
                messages.insert(limitation.message.clone());
            } else {
                messages.insert(format!("{}: {}", limitation.path, limitation.message));
            }
        }
    }
    if messages.is_empty() {
        return Ok(());
    }
    let mut lines = vec![
        "Vue rewriting is not supported yet. Apply was rejected before any files were changed."
            .to_owned(),
    ];
    lines.extend(messages);
    lines.push(
        "Fix or exclude the Vue component and regenerate the plan. Framework-specific validation is still future work."
            .to_owned(),
    );
    Err(TypeTuneError::new(lines.join("\n")))
}
