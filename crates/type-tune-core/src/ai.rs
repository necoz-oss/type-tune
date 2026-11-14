//! Provider-neutral AI review requests and provider adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{Result, TypeTuneError, plan::PlanFile};

/// Version of the AI review prompt contract.
pub const PROMPT_VERSION: u8 = 1;
/// Default timeout for AI provider requests in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const SYSTEM_PROMPT: &str = "You review TypeScript type extraction candidates. Do not rewrite code. Return only JSON with a decisions array. Each decision must include candidateId, status, confidence, reason, and concerns. Status must be apply, ignore, or needs-review. Confidence must be a number from 0 to 1.";

/// AI recommendation recorded on a plan entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewDecision {
    /// Candidate id reviewed by the provider.
    pub candidate_id: String,
    /// Recommended status: `apply`, `ignore`, or `needs-review`.
    pub status: String,
    /// Confidence score between zero and one.
    pub confidence: f64,
    /// Human-readable reason for the recommendation.
    pub reason: String,
    /// Potential risks or manual checks.
    pub concerns: Vec<String>,
    /// Provider that produced the decision.
    pub provider: String,
    /// Model used to produce the decision.
    pub model: String,
    /// RFC 3339 timestamp for the review.
    pub reviewed_at: String,
}

fn validate_decision_status(status: &str, path: &str) -> Result<()> {
    if !matches!(status, "apply" | "ignore" | "needs-review") {
        return Err(TypeTuneError::new(format!(
            "Invalid {path}.status. Expected \"apply\" or \"ignore\" or \"needs-review\"."
        )));
    }
    Ok(())
}

fn validate_decision_confidence(confidence: f64, path: &str) -> Result<()> {
    if !(0.0..=1.0).contains(&confidence) {
        return Err(TypeTuneError::new(format!(
            "Invalid {path}.confidence. Expected a number from 0 to 1."
        )));
    }
    Ok(())
}

/// Provider-independent request generated from a plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewRequest {
    /// Project path from the plan.
    pub project_path: String,
    /// Placement style used by the plan.
    pub placement_style: String,
    /// Candidate contexts submitted for review.
    pub candidates: Vec<AiReviewCandidateContext>,
    /// Prompt and context policy.
    pub policy: AiReviewPolicy,
}

/// Provider-independent request policy.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewPolicy {
    /// Context detail level.
    pub context_mode: String,
    /// Prompt contract version.
    pub prompt_version: u8,
    /// Optional maximum number of source-context bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_bytes: Option<usize>,
    /// Whether any source context was truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_truncated: Option<bool>,
}

/// Candidate data supplied to an AI review provider.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewCandidateContext {
    /// Candidate id.
    pub candidate_id: String,
    /// Exported type name.
    pub symbol_name: String,
    /// Declaration kind.
    pub kind: String,
    /// Candidate source file path.
    pub source_path: String,
    /// Proposed destination path.
    pub destination_path: String,
    /// Paths importing the candidate.
    pub referrer_paths: Vec<String>,
    /// Total importing-file count.
    pub referrer_count: usize,
    /// Count used for candidate eligibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counted_referrer_count: Option<usize>,
    /// Local declarations moving with the candidate.
    pub local_dependency_names: Vec<String>,
    /// Import strategy selected for each writable referrer.
    pub referrer_strategies: Vec<crate::model::ReferrerStrategy>,
    /// Referrers counted outside the writable TypeScript strategy list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readonly_referrers: Vec<crate::model::ReadOnlyReferrer>,
    /// Specifier updates planned inside Vue script blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vue_script_strategies: Vec<crate::model::ReferrerStrategy>,
    /// Vue referrers that block a move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vue_rewrite_blockers: Vec<crate::model::VueRewriteBlocker>,
    /// Analysis gaps that make rewriting this candidate unsafe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis_limitations: Vec<crate::model::AnalysisLimitation>,
    /// Plan freshness fingerprint.
    pub source_fingerprint: String,
    /// Original source-context byte count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_context_bytes: Option<usize>,
    /// Source-context bytes sent after truncation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_context_bytes: Option<usize>,
    /// Whether this candidate's context was truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_truncated: Option<bool>,
    /// Reason source context was truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<String>,
    /// Candidate declaration text for snippets and full modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_text: Option<String>,
    /// Local dependency texts for snippets and full modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_dependency_texts: Option<Vec<String>>,
    /// Relevant referrer imports for snippets and full modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer_import_snippets: Option<Vec<AiReviewReferrerImportSnippet>>,
    /// Full source and referrer texts for full mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file_texts: Option<Vec<AiReviewSourceFileText>>,
}

/// One referrer import included in a review request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewReferrerImportSnippet {
    /// Project-relative referrer path.
    pub referrer_path: String,
    /// Redacted import declaration text.
    pub import_text: String,
}

/// One full redacted source file included in a review request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReviewSourceFileText {
    /// Project-relative source path.
    pub source_path: String,
    /// Redacted source text.
    pub text: String,
}

/// Selected AI provider configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiReviewProviderOptions {
    /// Provider name selected by `--ai-provider`.
    pub provider: String,
    /// Provider model name, if required.
    pub model: Option<String>,
    /// Optional endpoint override.
    pub endpoint: Option<String>,
    /// Optional API-key environment variable name.
    pub api_key_env: Option<String>,
    /// Mock response file path.
    pub response_file: Option<String>,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

mod cache;
mod provider;
mod request;
mod response;
use cache::{cache_key, read_cache, write_cache};
use provider::call_http_provider;
#[cfg(test)]
use provider::{build_http_request, parse_http_response};
pub use request::build_request;
#[cfg(test)]
use request::redact;
use response::validate_response;

/// Persisted AI review cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiReviewCacheFile {
    /// Cache schema version.
    pub version: u8,
    /// Cached candidate decisions.
    pub entries: Vec<AiReviewCacheEntry>,
}

/// One cached AI review decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiReviewCacheEntry {
    /// Stable request and candidate fingerprint.
    pub key: String,
    /// Provider-normalized decision.
    pub decision: AiReviewDecision,
}

/// Runs a provider and validates its decisions against the request candidates.
///
/// # Errors
///
/// Returns an error for invalid provider configuration, transport failures, or invalid responses.
pub fn review(
    request: &AiReviewRequest,
    options: &AiReviewProviderOptions,
) -> Result<Vec<AiReviewDecision>> {
    let model = resolve_model(options)?;
    let raw = match options.provider.as_str() {
        "mock" => read_mock_response(options)?,
        "openai" | "openai-compatible" | "anthropic" | "ollama" => {
            call_http_provider(request, options, &model)?
        }
        provider => {
            return Err(TypeTuneError::new(format!(
                "AI review provider \"{provider}\" is not implemented yet. Use --ai-provider openai, --ai-provider openai-compatible, --ai-provider anthropic, --ai-provider ollama, or --ai-provider mock."
            )));
        }
    };
    validate_response(raw, request, &options.provider, &model)
}

/// Runs a provider using reusable candidate decisions from a cache file.
///
/// # Errors
///
/// Returns an error for malformed cache data, provider failures, or invalid responses.
pub fn review_with_cache(
    plan: &PlanFile,
    request: &AiReviewRequest,
    options: &AiReviewProviderOptions,
    cache_path: &std::path::Path,
) -> Result<Vec<AiReviewDecision>> {
    let cache = read_cache(cache_path)?;
    let entries = plan
        .entries
        .iter()
        .map(|entry| (entry.candidate.id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let cached = cache
        .entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry.decision.clone()))
        .collect::<BTreeMap<_, _>>();
    let model = resolve_model(options)?;
    let mut decisions = Vec::new();
    let mut missing = Vec::new();
    for candidate in &request.candidates {
        let entry = entries
            .get(candidate.candidate_id.as_str())
            .ok_or_else(|| {
                TypeTuneError::new(format!(
                    "AI review request references unknown candidate id \"{}\".",
                    candidate.candidate_id
                ))
            })?;
        let key = cache_key(
            &options.provider,
            &model,
            options.endpoint.as_deref(),
            request,
            candidate,
            entry,
        )?;
        if let Some(decision) = cached.get(key.as_str()) {
            if decision.candidate_id != candidate.candidate_id
                || decision.provider != options.provider
                || decision.model != model
            {
                return Err(TypeTuneError::new(format!(
                    "Invalid AI review cache entry for candidate \"{}\": decision metadata does not match the request.",
                    candidate.candidate_id
                )));
            }
            decisions.push(decision.clone());
        } else {
            missing.push(candidate.clone());
        }
    }
    if missing.is_empty() {
        return Ok(decisions);
    }
    let mut missed_request = request.clone();
    missed_request.candidates = missing;
    let new_decisions = review(&missed_request, options)?;
    let mut updated = cache
        .entries
        .into_iter()
        .map(|entry| (entry.key.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let candidates = request
        .candidates
        .iter()
        .map(|candidate| (candidate.candidate_id.as_str(), candidate))
        .collect::<BTreeMap<_, _>>();
    for decision in &new_decisions {
        if let (Some(candidate), Some(entry)) = (
            candidates.get(decision.candidate_id.as_str()),
            entries.get(decision.candidate_id.as_str()),
        ) {
            let key = cache_key(
                &options.provider,
                &model,
                options.endpoint.as_deref(),
                request,
                candidate,
                entry,
            )?;
            updated.insert(
                key.clone(),
                AiReviewCacheEntry {
                    key,
                    decision: decision.clone(),
                },
            );
        }
    }
    let cache = AiReviewCacheFile {
        version: 1,
        entries: updated.into_values().collect(),
    };
    write_cache(cache_path, &cache)?;
    decisions.extend(new_decisions);
    let request_order = request
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.candidate_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    decisions.sort_by_key(|decision| {
        request_order
            .get(decision.candidate_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    Ok(decisions)
}

fn resolve_model(options: &AiReviewProviderOptions) -> Result<String> {
    if options.provider == "mock" {
        return Ok("mock-model".into());
    }
    if !matches!(
        options.provider.as_str(),
        "openai" | "openai-compatible" | "anthropic" | "ollama"
    ) {
        return Err(TypeTuneError::new(format!(
            "AI review provider \"{}\" is not implemented yet. Use --ai-provider openai, --ai-provider openai-compatible, --ai-provider anthropic, --ai-provider ollama, or --ai-provider mock.",
            options.provider
        )));
    }
    let model = options.model.clone().ok_or_else(|| {
        TypeTuneError::new(format!(
            "--ai-model is required when using --ai-provider {}.",
            options.provider
        ))
    })?;
    if options.provider == "openai-compatible"
        && options.endpoint.as_deref().is_none_or(str::is_empty)
    {
        return Err(TypeTuneError::new(
            "--ai-endpoint is required when using --ai-provider openai-compatible.",
        ));
    }
    if matches!(options.provider.as_str(), "openai" | "anthropic") {
        let name = options
            .api_key_env
            .as_deref()
            .unwrap_or(if options.provider == "openai" {
                "OPENAI_API_KEY"
            } else {
                "ANTHROPIC_API_KEY"
            });
        if std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(TypeTuneError::new(format!(
                "Environment variable {name} is required for AI review."
            )));
        }
    }
    Ok(model)
}

fn read_mock_response(options: &AiReviewProviderOptions) -> Result<Value> {
    let path = options.response_file.as_ref().ok_or_else(|| {
        TypeTuneError::new("--ai-response-file is required when using --ai-provider mock.")
    })?;
    let content = std::fs::read_to_string(path)
        .map_err(|error| TypeTuneError::new(format!("Could not read {path}: {error}")))?;
    serde_json::from_str(&content)
        .map_err(|error| TypeTuneError::new(format!("Could not parse AI review response: {error}")))
}

/// Merges AI decisions into a plan, creating needs-review annotations for omissions.
#[must_use]
pub fn merge_decisions(
    plan: &PlanFile,
    provider: &str,
    model: &str,
    decisions: Vec<AiReviewDecision>,
) -> PlanFile {
    let by_id = decisions
        .into_iter()
        .map(|decision| (decision.candidate_id.clone(), decision))
        .collect::<BTreeMap<_, _>>();
    let reviewed_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());
    let mut plan = plan.clone();
    for entry in &mut plan.entries {
        let decision =
            by_id
                .get(&entry.candidate.id)
                .cloned()
                .unwrap_or_else(|| AiReviewDecision {
                    candidate_id: entry.candidate.id.clone(),
                    status: "needs-review".into(),
                    confidence: 0.0,
                    reason: "AI review response did not include a decision for this candidate."
                        .into(),
                    concerns: vec!["Review this candidate manually before applying.".into()],
                    provider: provider.into(),
                    model: model.into(),
                    reviewed_at: reviewed_at.clone(),
                });
        entry.status = if decision.status == "apply" {
            "apply"
        } else {
            "ignore"
        }
        .into();
        entry.ai_review = Some(decision);
    }
    plan
}

/// Marks every entry as needing manual review after a provider failure.
#[must_use]
pub fn mark_error(plan: &PlanFile, provider: &str, model: &str, error: &str) -> PlanFile {
    let reviewed_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());
    let mut plan = plan.clone();
    for entry in &mut plan.entries {
        entry.status = "ignore".into();
        entry.ai_review = Some(AiReviewDecision {
            candidate_id: entry.candidate.id.clone(),
            status: "needs-review".into(),
            confidence: 0.0,
            reason: "AI review failed before producing a usable decision for this candidate."
                .into(),
            concerns: vec![format!("AI review error: {error}")],
            provider: provider.into(),
            model: model.into(),
            reviewed_at: reviewed_at.clone(),
        });
    }
    plan
}

#[cfg(test)]
mod tests;
