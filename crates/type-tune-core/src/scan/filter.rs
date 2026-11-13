//! Candidate eligibility and scan diagnostics.

use super::apply_placement_with_aliases;
use crate::{
    destination::PlacementStyle,
    glob::matches,
    model::{Candidate, ScanDiagnostic},
    sort::sort_candidates,
};
use std::collections::BTreeMap;

/// Options applied to candidates before rendering an explainable scan result.
pub struct ScanFilterOptions<'a> {
    /// Minimum counted referrer count required for extraction.
    pub minimum_referrers: usize,
    /// Referrer paths excluded from the eligibility count.
    pub referrer_count_exclusions: &'a [String],
    /// Declaration kinds included in the command output.
    pub include_kinds: &'a [String],
    /// Ignore-file entries matched against candidate identity fields.
    pub ignore_entries: &'a [String],
    /// Whether declarations in `types` directories or `types.ts` files are eligible.
    pub include_existing_types: bool,
    /// Destination placement strategy.
    pub placement_style: PlacementStyle,
    /// TypeScript base URL used to preserve alias referrer strategies.
    pub base_url: Option<&'a str>,
    /// TypeScript path aliases used to preserve alias referrer strategies.
    pub path_aliases: &'a BTreeMap<String, Vec<String>>,
}

/// Filters scan candidates and records reasons for command-level omissions.
#[must_use]
pub fn filter_candidates_with_diagnostics(
    candidates: Vec<Candidate>,
    options: ScanFilterOptions<'_>,
) -> (Vec<Candidate>, Vec<ScanDiagnostic>) {
    let detected_candidates = apply_placement_with_aliases(
        candidates,
        options.placement_style,
        options.base_url,
        options.path_aliases,
    );
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    for mut candidate in detected_candidates
        .into_iter()
        .filter(|candidate| options.include_kinds.contains(&candidate.kind))
    {
        if candidate.is_ignored(options.ignore_entries) {
            diagnostics.push(scan_diagnostic(
                &candidate,
                "ignored",
                "Declaration matched an ignore entry.",
            ));
            continue;
        }
        if !options.include_existing_types
            && let Some((reason, message)) = existing_type_location(&candidate)
        {
            diagnostics.push(scan_diagnostic(&candidate, reason, message));
            continue;
        }
        let counted_referrer_count = count_referrers(&candidate, options.referrer_count_exclusions);
        let excluded_referrer_count = candidate.referrer_count - counted_referrer_count;
        candidate.counted_referrer_count =
            (counted_referrer_count != candidate.referrer_count).then_some(counted_referrer_count);
        if counted_referrer_count < options.minimum_referrers {
            let message = if excluded_referrer_count > 0 {
                format!(
                    "Declaration has {counted_referrer_count} counted unique referrer file{} after excluding {excluded_referrer_count}, below --min-referrers {}.",
                    if counted_referrer_count == 1 { "" } else { "s" },
                    options.minimum_referrers,
                )
            } else {
                format!(
                    "Declaration has {} unique referrer file{}, below --min-referrers {}.",
                    candidate.referrer_count,
                    if candidate.referrer_count == 1 {
                        ""
                    } else {
                        "s"
                    },
                    options.minimum_referrers,
                )
            };
            let mut diagnostic = scan_diagnostic(&candidate, "insufficientReferrers", &message);
            diagnostic.referrer_count = Some(counted_referrer_count);
            diagnostic.min_referrers = Some(options.minimum_referrers);
            diagnostics.push(diagnostic);
            continue;
        }
        if candidate.destination_path == candidate.source_path {
            diagnostics.push(scan_diagnostic(
                &candidate,
                "noopDestination",
                "Resolved destination is the same as the source file.",
            ));
            continue;
        }
        candidates.push(candidate);
    }
    sort_candidates(&mut candidates);
    diagnostics.sort_by(|left, right| {
        [
            left.source_path.as_str(),
            left.symbol_name.as_str(),
            left.reason.as_str(),
        ]
        .cmp(&[
            right.source_path.as_str(),
            right.symbol_name.as_str(),
            right.reason.as_str(),
        ])
    });
    (candidates, diagnostics)
}

fn is_existing_type(candidate: &Candidate) -> bool {
    existing_type_location(candidate).is_some()
}

fn existing_type_location(candidate: &Candidate) -> Option<(&'static str, &'static str)> {
    let mut segments = candidate.source_path.rsplit('/');
    let basename = segments.next()?;
    if segments.any(|segment| segment == "types") {
        Some((
            "existingTypesDirectory",
            "Declaration is already under a types directory. Use --include-existing-types to include it.",
        ))
    } else if basename == "types.ts" {
        Some((
            "existingTypesFile",
            "Declaration is in a types.ts file, excluded by default as an existing type location. Use --include-existing-types to include it.",
        ))
    } else {
        None
    }
}

fn count_referrers(candidate: &Candidate, patterns: &[String]) -> usize {
    let mut paths = std::collections::BTreeSet::new();
    paths.extend(candidate.referrer_paths.iter().map(String::as_str));
    paths.extend(
        candidate
            .readonly_referrers
            .iter()
            .map(|referrer| referrer.path.as_str()),
    );
    paths
        .into_iter()
        .filter(|path| !patterns.iter().any(|pattern| matches(path, pattern)))
        .count()
}

fn scan_diagnostic(candidate: &Candidate, reason: &str, message: &str) -> ScanDiagnostic {
    ScanDiagnostic {
        severity: "info".into(),
        reason: reason.into(),
        message: message.into(),
        candidate_id: candidate.id.clone(),
        symbol_name: candidate.symbol_name.clone(),
        kind: candidate.kind.clone(),
        source_path: candidate.source_path.clone(),
        referrer_count: None,
        min_referrers: None,
    }
}

/// Keeps candidates meeting the configured minimum referrer count.
#[must_use]
pub fn filter_min_referrers(candidates: Vec<Candidate>, minimum: usize) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate
                .counted_referrer_count
                .unwrap_or(candidate.referrer_count)
                >= minimum
        })
        .collect()
}

/// Drops candidates whose destination path matches the source file.
#[must_use]
pub fn filter_noop_destinations(candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|candidate| candidate.destination_path != candidate.source_path)
        .collect()
}

/// Applies referrer-count exclusions while retaining every rewrite target path.
#[must_use]
pub fn apply_referrer_count_exclusions(
    mut candidates: Vec<Candidate>,
    patterns: &[String],
) -> Vec<Candidate> {
    for candidate in &mut candidates {
        let counted = count_referrers(candidate, patterns);
        candidate.counted_referrer_count = (counted != candidate.referrer_count).then_some(counted);
    }
    candidates
}

/// Removes declarations already in a `types` directory or a `types.ts` file.
#[must_use]
pub fn filter_existing_types(
    candidates: Vec<Candidate>,
    include_existing_types: bool,
) -> Vec<Candidate> {
    if include_existing_types {
        return candidates;
    }
    candidates
        .into_iter()
        .filter(|candidate| !is_existing_type(candidate))
        .collect()
}

/// Keeps candidates whose declaration kind was requested by the caller.
#[must_use]
pub fn filter_kinds(candidates: Vec<Candidate>, kinds: &[String]) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|candidate| kinds.contains(&candidate.kind))
        .collect()
}

/// Removes candidates matched by scan ignore-file entries.
#[must_use]
pub fn filter_ignored_candidates(
    candidates: Vec<Candidate>,
    ignore_entries: &[String],
) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|candidate| !candidate.is_ignored(ignore_entries))
        .collect()
}
