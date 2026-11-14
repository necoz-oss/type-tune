//! Apply command support routines.

use std::{path::Path, process::ExitCode};

use crate::{
    commit::{commit_and_validate, report_commit_failure},
    help::print_apply_help,
    options::{APPLY_OPTIONS, has_flag, has_help_flag, option_value, validate_options},
    output::read_text_file,
    preview::{classify_dry_run_paths, preview_changes},
    project::{
        add_source_package_aliases, compiler_project, load_project_config, plan_extraction_rewrite,
        read_rewrite_sources,
    },
};

/// Loads an apply plan and optionally merges reviewed Markdown selections.
pub(crate) fn read_apply_plan(
    plan_path: &str,
    markdown_path: Option<&str>,
) -> Result<type_tune_core::plan::PlanFile, String> {
    let content = read_text_file(Path::new(plan_path))?;
    let plan = type_tune_core::plan::read_plan_json(&content).map_err(|error| error.to_string())?;
    match markdown_path {
        Some(markdown_path) => {
            let markdown = read_text_file(Path::new(markdown_path))?;
            type_tune_core::plan::merge_plan_markdown_state(&plan, &markdown)
                .map_err(|error| error.to_string())
        }
        None => Ok(plan),
    }
}

/// Re-scans a plan's project using the eligibility options captured in the plan.
pub(crate) fn scan_plan_candidates(
    plan: &type_tune_core::plan::PlanFile,
) -> type_tune_core::Result<Vec<type_tune_core::model::Candidate>> {
    let config = type_tune_core::config::load_project_config(Path::new(&plan.project_path))?;
    let candidates = type_tune_core::scan::scan_tsconfig_with_generated(
        Path::new(&plan.project_path),
        plan.options.include_generated,
    )?;
    let candidates = type_tune_core::scan::apply_placement_with_aliases(
        candidates,
        plan.options.placement_style,
        config.base_url.as_deref(),
        &config.paths,
    );
    let candidates = type_tune_core::scan::filter_kinds(candidates, &plan.options.include_kinds);
    let candidates = type_tune_core::scan::filter_existing_types(
        candidates,
        plan.options.include_existing_types,
    );
    let candidates = type_tune_core::scan::apply_referrer_count_exclusions(
        candidates,
        &plan.options.referrer_count_exclude_globs,
    );
    Ok(type_tune_core::scan::filter_min_referrers(
        candidates,
        plan.options.min_referrers,
    ))
}

/// Runs the apply command.
pub(crate) fn run_apply() -> ExitCode {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    if has_help_flag(&arguments) {
        print_apply_help();
        return ExitCode::SUCCESS;
    }
    if let Err(error) = validate_options(
        &arguments,
        APPLY_OPTIONS.value_options,
        APPLY_OPTIONS.flag_options,
    ) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let Some(plan_path) = option_value(&arguments, "--plan") else {
        eprintln!("apply requires --plan <path>");
        return ExitCode::from(2);
    };
    let dry_run = has_flag(&arguments, "--dry-run");
    let accept_unreviewed = has_flag(&arguments, "--accept-unreviewed");
    let markdown_path = option_value(&arguments, "--markdown");
    if markdown_path.is_none() && !accept_unreviewed {
        eprintln!("Apply requires --markdown <path> or explicit --accept-unreviewed.");
        return ExitCode::from(2);
    }
    let plan = match read_apply_plan(plan_path, markdown_path) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut config = match load_project_config(Path::new(&plan.project_path)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let planned_candidates = plan
        .entries
        .iter()
        .map(|entry| entry.candidate.clone())
        .collect::<Vec<_>>();
    if let Err(error) = type_tune_core::plan::validate_plan_candidate_path_boundaries(
        &config.project_root,
        &planned_candidates,
    ) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let current_candidates = match scan_plan_candidates(&plan) {
        Ok(candidates) => candidates,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) =
        type_tune_core::plan::validate_plan_candidate_metadata(&plan, &current_candidates)
    {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let candidates = plan
        .entries
        .iter()
        .map(|entry| entry.candidate.clone())
        .collect::<Vec<_>>();
    let sources = match read_rewrite_sources(Path::new(&plan.project_path), &candidates) {
        Ok(sources) => sources,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = add_source_package_aliases(&mut config, &sources) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = type_tune_core::plan::validate_plan_freshness(&plan, &sources) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let apply_entries = plan
        .entries
        .into_iter()
        .filter(|entry| entry.status == "apply")
        .collect::<Vec<_>>();
    let mut apply_candidates = Vec::with_capacity(apply_entries.len());
    for entry in apply_entries {
        let candidate = if let Some(destination) = entry.reviewed_destination {
            type_tune_core::scan::override_candidate_destination(
                entry.candidate,
                destination,
                config.base_url.as_deref(),
                &config.paths,
            )
        } else {
            entry.candidate
        };
        if candidate.destination_path == candidate.source_path {
            eprintln!(
                "Destination path must differ from source path: {}.",
                candidate.source_path
            );
            return ExitCode::FAILURE;
        }
        apply_candidates.push(candidate);
    }
    if let Err(error) = type_tune_core::plan::validate_plan_candidate_path_boundaries(
        &config.project_root,
        &apply_candidates,
    ) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    for candidate in &apply_candidates {
        let path = config.project_root.join(&candidate.destination_path);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                eprintln!(
                    "Destination already exists: {}.",
                    candidate.destination_path
                );
                return ExitCode::FAILURE;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!(
                    "Could not inspect destination {}: {error}",
                    candidate.destination_path
                );
                return ExitCode::FAILURE;
            }
        }
    }
    // Plan freshness uses every entry, including ignored candidates. Rewrite planning must not.
    // `plan_rewrite` rejects a Vue file that is not a script-strategy target of the candidates it
    // receives, so Vue files that belong only to ignored entries have to stay out of this map.
    let rewrite_sources =
        match read_rewrite_sources(Path::new(&plan.project_path), &apply_candidates) {
            Ok(sources) => sources,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
    let changes = match plan_extraction_rewrite(
        &config,
        &apply_candidates,
        &rewrite_sources,
        type_tune_core::rewrite::CompatibilityReexportQuoteStyle::Apply,
    ) {
        Ok(changes) => changes,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let ignored = candidates.len().saturating_sub(apply_candidates.len());
    let project_path = Path::new(&plan.project_path);
    let project_root = &config.project_root;
    let compiler_project = match compiler_project(project_path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if dry_run {
        if let Err(error) = preview_changes(
            project_root,
            &compiler_project,
            &changes,
            "Compile validation failed during apply dry run; rolled back file changes.",
        ) {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        let (changed_paths, created_paths, deleted_paths) =
            classify_dry_run_paths(project_root, &changes);
        print!(
            "{}",
            type_tune_core::render::render_apply_dry_run(
                apply_candidates.len(),
                ignored,
                &apply_candidates,
                &changed_paths,
                &created_paths,
                &deleted_paths,
            )
        );
        return ExitCode::SUCCESS;
    }
    if changes.is_empty() {
        println!("Applied 0 candidates; {ignored} ignored. No file changes.");
        return ExitCode::SUCCESS;
    }
    if let Err(error) = commit_and_validate(project_root, &compiler_project, &changes) {
        return report_commit_failure("apply", error);
    }
    println!(
        "Applied {} candidate{}; {ignored} ignored.",
        apply_candidates.len(),
        if apply_candidates.len() == 1 { "" } else { "s" }
    );
    ExitCode::SUCCESS
}
