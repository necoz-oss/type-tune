//! Render reviewable plans and merge reviewed Markdown statuses.

use std::collections::BTreeMap;

use crate::{Result, TypeTuneError, model::ImportStrategy};

use super::PlanFile;

/// Merges reviewed candidate statuses from plan Markdown into a native plan.
///
/// # Errors
///
/// Returns an error when the Markdown has malformed, duplicate, unknown, or missing candidates.
pub fn merge_plan_markdown_state(plan: &PlanFile, markdown: &str) -> Result<PlanFile> {
    let mut reviewed_statuses = BTreeMap::<String, (String, Option<String>)>::new();
    let mut current_id: Option<String> = None;
    for line in markdown.lines() {
        if let Some((candidate_id, status)) = parse_markdown_candidate_status(line)? {
            if reviewed_statuses
                .insert(candidate_id.to_owned(), (status, None))
                .is_some()
            {
                return Err(TypeTuneError::new(format!(
                    "Duplicate candidate id \"{candidate_id}\" in plan markdown."
                )));
            }
            current_id = Some(candidate_id.to_owned());
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            current_id = None;
        }
        if let Some(rest) = line.strip_prefix("  - to:")
            && let Some(id) = &current_id
        {
            let value = rest.trim();
            let destination = value
                .strip_prefix('`')
                .and_then(|s| s.strip_suffix('`'))
                .filter(|s| !s.is_empty() && !s.contains('`'))
                .ok_or_else(|| {
                    TypeTuneError::new(format!(
                        "Malformed destination for candidate {id} in plan markdown."
                    ))
                })?;
            crate::path::validate_project_relative_typescript_path(
                destination,
                "reviewed destination",
            )?;
            let field = &mut reviewed_statuses
                .get_mut(id)
                .expect("current candidate exists")
                .1;
            if field.replace(destination.to_owned()).is_some() {
                return Err(TypeTuneError::new(format!(
                    "Duplicate destination for candidate {id} in plan markdown."
                )));
            }
        }
    }
    let expected_ids = plan
        .entries
        .iter()
        .map(|entry| entry.candidate.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if expected_ids.len() != plan.entries.len() {
        return Err(TypeTuneError::new(
            "Duplicate candidate id in plan entries.",
        ));
    }
    for candidate_id in reviewed_statuses.keys() {
        if !expected_ids.contains(candidate_id.as_str()) {
            return Err(TypeTuneError::new(format!(
                "Unknown candidate id \"{candidate_id}\" in plan markdown."
            )));
        }
    }
    for candidate_id in &expected_ids {
        if !reviewed_statuses.contains_key(*candidate_id) {
            return Err(TypeTuneError::new(format!(
                "Missing candidate id \"{candidate_id}\" in plan markdown."
            )));
        }
    }
    let mut reviewed_plan = plan.clone();
    for entry in &mut reviewed_plan.entries {
        let (status, destination) = reviewed_statuses
            .remove(&entry.candidate.id)
            .expect("validated reviewed status");
        entry.status = status;
        let destination = destination.ok_or_else(|| {
            TypeTuneError::new(format!(
                "Missing destination for candidate {} in plan markdown.",
                entry.candidate.id
            ))
        })?;
        entry.reviewed_destination =
            (destination != entry.candidate.destination_path).then_some(destination);
    }
    Ok(reviewed_plan)
}

fn parse_markdown_candidate_status(line: &str) -> Result<Option<(&str, String)>> {
    let line = line.trim_start();
    let Some(rest) = line.strip_prefix("- [") else {
        return Ok(None);
    };
    let Some((status, candidate)) = rest.split_once("] ") else {
        return Err(TypeTuneError::new(
            "Invalid candidate line in plan markdown.",
        ));
    };
    if !matches!(status, "apply" | "ignore") {
        return Err(TypeTuneError::new(format!(
            "Invalid plan markdown status \"{status}\". Expected \"apply\" or \"ignore\"."
        )));
    }
    let candidate_id = candidate.split_whitespace().next().unwrap_or_default();
    if candidate_id.is_empty() {
        return Err(TypeTuneError::new("Missing candidate id in plan markdown."));
    }
    Ok(Some((candidate_id, status.into())))
}

/// Renders a native plan as reviewable Markdown.
#[must_use]
pub fn render_plan_markdown(plan: &PlanFile) -> String {
    let mut lines = vec![
        "# type-tune plan".into(),
        String::new(),
        format!("Project: `{}`", plan.project_path),
        format!("Created: `{}`", plan.created_at),
        format!("Candidates: {}", plan.entries.len()),
        String::new(),
        "## Candidates".into(),
        String::new(),
    ];
    if plan.entries.is_empty() {
        lines.push("No candidates.".into());
        return format!("{}\n", lines.join("\n"));
    }
    for entry in &plan.entries {
        let candidate = &entry.candidate;
        lines.extend([
            format!(
                "- [{}] {} `{}`",
                entry.status, candidate.id, candidate.symbol_name
            ),
            format!("  - kind: `{}`", candidate.kind),
            format!("  - from: `{}`", candidate.source_path),
            format!("  - to: `{}`", candidate.destination_path),
            match candidate.counted_referrer_count {
                Some(counted) => format!(
                    "  - referrers: {} total, {counted} counted",
                    candidate.referrer_count
                ),
                None => format!("  - referrers: {}", candidate.referrer_count),
            },
            format!(
                "  - local dependencies: {}",
                if candidate.local_dependency_names.is_empty() {
                    "`none`".into()
                } else {
                    candidate
                        .local_dependency_names
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
            "  - strategies:".into(),
        ]);
        lines.extend(candidate.referrer_strategies.iter().map(|referrer| {
            format!(
                "    - `{}`: {}",
                referrer.referrer_path,
                render_strategy(&referrer.strategy)
            )
        }));
        if !candidate.readonly_referrers.is_empty() {
            lines.push("  - vue script referrers:".into());
            lines.extend(candidate.readonly_referrers.iter().map(|referrer| {
                if let Some(strategy) = candidate
                    .vue_script_strategies
                    .iter()
                    .find(|strategy| strategy.referrer_path == referrer.path)
                {
                    format!(
                        "    - `{}`: {}",
                        referrer.path,
                        render_strategy(&strategy.strategy)
                    )
                } else if let Some(blocker) = candidate
                    .vue_rewrite_blockers
                    .iter()
                    .find(|blocker| blocker.path == referrer.path)
                {
                    format!("    - `{}`: {}", referrer.path, blocker.message)
                } else {
                    format!("    - `{}`: Vue script usage", referrer.path)
                }
            }));
        }
        if !candidate.analysis_limitations.is_empty() {
            lines.push("  - analysis limitations:".into());
            lines.extend(
                candidate
                    .analysis_limitations
                    .iter()
                    .map(|limitation| format!("    - {}", limitation.message)),
            );
        }
        if let Some(review) = &entry.ai_review {
            lines.extend([
                format!(
                    "  - AI: `{}`, confidence: {}",
                    review.status, review.confidence
                ),
                format!("  - AI reason: {}", review.reason),
                format!(
                    "  - AI concerns: {}",
                    if review.concerns.is_empty() {
                        "`none`".into()
                    } else {
                        review.concerns.join("; ")
                    }
                ),
            ]);
        }
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n").trim_end())
}

fn render_strategy(strategy: &ImportStrategy) -> String {
    match strategy {
        ImportStrategy::Alias { module_specifier } => {
            format!("`alias` import `{module_specifier}`")
        }
        ImportStrategy::Barrel {
            barrel_file_path,
            export_module_specifier,
        } => format!("`barrel` via `{barrel_file_path}` export `{export_module_specifier}`"),
        ImportStrategy::Relative { module_specifier } => {
            format!("`relative` import `{module_specifier}`")
        }
    }
}
