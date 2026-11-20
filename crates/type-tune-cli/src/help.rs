//! User-facing usage text for every CLI command.

/// Prints top-level command usage.
pub(crate) fn print_help() {
    println!(
        "type-tune {}\n\nUsage: type-tune <command>\n\nCommands:\n  scan    Find extractable TypeScript declarations\n  plan    Create a reviewable extraction plan\n  review  Interactively review plan markdown\n  apply   Apply an approved plan\n  extract Move one declaration directly",
        type_tune_core::version()
    );
}

/// Returns scan command usage.
pub(crate) fn scan_help() -> &'static str {
    "Usage: type-tune scan --project <tsconfig> [options]\n\nOptions:\n  --format <text|json>\n  --min-referrers <number>\n  --referrer-count-exclude <globs>\n  --placement-style <collocation|repo-types>\n  --include <type,interface>\n  --ignore-file <path>\n  --include-existing-types\n  --include-generated\n  --compiler-resolution-bridge\n  --verbose\n  --explain"
}

/// Prints scan command usage.
pub(crate) fn print_scan_help() {
    println!("{}", scan_help());
}

/// Returns plan command usage.
pub(crate) fn plan_help() -> &'static str {
    "Usage: type-tune plan [options]\n\nOptions:\n  --project <tsconfig>\n  --out <path>\n  --markdown <path>\n  --min-referrers <number>\n  --referrer-count-exclude <globs>\n  --placement-style <collocation|repo-types>\n  --include <type,interface>\n  --ignore-file <path>\n  --default-status <apply|ignore>\n  --include-existing-types\n  --include-generated\n  --compiler-resolution-bridge\n  --ai-review\n  --ai-dry-run\n  --ai-provider <mock|openai|openai-compatible|anthropic|ollama>\n  --ai-context <summary|snippets|full>\n  --ai-request-out <path>\n  --ai-response-file <path>\n  --ai-endpoint <url>\n  --ai-model <model>\n  --ai-api-key-env <name>\n  --ai-timeout-ms <number>\n  --ai-on-error <fail|skip|mark-needs-review>\n  --ai-cache <path>\n  --ai-max-candidates <number>\n  --ai-max-context-bytes <number>"
}

/// Prints plan command usage.
pub(crate) fn print_plan_help() {
    println!("{}", plan_help());
}

/// Returns review command usage.
pub(crate) fn review_help() -> &'static str {
    "Usage: type-tune review --markdown <path>\n\nOptions:\n  --markdown <path>"
}

/// Prints review command usage.
pub(crate) fn print_review_help() {
    println!("{}", review_help());
}

/// Returns apply command usage.
pub(crate) fn apply_help() -> &'static str {
    "Usage: type-tune apply --plan <path> (--markdown <path> | --accept-unreviewed) [options]\n\nOptions:\n  --plan <path>\n  --markdown <path>\n  --accept-unreviewed\n  --dry-run\n  --compiler-resolution-bridge"
}

/// Prints apply command usage.
pub(crate) fn print_apply_help() {
    println!("{}", apply_help());
}

/// Returns extract command usage.
pub(crate) fn extract_help() -> &'static str {
    "Usage: type-tune extract <symbol> --from <path> [options]\n\nOptions:\n  --from <path>\n  --to <path>\n  --project <tsconfig>\n  --placement-style <collocation|repo-types>\n  --dry-run\n  --include-generated\n  --compiler-resolution-bridge"
}

/// Prints extract command usage.
pub(crate) fn print_extract_help() {
    println!("{}", extract_help());
}
