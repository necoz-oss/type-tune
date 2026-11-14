//! Candidate destination and referrer strategy placement.

use crate::{
    destination::{PlacementStyle, resolve_destination},
    model::{Candidate, ImportStrategy, ReferrerStrategy},
    module_path::{module_specifier_preserving_alias, resolve_alias_module_specifier_from},
    module_specifier::{
        is_relative_module_specifier, preserved_module_specifier_extension,
        relative_module_specifier_with_extension,
    },
};
use std::collections::BTreeMap;

use super::destination_index_path;

/// Recomputes destinations for a requested placement style.
#[must_use]
pub fn apply_placement(
    candidates: Vec<Candidate>,
    placement_style: PlacementStyle,
) -> Vec<Candidate> {
    apply_placement_with_aliases(candidates, placement_style, None, &BTreeMap::new())
}

/// Recomputes destinations and import strategies using TypeScript path alias configuration.
#[must_use]
pub fn apply_placement_with_aliases(
    candidates: Vec<Candidate>,
    placement_style: PlacementStyle,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
) -> Vec<Candidate> {
    candidates
        .into_iter()
        .map(|candidate| {
            let destination_path = resolve_destination(&candidate.source_path, placement_style);
            if candidate.destination_path == destination_path {
                candidate
            } else {
                override_candidate_destination(candidate, destination_path, base_url, path_aliases)
            }
        })
        .collect()
}

/// Recomputes one candidate's referrer strategies for an explicit destination path.
#[must_use]
pub fn override_candidate_destination(
    mut candidate: Candidate,
    destination_path: String,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
) -> Candidate {
    candidate.destination_path = destination_path;
    candidate.referrer_strategies = candidate
        .referrer_strategies
        .into_iter()
        .map(|referrer| {
            retarget_referrer(
                referrer,
                &candidate.destination_path,
                base_url,
                path_aliases,
            )
        })
        .collect();
    candidate.vue_script_strategies = candidate
        .vue_script_strategies
        .into_iter()
        .map(|referrer| {
            retarget_referrer(
                referrer,
                &candidate.destination_path,
                base_url,
                path_aliases,
            )
        })
        .collect();
    candidate.destination_index_path = destination_index_path(&candidate.referrer_strategies);
    candidate
}

fn retarget_referrer(
    referrer: ReferrerStrategy,
    destination_path: &str,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
) -> ReferrerStrategy {
    let extension = match &referrer.strategy {
        ImportStrategy::Alias { module_specifier }
        | ImportStrategy::Relative { module_specifier } => {
            preserved_module_specifier_extension(module_specifier)
                .unwrap_or("")
                .to_owned()
        }
        ImportStrategy::Barrel {
            export_module_specifier,
            ..
        } => preserved_module_specifier_extension(export_module_specifier)
            .unwrap_or("")
            .to_owned(),
    };
    let strategy = match referrer.strategy {
        ImportStrategy::Barrel {
            barrel_file_path,
            export_module_specifier,
        } => {
            let export_module_specifier = if is_relative_module_specifier(&export_module_specifier)
            {
                relative_module_specifier_with_extension(
                    &barrel_file_path,
                    destination_path,
                    &extension,
                )
            } else {
                module_specifier_preserving_alias(
                    &barrel_file_path,
                    destination_path,
                    &extension,
                    &export_module_specifier,
                    base_url,
                    path_aliases,
                )
            };
            ImportStrategy::Barrel {
                export_module_specifier,
                barrel_file_path,
            }
        }
        ImportStrategy::Alias { module_specifier } => {
            let alias_extension =
                preserved_module_specifier_extension(&module_specifier).unwrap_or("");
            resolve_alias_module_specifier_from(
                destination_path,
                base_url,
                path_aliases,
                Some(&module_specifier),
            )
            .map(|module_specifier| ImportStrategy::Alias {
                module_specifier: format!("{module_specifier}{alias_extension}"),
            })
            .unwrap_or_else(|| ImportStrategy::Relative {
                module_specifier: relative_module_specifier_with_extension(
                    &referrer.referrer_path,
                    destination_path,
                    &extension,
                ),
            })
        }
        ImportStrategy::Relative { .. } => ImportStrategy::Relative {
            module_specifier: relative_module_specifier_with_extension(
                &referrer.referrer_path,
                destination_path,
                &extension,
            ),
        },
    };
    ReferrerStrategy {
        referrer_path: referrer.referrer_path,
        strategy,
    }
}
