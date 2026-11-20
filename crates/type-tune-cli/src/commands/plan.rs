//! Plan command support routines.

use std::{path::Path, process::ExitCode};

use type_tune_core::{destination::PlacementStyle, model::Candidate};

use crate::{
    help::print_plan_help,
    options::{
        PLAN_OPTIONS, has_flag, has_help_flag, option_value, parse_ai_timeout_ms,
        parse_default_status, parse_minimum_referrers, parse_optional_positive_integer,
        parse_placement_style, parse_referrer_count_exclusions, read_ignore_entries,
        validate_ai_on_error, validate_ai_provider, validate_options,
    },
    output::write_text_file,
    project::{read_candidate_sources, read_rewrite_sources},
};

/// Eligibility options used while producing plan candidates.
pub(crate) struct PlanCandidateOptions<'a> {
    pub(crate) include_generated: bool,
    pub(crate) placement_style: PlacementStyle,
    pub(crate) include_kinds: &'a [String],
    pub(crate) include_existing_types: bool,
    pub(crate) exclusions: &'a [String],
    pub(crate) minimum_referrers: usize,
    pub(crate) ignore_entries: &'a [String],
    pub(crate) compiler_resolution_bridge: bool,
}

/// Scans and filters the candidates that can be written to a plan.
pub(crate) fn scan_plan_candidates(
    project: &str,
    options: PlanCandidateOptions<'_>,
) -> type_tune_core::Result<Vec<Candidate>> {
    let config = type_tune_core::config::load_project_config(Path::new(project))?;
    let resolution = if options.compiler_resolution_bridge {
        type_tune_core::compiler_bridge::ResolutionMode::CompilerBridge
    } else {
        type_tune_core::compiler_bridge::ResolutionMode::Native
    };
    let candidates = type_tune_core::scan::scan_tsconfig_with_generated_with_resolution(
        Path::new(project),
        options.include_generated,
        resolution,
    )?;
    let (candidates, _) = type_tune_core::scan::filter_candidates_with_diagnostics(
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
    );
    Ok(candidates)
}

/// Builds a persisted plan after reading its declaration sources.
pub(crate) fn build_plan(
    project: &str,
    candidates: Vec<Candidate>,
    options: type_tune_core::plan::PlanOptions,
) -> Result<type_tune_core::plan::PlanFile, String> {
    let sources = read_candidate_sources(Path::new(project), &candidates)?;
    type_tune_core::plan::build_plan(project.into(), candidates, &sources, options)
        .map_err(|error| error.to_string())
}

/// Builds the AI review request with all rewrite-relevant source files.
pub(crate) fn build_ai_review_request(
    project: &str,
    plan: &type_tune_core::plan::PlanFile,
    context: &str,
    max_candidates: Option<usize>,
    max_context_bytes: Option<usize>,
) -> Result<type_tune_core::ai::AiReviewRequest, String> {
    let candidates = plan
        .entries
        .iter()
        .map(|entry| entry.candidate.clone())
        .collect::<Vec<_>>();
    let sources = read_rewrite_sources(Path::new(project), &candidates)?;
    type_tune_core::ai::build_request(plan, &sources, context, max_candidates, max_context_bytes)
        .map_err(|error| error.to_string())
}

/// Writes the JSON plan and its review Markdown companion.
pub(crate) fn write_plan_outputs(
    plan: &type_tune_core::plan::PlanFile,
    out: &str,
    markdown: &str,
) -> Result<(), String> {
    let output = serde_json::to_string_pretty(plan)
        .map_err(|error| format!("Could not serialize plan: {error}"))?;
    write_text_file(Path::new(out), &format!("{output}\n"))?;
    write_text_file(
        Path::new(markdown),
        &type_tune_core::plan::render_plan_markdown(plan),
    )
}

/// Serializes an AI review request for dry-run output.
pub(crate) fn render_ai_review_request(
    request: &type_tune_core::ai::AiReviewRequest,
) -> Result<String, String> {
    serde_json::to_string_pretty(request)
        .map(|json| format!("{json}\n"))
        .map_err(|error| format!("Could not serialize AI review request: {error}"))
}

/// Builds provider settings and the stable model label persisted in a reviewed plan.
pub(crate) fn ai_review_provider_options(
    provider: &str,
    model: Option<&str>,
    endpoint: Option<&str>,
    api_key_env: Option<&str>,
    response_file: Option<&str>,
    timeout_ms: u64,
) -> (type_tune_core::ai::AiReviewProviderOptions, String) {
    let model = model.map(str::to_owned);
    let model_label = if provider == "mock" {
        "mock-model".into()
    } else {
        model.clone().unwrap_or_else(|| "unknown".into())
    };
    (
        type_tune_core::ai::AiReviewProviderOptions {
            provider: provider.into(),
            model,
            endpoint: endpoint.map(str::to_owned),
            api_key_env: api_key_env.map(str::to_owned),
            response_file: response_file.map(str::to_owned),
            timeout_ms,
        },
        model_label,
    )
}

/// Runs the plan command.
pub(crate) fn run_plan() -> ExitCode {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    if has_help_flag(&arguments) {
        print_plan_help();
        return ExitCode::SUCCESS;
    }
    if let Err(error) = validate_options(
        &arguments,
        PLAN_OPTIONS.value_options,
        PLAN_OPTIONS.flag_options,
    ) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let project = option_value(&arguments, "--project").unwrap_or("tsconfig.json");
    let out = option_value(&arguments, "--out").unwrap_or(".type-tune/plan.json");
    let markdown = option_value(&arguments, "--markdown").unwrap_or(".type-tune/plan.md");
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
    let include_kinds = match option_value(&arguments, "--include")
        .map(type_tune_core::options::parse_include_kinds)
        .transpose()
    {
        Ok(Some(value)) => value,
        Ok(None) => vec!["typeAlias".into(), "interface".into()],
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let placement_style = match parse_placement_style(option_value(&arguments, "--placement-style"))
    {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let include_existing_types = has_flag(&arguments, "--include-existing-types");
    let include_generated = has_flag(&arguments, "--include-generated");
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
    let default_status = match parse_default_status(&arguments) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let candidates = match scan_plan_candidates(
        project,
        PlanCandidateOptions {
            include_generated,
            placement_style,
            include_kinds: &include_kinds,
            include_existing_types,
            exclusions: &exclusions,
            minimum_referrers: minimum,
            ignore_entries: &ignore_entries,
            compiler_resolution_bridge: has_flag(&arguments, "--compiler-resolution-bridge"),
        },
    ) {
        Ok(candidates) => candidates,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut plan = match build_plan(
        project,
        candidates,
        type_tune_core::plan::PlanOptions {
            project_path: project.into(),
            min_referrers: minimum,
            referrer_count_exclude_globs: exclusions,
            placement_style,
            include_kinds,
            ignore_entries,
            default_status,
            include_existing_types,
            include_generated,
        },
    ) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let ai_review = has_flag(&arguments, "--ai-review");
    let ai_dry_run = has_flag(&arguments, "--ai-dry-run");
    if ai_dry_run && !ai_review {
        eprintln!("--ai-dry-run requires --ai-review.");
        return ExitCode::from(2);
    }
    if ai_review {
        let provider = option_value(&arguments, "--ai-provider").unwrap_or("mock");
        if let Err(error) = validate_ai_provider(provider) {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
        let on_error = option_value(&arguments, "--ai-on-error").unwrap_or("fail");
        if let Err(error) = validate_ai_on_error(on_error) {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
        let context = option_value(&arguments, "--ai-context").unwrap_or("snippets");
        let timeout_ms = match parse_ai_timeout_ms(
            option_value(&arguments, "--ai-timeout-ms").unwrap_or("60000"),
        ) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
        };
        let max_candidates =
            match parse_optional_positive_integer(&arguments, "--ai-max-candidates") {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(2);
                }
            };
        let max_context_bytes =
            match parse_optional_positive_integer(&arguments, "--ai-max-context-bytes") {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(2);
                }
            };
        let request = match build_ai_review_request(
            project,
            &plan,
            context,
            max_candidates,
            max_context_bytes,
        ) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        if ai_dry_run {
            let request_json = match render_ai_review_request(&request) {
                Ok(json) => json,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            };
            if let Some(path) = option_value(&arguments, "--ai-request-out") {
                let path = Path::new(path);
                if let Err(error) = write_text_file(path, &request_json) {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{request_json}");
            }
        } else {
            let (review_options, model_label) = ai_review_provider_options(
                provider,
                option_value(&arguments, "--ai-model"),
                option_value(&arguments, "--ai-endpoint"),
                option_value(&arguments, "--ai-api-key-env"),
                option_value(&arguments, "--ai-response-file"),
                timeout_ms,
            );
            let review = match option_value(&arguments, "--ai-cache") {
                Some(path) => type_tune_core::ai::review_with_cache(
                    &plan,
                    &request,
                    &review_options,
                    Path::new(path),
                ),
                None => type_tune_core::ai::review(&request, &review_options),
            };
            match review {
                Ok(decisions) => {
                    plan = type_tune_core::ai::merge_decisions(
                        &plan,
                        provider,
                        &model_label,
                        decisions,
                    )
                }
                Err(error) => match on_error {
                    "fail" => {
                        eprintln!("{error}");
                        return ExitCode::FAILURE;
                    }
                    "skip" => {}
                    "mark-needs-review" => {
                        plan = type_tune_core::ai::mark_error(
                            &plan,
                            provider,
                            &model_label,
                            &error.to_string(),
                        )
                    }
                    _ => unreachable!("AI error mode was validated before reviewing"),
                },
            }
        }
    }
    if let Err(error) = write_plan_outputs(&plan, out, markdown) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    println!(
        "Wrote {} candidate{} to {out} and {markdown}.",
        plan.entries.len(),
        if plan.entries.len() == 1 { "" } else { "s" }
    );
    println!("Run `type-tune review --markdown {markdown}` to choose entries.");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::ai_review_provider_options;

    #[test]
    fn labels_mock_reviews_without_a_user_model() {
        let (options, label) = ai_review_provider_options("mock", None, None, None, None, 60_000);

        assert_eq!(options.provider, "mock");
        assert_eq!(options.model, None);
        assert_eq!(label, "mock-model");
    }

    #[test]
    fn preserves_network_provider_configuration() {
        let (options, label) = ai_review_provider_options(
            "openai",
            Some("gpt-test"),
            Some("https://example.test/v1"),
            Some("OPENAI_API_KEY"),
            Some("response.json"),
            1_000,
        );

        assert_eq!(options.provider, "openai");
        assert_eq!(options.model.as_deref(), Some("gpt-test"));
        assert_eq!(options.endpoint.as_deref(), Some("https://example.test/v1"));
        assert_eq!(options.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert_eq!(options.response_file.as_deref(), Some("response.json"));
        assert_eq!(options.timeout_ms, 1_000);
        assert_eq!(label, "gpt-test");
    }
}
