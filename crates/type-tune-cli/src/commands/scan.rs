//! Scan command implementation and support routines.

use std::{path::Path, process::ExitCode};

use type_tune_core::{
    destination::PlacementStyle,
    model::{AnalysisLimitation, Candidate, ScanDiagnostic},
};

use crate::{
    help::print_scan_help,
    options::{
        SCAN_OPTIONS, has_flag, has_help_flag, option_value, parse_first_include_kinds,
        parse_minimum_referrers, parse_placement_style, parse_referrer_count_exclusions,
        parse_scan_format, read_ignore_entries, scan_project_path, validate_options,
    },
    scan::{remove_unsupported_candidates, sort_diagnostics},
};

/// Runs the scan command.
pub(crate) fn run_scan() -> ExitCode {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    if has_help_flag(&arguments) {
        print_scan_help();
        return ExitCode::SUCCESS;
    }
    if let Err(error) = validate_options(
        &arguments,
        SCAN_OPTIONS.value_options,
        SCAN_OPTIONS.flag_options,
    ) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let format = match parse_scan_format(&arguments) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let verbose = has_flag(&arguments, "--verbose");
    let explain = has_flag(&arguments, "--explain");
    let ignore_entries = match option_value(&arguments, "--ignore-file") {
        Some(path) => match read_ignore_entries(path) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };
    let project = scan_project_path(&arguments);
    let minimum = match parse_minimum_referrers(&arguments) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let exclusions = match parse_referrer_count_exclusions(&arguments) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let include_existing_types = has_flag(&arguments, "--include-existing-types");
    let include_generated = has_flag(&arguments, "--include-generated");
    let placement_style = match parse_placement_style(option_value(&arguments, "--placement-style"))
    {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let include_kinds = match parse_first_include_kinds(&arguments) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    match scan_candidates(
        project,
        ScanCandidateOptions {
            minimum_referrers: minimum,
            exclusions: &exclusions,
            include_kinds: &include_kinds,
            ignore_entries: &ignore_entries,
            include_existing_types,
            include_generated,
            placement_style,
            explain,
        },
    ) {
        Ok(result) => match render_scan_result(result, format, verbose, explain) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Eligibility options used while scanning candidates.
pub(crate) struct ScanCandidateOptions<'a> {
    pub(crate) minimum_referrers: usize,
    pub(crate) exclusions: &'a [String],
    pub(crate) include_kinds: &'a [String],
    pub(crate) ignore_entries: &'a [String],
    pub(crate) include_existing_types: bool,
    pub(crate) include_generated: bool,
    pub(crate) placement_style: PlacementStyle,
    pub(crate) explain: bool,
}

/// Candidates and optional explain diagnostics produced by a scan.
pub(crate) struct ScanResult {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) diagnostics: Vec<ScanDiagnostic>,
    pub(crate) analysis_limitations: Vec<AnalysisLimitation>,
}

/// Renders a scan result in the selected CLI output format.
pub(crate) fn render_scan_result(
    result: ScanResult,
    format: &str,
    verbose: bool,
    explain: bool,
) -> Result<String, String> {
    if format == "text" {
        let mut output = type_tune_core::render::render_scan_text_with_diagnostics(
            &result.candidates,
            verbose,
            explain.then_some(result.diagnostics.as_slice()),
        );
        if !explain && !result.analysis_limitations.is_empty() {
            output.push_str("\nVue analysis limitations:\n");
            for limitation in &result.analysis_limitations {
                output.push_str(&format!("  - {}\n", limitation.message));
            }
        }
        return Ok(output);
    }
    serde_json::to_string_pretty(&type_tune_core::model::ScanOutput {
        candidates: result.candidates,
        diagnostics: explain.then_some(result.diagnostics),
        analysis_limitations: result.analysis_limitations,
    })
    .map(|output| format!("{output}\n"))
    .map_err(|error| format!("Could not serialize scan output: {error}"))
}

/// Scans a project and applies the command's eligibility rules.
pub(crate) fn scan_candidates(
    project: &str,
    options: ScanCandidateOptions<'_>,
) -> type_tune_core::Result<ScanResult> {
    let config = type_tune_core::config::load_project_config(Path::new(project))?;
    let filter = |candidates| {
        type_tune_core::scan::filter_candidates_with_diagnostics(
            candidates,
            type_tune_core::scan::ScanFilterOptions {
                minimum_referrers: options.minimum_referrers,
                referrer_count_exclusions: options.exclusions,
                include_kinds: options.include_kinds,
                ignore_entries: options.ignore_entries,
                include_existing_types: options.include_existing_types,
                placement_style: options.placement_style,
                base_url: config.base_url.as_deref(),
                path_aliases: &config.paths,
            },
        )
    };
    let (candidates, unsupported_diagnostics, analysis_limitations) = if options.explain {
        let (candidates, diagnostics) =
            type_tune_core::scan::scan_tsconfig_with_generated_and_diagnostics(
                Path::new(project),
                options.include_generated,
            )?;
        let limitations = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.reason == "unresolvedVueSfc")
            .map(|diagnostic| AnalysisLimitation {
                path: diagnostic.source_path.clone(),
                reason: diagnostic.reason.clone(),
                message: diagnostic.message.clone(),
            })
            .collect::<Vec<_>>();
        (candidates, diagnostics, limitations)
    } else {
        let scan = type_tune_core::scan::scan_tsconfig_project(
            Path::new(project),
            options.include_generated,
        )?;
        (scan.candidates, Vec::new(), scan.analysis_limitations)
    };
    let (mut candidates, mut diagnostics) = filter(candidates);
    if options.explain {
        let (filtered_candidates, mut unsupported_diagnostics) =
            remove_unsupported_candidates(candidates, unsupported_diagnostics);
        candidates = filtered_candidates;
        diagnostics.append(&mut unsupported_diagnostics);
        sort_diagnostics(&mut diagnostics);
    }
    Ok(ScanResult {
        candidates,
        diagnostics,
        analysis_limitations,
    })
}
