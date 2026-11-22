//! Shared command-line option parsing and validation.

use std::fs;

use type_tune_core::destination::PlacementStyle;

/// Returns whether a command invocation requests its usage text.
pub(crate) fn has_help_flag(arguments: &[String]) -> bool {
    has_flag(arguments, "--help") || has_flag(arguments, "-h")
}

/// Returns whether a command invocation includes an exact flag.
pub(crate) fn has_flag(arguments: &[String], flag: &str) -> bool {
    arguments.iter().any(|argument| argument == flag)
}

/// Returns the final value supplied for an option.
pub(crate) fn option_value<'a>(arguments: &'a [String], option: &str) -> Option<&'a str> {
    arguments
        .iter()
        .rposition(|value| value == option)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
}

/// Rejects unknown options and missing values before a command reads them.
pub(crate) fn validate_options(
    arguments: &[String],
    value_options: &[&str],
    flag_options: &[&str],
) -> Result<(), String> {
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if flag_options.contains(&argument.as_str()) {
            index += 1;
            continue;
        }
        if value_options.contains(&argument.as_str()) {
            let Some(value) = arguments.get(index + 1) else {
                return Err(format!("Option {argument} requires a value."));
            };
            if value.starts_with("--") {
                return Err(format!("Option {argument} requires a value."));
            }
            index += 2;
            continue;
        }
        return Err(format!("Unknown option: {argument}"));
    }
    Ok(())
}

/// Options accepted by one command.
///
/// Value options consume the following argument. Flag options do not.
pub(crate) struct CommandOptions {
    /// Options that require a following value.
    pub(crate) value_options: &'static [&'static str],
    /// Boolean options.
    pub(crate) flag_options: &'static [&'static str],
}

/// Options accepted by `scan`.
pub(crate) const SCAN_OPTIONS: CommandOptions = CommandOptions {
    value_options: &[
        "--project",
        "--format",
        "--min-referrers",
        "--referrer-count-exclude",
        "--placement-style",
        "--include",
        "--ignore-file",
    ],
    flag_options: &[
        "--include-existing-types",
        "--include-generated",
        "--compiler-resolution-bridge",
        "--verbose",
        "--explain",
    ],
};

/// Options accepted by `plan`, including optional AI review options.
pub(crate) const PLAN_OPTIONS: CommandOptions = CommandOptions {
    value_options: &[
        "--project",
        "--out",
        "--markdown",
        "--min-referrers",
        "--referrer-count-exclude",
        "--placement-style",
        "--include",
        "--ignore-file",
        "--default-status",
        "--ai-provider",
        "--ai-context",
        "--ai-request-out",
        "--ai-response-file",
        "--ai-endpoint",
        "--ai-model",
        "--ai-api-key-env",
        "--ai-timeout-ms",
        "--ai-on-error",
        "--ai-cache",
        "--ai-max-candidates",
        "--ai-max-context-bytes",
    ],
    flag_options: &[
        "--include-existing-types",
        "--include-generated",
        "--compiler-resolution-bridge",
        "--ai-review",
        "--ai-dry-run",
    ],
};

/// Options accepted by `review`.
pub(crate) const REVIEW_OPTIONS: CommandOptions = CommandOptions {
    value_options: &["--markdown"],
    flag_options: &[],
};

/// Options accepted by `apply`.
pub(crate) const APPLY_OPTIONS: CommandOptions = CommandOptions {
    value_options: &["--plan", "--markdown"],
    flag_options: &[
        "--accept-unreviewed",
        "--dry-run",
        "--compiler-resolution-bridge",
    ],
};

/// Options accepted by `extract`.
pub(crate) const EXTRACT_OPTIONS: CommandOptions = CommandOptions {
    value_options: &["--from", "--to", "--project", "--placement-style"],
    flag_options: &[
        "--dry-run",
        "--include-generated",
        "--compiler-resolution-bridge",
    ],
};

/// Parses the supported declaration destination placement styles.
pub(crate) fn parse_placement_style(value: Option<&str>) -> Result<PlacementStyle, String> {
    match value {
        None | Some("collocation") => Ok(PlacementStyle::Collocation),
        Some("repo-types") => Ok(PlacementStyle::RepoTypes),
        Some(value) => Err(format!(
            "Invalid --placement-style \"{value}\". Expected \"collocation\" or \"repo-types\"."
        )),
    }
}

/// Parses a positive provider request timeout in milliseconds.
pub(crate) fn parse_ai_timeout_ms(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("Invalid --ai-timeout-ms \"{value}\". Expected a positive integer."))
}

/// Validates the provider names supported by AI review.
pub(crate) fn validate_ai_provider(value: &str) -> Result<(), String> {
    if matches!(
        value,
        "mock" | "openai" | "openai-compatible" | "anthropic" | "ollama"
    ) {
        return Ok(());
    }
    Err(format!(
        "Invalid --ai-provider \"{value}\". Expected \"mock\" or \"openai\" or \"openai-compatible\" or \"anthropic\" or \"ollama\"."
    ))
}

/// Validates the explicit recovery behavior for AI review failures.
pub(crate) fn validate_ai_on_error(value: &str) -> Result<(), String> {
    if matches!(value, "fail" | "skip" | "mark-needs-review") {
        return Ok(());
    }
    Err(format!(
        "Invalid --ai-on-error \"{value}\". Expected \"fail\" or \"skip\" or \"mark-needs-review\"."
    ))
}

/// Returns the selected project config path, defaulting to the project root tsconfig.
pub(crate) fn scan_project_path(arguments: &[String]) -> &str {
    option_value(arguments, "--project").unwrap_or("tsconfig.json")
}

/// Parses the minimum referrer threshold with the CLI default.
pub(crate) fn parse_minimum_referrers(arguments: &[String]) -> Result<usize, String> {
    option_value(arguments, "--min-referrers")
        .map(|value| type_tune_core::options::parse_positive_integer(value, "--min-referrers"))
        .transpose()
        .map(|value| value.unwrap_or(type_tune_core::options::DEFAULT_MIN_REFERRERS))
        .map_err(|error| error.to_string())
}

/// Parses the referrer-count exclusion globs, defaulting to no exclusions.
pub(crate) fn parse_referrer_count_exclusions(arguments: &[String]) -> Result<Vec<String>, String> {
    option_value(arguments, "--referrer-count-exclude")
        .map(|value| {
            type_tune_core::options::parse_project_globs(value, "--referrer-count-exclude")
        })
        .transpose()
        .map(|value| value.unwrap_or_default())
        .map_err(|error| error.to_string())
}

/// Parses the status assigned to plan entries before review.
pub(crate) fn parse_default_status(arguments: &[String]) -> Result<String, String> {
    match option_value(arguments, "--default-status").unwrap_or("ignore") {
        "apply" | "ignore" => Ok(option_value(arguments, "--default-status")
            .unwrap_or("ignore")
            .into()),
        value => Err(format!(
            "Invalid --default-status \"{value}\". Expected \"apply\" or \"ignore\"."
        )),
    }
}

/// Parses an optional strictly positive integer option.
pub(crate) fn parse_optional_positive_integer(
    arguments: &[String],
    option: &str,
) -> Result<Option<usize>, String> {
    option_value(arguments, option)
        .map(|value| type_tune_core::options::parse_positive_integer(value, option))
        .transpose()
        .map_err(|error| error.to_string())
}

/// Parses the scan output format, defaulting to the human-readable text format.
pub(crate) fn parse_scan_format(arguments: &[String]) -> Result<&str, String> {
    match option_value(arguments, "--format").unwrap_or("text") {
        "text" | "json" => Ok(option_value(arguments, "--format").unwrap_or("text")),
        value => Err(format!(
            "Invalid --format \"{value}\". Expected \"text\" or \"json\"."
        )),
    }
}

/// Parses the first scan include-kind option, preserving the scan command's precedence rule.
pub(crate) fn parse_first_include_kinds(arguments: &[String]) -> Result<Vec<String>, String> {
    arguments
        .iter()
        .position(|value| value == "--include")
        .and_then(|index| arguments.get(index + 1))
        .map(|value| type_tune_core::options::parse_include_kinds(value))
        .transpose()
        .map(|value| value.unwrap_or_else(|| vec!["typeAlias".into(), "interface".into()]))
        .map_err(|error| error.to_string())
}

/// Reads ignore entries, skipping blank lines and comments.
pub(crate) fn read_ignore_entries(path: &str) -> Result<Vec<String>, String> {
    fs::read_to_string(path)
        .map(|content| parse_ignore_entries(&content))
        .map_err(|error| format!("Could not read {path}: {error}"))
}

fn parse_ignore_entries(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use type_tune_core::destination::PlacementStyle;

    use super::{
        has_flag, has_help_flag, option_value, parse_ai_timeout_ms, parse_default_status,
        parse_first_include_kinds, parse_ignore_entries, parse_minimum_referrers,
        parse_optional_positive_integer, parse_placement_style, parse_referrer_count_exclusions,
        parse_scan_format, scan_project_path, validate_ai_on_error, validate_ai_provider,
        validate_options,
    };

    #[test]
    fn recognizes_help_flags_without_matching_regular_arguments() {
        assert!(has_help_flag(&["--help".into()]));
        assert!(has_help_flag(&["-h".into()]));
        assert!(!has_help_flag(&[
            "--project".into(),
            "tsconfig.json".into()
        ]));
    }

    #[test]
    fn recognizes_exact_flags_only() {
        assert!(has_flag(&["--dry-run".into()], "--dry-run"));
        assert!(!has_flag(&["--dry-run=true".into()], "--dry-run"));
    }

    #[test]
    fn parses_ignore_entries_without_blank_lines_or_comments() {
        assert_eq!(
            parse_ignore_entries("\n# generated\nsrc/generated.ts\n  src/legacy.ts  \n"),
            ["src/generated.ts", "src/legacy.ts"]
        );
    }

    #[test]
    fn parses_supported_placement_styles() {
        assert_eq!(parse_placement_style(None), Ok(PlacementStyle::Collocation));
        assert_eq!(
            parse_placement_style(Some("repo-types")),
            Ok(PlacementStyle::RepoTypes)
        );
        assert!(parse_placement_style(Some("package-types")).is_err());
    }

    #[test]
    fn parses_shared_candidate_thresholds_and_exclusions() {
        assert_eq!(parse_minimum_referrers(&[]), Ok(2));
        assert_eq!(
            parse_minimum_referrers(&["--min-referrers".into(), "3".into()]),
            Ok(3)
        );
        assert_eq!(parse_referrer_count_exclusions(&[]), Ok(Vec::new()));
        assert_eq!(
            parse_referrer_count_exclusions(&[
                "--referrer-count-exclude".into(),
                "src/generated/**,src/legacy/**".into()
            ]),
            Ok(vec!["src/generated/**".into(), "src/legacy/**".into()])
        );
    }

    #[test]
    fn parses_default_plan_status() {
        assert_eq!(parse_default_status(&[]), Ok("ignore".into()));
        assert_eq!(
            parse_default_status(&["--default-status".into(), "apply".into()]),
            Ok("apply".into())
        );
        assert!(parse_default_status(&["--default-status".into(), "defer".into()]).is_err());
    }

    #[test]
    fn parses_optional_positive_integers() {
        assert_eq!(
            parse_optional_positive_integer(&[], "--ai-max-candidates"),
            Ok(None)
        );
        assert_eq!(
            parse_optional_positive_integer(
                &["--ai-max-candidates".into(), "12".into()],
                "--ai-max-candidates"
            ),
            Ok(Some(12))
        );
        assert!(
            parse_optional_positive_integer(
                &["--ai-max-candidates".into(), "0".into()],
                "--ai-max-candidates"
            )
            .is_err()
        );
    }

    #[test]
    fn parses_scan_output_formats() {
        assert_eq!(parse_scan_format(&[]), Ok("text"));
        assert_eq!(
            parse_scan_format(&["--format".into(), "json".into()]),
            Ok("json")
        );
        assert!(parse_scan_format(&["--format".into(), "yaml".into()]).is_err());
    }

    #[test]
    fn parses_the_first_scan_include_option() {
        assert_eq!(
            parse_first_include_kinds(&[]),
            Ok(vec!["typeAlias".into(), "interface".into()])
        );
        assert_eq!(
            parse_first_include_kinds(&[
                "--include".into(),
                "type".into(),
                "--include".into(),
                "interface".into()
            ]),
            Ok(vec!["typeAlias".into()])
        );
    }

    #[test]
    fn accepts_known_options_with_values_and_flags() {
        assert!(
            validate_options(
                &[
                    "--project".into(),
                    "tsconfig.json".into(),
                    "--verbose".into()
                ],
                &["--project"],
                &["--verbose"],
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_unknown_and_missing_option_values() {
        assert!(validate_options(&["--unknown".into()], &[], &[]).is_err());
        assert!(validate_options(&["--project".into()], &["--project"], &[]).is_err());
        assert!(
            validate_options(
                &["--project".into(), "--verbose".into()],
                &["--project"],
                &["--verbose"],
            )
            .is_err()
        );
    }

    #[test]
    fn uses_the_last_repeated_option_value() {
        let arguments = [
            "--min-referrers".into(),
            "1".into(),
            "--min-referrers".into(),
            "3".into(),
        ];
        assert_eq!(option_value(&arguments, "--min-referrers"), Some("3"));
    }

    #[test]
    fn parses_positive_ai_timeouts() {
        assert_eq!(parse_ai_timeout_ms("1"), Ok(1));
        assert_eq!(parse_ai_timeout_ms("60000"), Ok(60_000));
        assert!(parse_ai_timeout_ms("0").is_err());
        assert!(parse_ai_timeout_ms("1.5").is_err());
    }

    #[test]
    fn validates_ai_providers() {
        for provider in ["mock", "openai", "openai-compatible", "anthropic", "ollama"] {
            assert!(validate_ai_provider(provider).is_ok());
        }
        assert!(validate_ai_provider("custom").is_err());
    }

    #[test]
    fn validates_ai_error_modes() {
        for mode in ["fail", "skip", "mark-needs-review"] {
            assert!(validate_ai_on_error(mode).is_ok());
        }
        assert!(validate_ai_on_error("continue").is_err());
    }

    #[test]
    fn defaults_scan_to_the_project_tsconfig() {
        assert_eq!(scan_project_path(&[]), "tsconfig.json");
        assert_eq!(
            scan_project_path(&["--project".into(), "configs/tsconfig.json".into()]),
            "configs/tsconfig.json"
        );
    }

    #[test]
    fn documented_options_match_accepted_options_and_help() {
        let documented = documented_cli_options();
        assert_eq!(
            documented.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "ai-review",
                "apply",
                "extract",
                "plan",
                "review",
                "scan",
                "shared"
            ]
        );
        expect_options(
            &documented,
            &["shared", "scan"],
            &super::SCAN_OPTIONS,
            crate::help::scan_help(),
        );
        expect_options(
            &documented,
            &["shared", "plan", "ai-review"],
            &super::PLAN_OPTIONS,
            crate::help::plan_help(),
        );
        expect_options(
            &documented,
            &["review"],
            &super::REVIEW_OPTIONS,
            crate::help::review_help(),
        );
        expect_options(
            &documented,
            &["apply"],
            &super::APPLY_OPTIONS,
            crate::help::apply_help(),
        );
        expect_options(
            &documented,
            &["extract"],
            &super::EXTRACT_OPTIONS,
            crate::help::extract_help(),
        );
    }

    fn documented_cli_options()
    -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
        let source = include_str!("../../../docs/src/shared/config/cli-options.ts");
        let mut command = String::new();
        let mut options =
            std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
        for line in source.lines() {
            let line = line.trim();
            if let Some(value) = quoted_field(line, "command") {
                command = value;
            } else if let Some(value) = quoted_field(line, "name")
                && value.starts_with("--")
            {
                options.entry(command.clone()).or_default().insert(value);
            }
        }
        options
    }

    fn quoted_field(line: &str, field: &str) -> Option<String> {
        let rest = line.strip_prefix(&format!("{field}:"))?.trim();
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    fn expect_options(
        documented: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
        sections: &[&str],
        accepted: &super::CommandOptions,
        help: &str,
    ) {
        let mut expected = std::collections::BTreeSet::new();
        for section in sections {
            expected.extend(documented[*section].iter().cloned());
        }
        let actual = accepted
            .value_options
            .iter()
            .chain(accepted.flag_options)
            .map(|option| (*option).to_string())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert_eq!(help_options(help), expected);
    }

    fn help_options(help: &str) -> std::collections::BTreeSet<String> {
        let mut options = std::collections::BTreeSet::new();
        let bytes = help.as_bytes();
        let mut index = 0;
        while index + 2 < bytes.len() {
            if bytes[index] == b'-' && bytes[index + 1] == b'-' {
                let start = index;
                index += 2;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'-')
                {
                    index += 1;
                }
                if index > start + 2 {
                    options.insert(help[start..index].to_string());
                }
            } else {
                index += 1;
            }
        }
        options
    }
}
