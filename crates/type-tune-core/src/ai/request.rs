//! Construction and redaction of provider-neutral review context.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use regex::Regex;

use crate::{
    Result, TypeTuneError,
    hash::sha256,
    parser::{parse_named_imports, parse_type_declarations},
    plan::{PlanEntry, PlanFile},
};

use super::{
    AiReviewCandidateContext, AiReviewPolicy, AiReviewReferrerImportSnippet, AiReviewRequest,
    AiReviewSourceFileText, PROMPT_VERSION,
};

const CONTEXT_TRUNCATION_REASON: &str = "AI review source context exceeded maxContextBytes.";

/// Builds an AI request from a plan and project-relative sources.
///
/// # Errors
///
/// Returns an error when a candidate declaration cannot be recovered from its source.
pub fn build_request(
    plan: &PlanFile,
    sources: &BTreeMap<String, String>,
    context_mode: &str,
    max_candidates: Option<usize>,
    max_context_bytes: Option<usize>,
) -> Result<AiReviewRequest> {
    if !matches!(context_mode, "summary" | "snippets" | "full") {
        return Err(TypeTuneError::new(format!(
            "Invalid --ai-context \"{context_mode}\". Expected \"summary\" or \"snippets\" or \"full\"."
        )));
    }
    let entries = max_candidates.map_or_else(
        || plan.entries.as_slice(),
        |count| &plan.entries[..count.min(plan.entries.len())],
    );
    let mut request = AiReviewRequest {
        project_path: plan.project_path.clone(),
        placement_style: match plan.options.placement_style {
            crate::destination::PlacementStyle::Collocation => "collocation".into(),
            crate::destination::PlacementStyle::RepoTypes => "repo-types".into(),
        },
        candidates: entries
            .iter()
            .map(|entry| build_candidate_context(entry, sources, context_mode))
            .collect::<Result<Vec<_>>>()?,
        policy: AiReviewPolicy {
            context_mode: context_mode.into(),
            prompt_version: PROMPT_VERSION,
            max_context_bytes,
            context_truncated: None,
        },
    };
    if let Some(limit) = max_context_bytes {
        apply_context_limit(&mut request, limit);
    }
    Ok(request)
}

fn build_candidate_context(
    entry: &PlanEntry,
    sources: &BTreeMap<String, String>,
    context_mode: &str,
) -> Result<AiReviewCandidateContext> {
    let candidate = &entry.candidate;
    let mut context = AiReviewCandidateContext {
        candidate_id: candidate.id.clone(),
        symbol_name: candidate.symbol_name.clone(),
        kind: candidate.kind.clone(),
        source_path: candidate.source_path.clone(),
        destination_path: candidate.destination_path.clone(),
        referrer_paths: candidate.referrer_paths.clone(),
        referrer_count: candidate.referrer_count,
        counted_referrer_count: candidate.counted_referrer_count,
        local_dependency_names: candidate.local_dependency_names.clone(),
        referrer_strategies: candidate.referrer_strategies.clone(),
        readonly_referrers: candidate.readonly_referrers.clone(),
        vue_script_strategies: candidate.vue_script_strategies.clone(),
        vue_rewrite_blockers: candidate.vue_rewrite_blockers.clone(),
        analysis_limitations: candidate.analysis_limitations.clone(),
        source_fingerprint: entry.source_fingerprint.clone(),
        source_context_bytes: None,
        included_context_bytes: None,
        context_truncated: None,
        truncation_reason: None,
        declaration_text: None,
        local_dependency_texts: None,
        referrer_import_snippets: None,
        source_file_texts: None,
    };
    if context_mode == "summary" {
        return Ok(context);
    }
    let source = sources.get(&candidate.source_path).ok_or_else(|| {
        TypeTuneError::new(format!(
            "Could not find source for {}.",
            candidate.source_path
        ))
    })?;
    let declarations = parse_type_declarations(source)?;
    let by_key = declarations
        .iter()
        .map(|declaration| {
            let key = sha256(&declaration.stable_key(&candidate.source_path));
            (key, declaration)
        })
        .collect::<BTreeMap<_, _>>();
    context.declaration_text = Some(redact(
        &by_key
            .get(&candidate.declaration_key)
            .ok_or_else(|| {
                TypeTuneError::new(format!(
                    "Could not find declaration for key {} in {}.",
                    candidate.declaration_key, candidate.source_path
                ))
            })?
            .declaration_text,
    ));
    context.local_dependency_texts = Some(
        candidate
            .local_dependency_declaration_keys
            .iter()
            .map(|key| {
                by_key
                    .get(key)
                    .map(|declaration| redact(&declaration.declaration_text))
                    .ok_or_else(|| {
                        TypeTuneError::new(format!(
                            "Could not find declaration for key {key} in {}.",
                            candidate.source_path
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    context.referrer_import_snippets = Some(collect_import_snippets(candidate, sources)?);
    if context_mode == "full" {
        let mut seen_paths = BTreeSet::new();
        let paths = std::iter::once(&candidate.source_path)
            .chain(candidate.referrer_paths.iter())
            .filter(|path| seen_paths.insert(path.as_str()))
            .collect::<Vec<_>>();
        context.source_file_texts = Some(
            paths
                .into_iter()
                .filter_map(|path| {
                    sources.get(path).map(|text| AiReviewSourceFileText {
                        source_path: path.clone(),
                        text: redact(text),
                    })
                })
                .collect(),
        );
    }
    Ok(context)
}

fn collect_import_snippets(
    candidate: &crate::model::Candidate,
    sources: &BTreeMap<String, String>,
) -> Result<Vec<AiReviewReferrerImportSnippet>> {
    let mut snippets = Vec::new();
    for path in &candidate.referrer_paths {
        let Some(source) = sources.get(path) else {
            continue;
        };
        for import in parse_named_imports(source)?
            .into_iter()
            .filter(|import| import.imported_name == candidate.symbol_name)
        {
            snippets.push(AiReviewReferrerImportSnippet {
                referrer_path: path.clone(),
                import_text: redact(&source[import.statement_start..import.statement_end]),
            });
        }
    }
    Ok(snippets)
}

pub(super) fn redact(text: &str) -> String {
    static PROCESS_ENV: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^.*\bprocess\.env\b.*$").expect("valid regex"));
    static DOUBLE_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(\b[\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*\b\s*[:=]\s*)\"[^\"\n]*\""#).expect("valid regex")
    });
    static SINGLE_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(\b[\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*\b\s*[:=]\s*)'[^'\n]*'").expect("valid regex")
    });
    static TEMPLATE_VALUE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(\b[\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*\b\s*[:=]\s*)`[^`\n]*`").expect("valid regex")
    });
    static QUOTED_DOUBLE_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)([\"'][\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*[\"']\s*:\s*)\"[^\"\n]*\""#).expect("valid regex")
    });
    static QUOTED_SINGLE_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)([\"'][\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*[\"']\s*:\s*)'[^'\n]*'"#).expect("valid regex")
    });
    static QUOTED_TEMPLATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)([\"'][\w$-]*(?:api[_-]?key|token|secret|password|passwd|pwd|authorization|credential|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|session[_-]?secret)[\w$-]*[\"']\s*:\s*)`[^`\n]*`"#).expect("valid regex")
    });
    static AUTHORIZATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(authorization\s*[:=]\s*)([\"']?)bearer\s+[^\"'\s,;}]+([\"']?)"#)
            .expect("valid regex")
    });

    let text = PROCESS_ENV.replace_all(text, "[REDACTED: process.env reference]");
    let text = DOUBLE_VALUE.replace_all(&text, "$1\"[REDACTED]\"");
    let text = SINGLE_VALUE.replace_all(&text, "$1'[REDACTED]'");
    let text = TEMPLATE_VALUE.replace_all(&text, "$1`[REDACTED]`");
    let text = QUOTED_DOUBLE_KEY.replace_all(&text, "$1\"[REDACTED]\"");
    let text = QUOTED_SINGLE_KEY.replace_all(&text, "$1'[REDACTED]'");
    let text = QUOTED_TEMPLATE_KEY.replace_all(&text, "$1`[REDACTED]`");
    AUTHORIZATION
        .replace_all(&text, "$1$2Bearer [REDACTED]$3")
        .into_owned()
}

fn apply_context_limit(request: &mut AiReviewRequest, limit: usize) {
    let mut remaining = limit;
    let mut any_truncated = false;
    for candidate in &mut request.candidates {
        let original = context_bytes(candidate);
        let mut included = 0;
        let mut truncated = false;
        let mut take = |text: &mut String| {
            let available = remaining.saturating_sub(included);
            if text.len() > available {
                *text = utf8_prefix(text, available);
                truncated = true;
            }
            included += text.len();
        };
        if let Some(text) = &mut candidate.declaration_text {
            take(text);
        }
        if let Some(texts) = &mut candidate.local_dependency_texts {
            for text in texts {
                take(text);
            }
        }
        if let Some(snippets) = &mut candidate.referrer_import_snippets {
            for snippet in snippets {
                take(&mut snippet.import_text);
            }
        }
        if let Some(files) = &mut candidate.source_file_texts {
            for file in files {
                take(&mut file.text);
            }
        }
        remaining = remaining.saturating_sub(included);
        candidate.source_context_bytes = Some(original);
        candidate.included_context_bytes = Some(included);
        if truncated {
            candidate.context_truncated = Some(true);
            candidate.truncation_reason = Some(CONTEXT_TRUNCATION_REASON.into());
            any_truncated = true;
        }
    }
    if any_truncated {
        request.policy.context_truncated = Some(true);
    }
}

fn context_bytes(candidate: &AiReviewCandidateContext) -> usize {
    candidate.declaration_text.as_ref().map_or(0, String::len)
        + candidate
            .local_dependency_texts
            .as_ref()
            .map_or(0, |texts| texts.iter().map(String::len).sum())
        + candidate
            .referrer_import_snippets
            .as_ref()
            .map_or(0, |items| {
                items.iter().map(|item| item.import_text.len()).sum()
            })
        + candidate
            .source_file_texts
            .as_ref()
            .map_or(0, |items| items.iter().map(|item| item.text.len()).sum())
}

fn utf8_prefix(text: &str, max_bytes: usize) -> String {
    if max_bytes >= text.len() {
        return text.into();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}
