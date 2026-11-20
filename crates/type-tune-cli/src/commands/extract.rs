//! Extract command implementation.

use std::{path::Path, process::ExitCode};

use crate::{
    commit::{commit_and_validate, report_commit_failure},
    help::print_extract_help,
    options::{
        EXTRACT_OPTIONS, has_flag, has_help_flag, option_value, parse_placement_style,
        validate_options,
    },
    preview::{classify_dry_run_paths, preview_changes},
    project::{
        add_source_package_aliases, compiler_project, load_project_config,
        normalize_project_input_path, plan_extraction_rewrite, read_rewrite_sources,
    },
};

/// Runs the extraction command.
pub(crate) fn run_extract() -> ExitCode {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    if has_help_flag(&arguments) {
        print_extract_help();
        return ExitCode::SUCCESS;
    }
    let Some(symbol_name) = arguments.first() else {
        eprintln!("extract requires <symbol>");
        return ExitCode::from(2);
    };
    let options = &arguments[1..];
    if let Err(error) = validate_options(
        options,
        EXTRACT_OPTIONS.value_options,
        EXTRACT_OPTIONS.flag_options,
    ) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let Some(source_input) = option_value(options, "--from") else {
        eprintln!("extract requires --from <path>");
        return ExitCode::from(2);
    };
    let project = option_value(options, "--project").unwrap_or("tsconfig.json");
    let placement_style = match parse_placement_style(option_value(options, "--placement-style")) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let mut config = match load_project_config(Path::new(project)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let source_path = match normalize_project_input_path(&config.project_root, source_input) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let destination_path = match option_value(options, "--to") {
        Some(path) => match normalize_project_input_path(&config.project_root, path) {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let resolution = if has_flag(options, "--compiler-resolution-bridge") {
        type_tune_core::compiler_bridge::ResolutionMode::CompilerBridge
    } else {
        type_tune_core::compiler_bridge::ResolutionMode::Native
    };
    let candidates = match type_tune_core::scan::scan_tsconfig_with_generated_with_resolution(
        Path::new(project),
        has_flag(options, "--include-generated"),
        resolution,
    ) {
        Ok(candidates) => type_tune_core::scan::filter_existing_types(
            type_tune_core::scan::apply_placement_with_aliases(
                candidates,
                placement_style,
                config.base_url.as_deref(),
                &config.paths,
            ),
            true,
        ),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let matches = candidates
        .into_iter()
        .filter(|candidate| {
            candidate.symbol_name == *symbol_name && candidate.source_path == source_path
        })
        .collect::<Vec<_>>();
    let [candidate] = matches.as_slice() else {
        if matches.is_empty() {
            eprintln!("No extractable candidate found for {symbol_name} from {source_path}.");
        } else {
            eprintln!(
                "Multiple extractable candidates found for {symbol_name} from {source_path}."
            );
        }
        return ExitCode::FAILURE;
    };
    let candidate = destination_path.map_or_else(
        || candidate.clone(),
        |destination_path| {
            type_tune_core::scan::override_candidate_destination(
                candidate.clone(),
                destination_path,
                config.base_url.as_deref(),
                &config.paths,
            )
        },
    );
    if candidate.destination_path == candidate.source_path {
        eprintln!(
            "Destination path must differ from source path: {}.",
            candidate.source_path
        );
        return ExitCode::FAILURE;
    }
    if let Err(error) = type_tune_core::plan::validate_plan_candidate_path_boundaries(
        &config.project_root,
        std::slice::from_ref(&candidate),
    ) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let sources = match read_rewrite_sources(Path::new(project), std::slice::from_ref(&candidate)) {
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
    let changes = match plan_extraction_rewrite(
        &config,
        std::slice::from_ref(&candidate),
        &sources,
        type_tune_core::rewrite::CompatibilityReexportQuoteStyle::Extract,
    ) {
        Ok(changes) => changes,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let compiler_project = match compiler_project(Path::new(project)) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if has_flag(options, "--dry-run") {
        if let Err(error) = preview_changes(
            &config.project_root,
            &compiler_project,
            &changes,
            "Compile validation failed during extract dry run; rolled back file changes.",
        ) {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        let (changed_paths, created_paths, deleted_paths) =
            classify_dry_run_paths(&config.project_root, &changes);
        print!(
            "{}",
            type_tune_core::render::render_extract_dry_run(
                &candidate.symbol_name,
                &candidate.source_path,
                &candidate.destination_path,
                &changed_paths,
                &created_paths,
                &deleted_paths,
            )
        );
        return ExitCode::SUCCESS;
    }
    if let Err(error) = commit_and_validate(&config.project_root, &compiler_project, &changes) {
        return report_commit_failure("extract", error);
    }
    println!(
        "Extracted {} from {} to {}.",
        candidate.symbol_name, candidate.source_path, candidate.destination_path
    );
    ExitCode::SUCCESS
}
