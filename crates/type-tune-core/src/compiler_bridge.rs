//! Experimental TypeScript compiler resolution bridge.
//!
//! Rust owns project-boundary policy. The Node bridge is a mechanical interpreter of the
//! policy snapshot below and a source of compiler resolution evidence. It is not a second
//! authorization policy and it is not the default scan path.

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Result, TypeTuneError, config::ProjectConfig,
    module_path::resolve_relative_module_with_options,
    module_path::resolve_scoped_path_alias_with_base_url_and_module_suffixes,
    module_specifier::is_relative_module_specifier, source::validate_discovery_path,
};

const PROTOCOL: &str = "type-tune.compiler-resolution.v1";
const POLICY_VERSION: u32 = 1;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCES: usize = 20_000;
const MAX_EDGES: usize = 100_000;
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(120);

const IMPLEMENTATION_EXTENSIONS: &[&str] = &[
    ".vue", ".tsx", ".mts", ".cts", ".ts", ".jsx", ".mjs", ".cjs", ".js",
];
const DECLARATION_SUFFIXES: &[&str] = &[".d.mts", ".d.cts", ".d.ts"];
const OUTSIDE_BOUNDARY: &str = "project source resolves outside the project boundary";
const DEPENDENCY_IMPLEMENTATION: &str = "dependency implementation source is not authorized";
const SYMLINK_IDENTITY: &str = "source symlink identity is unsupported";

/// How a scan should establish module-resolution evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMode {
    /// The existing Rust resolver. This remains the default production path.
    Native,
    /// One guarded TypeScript program per project. Incomplete evidence is an error.
    CompilerBridge,
}

/// Additional in-project implementation files the compiler proved and Rust can represent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeReconciliation {
    /// Project-relative paths to add to the Rust analysis graph.
    pub extra_sources: Vec<String>,
    /// Compiler and process timings for the experimental bridge.
    pub timings: BridgeTimings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTimings {
    /// Wall time spent in the Node process, including startup.
    pub process_ms: u128,
    /// Time to read and parse the TypeScript project configuration.
    pub config_ms: u128,
    /// Time to build the program and resolve modules.
    pub program_ms: u128,
    /// Time to serialize the resolution graph inside the bridge.
    pub graph_ms: u128,
    /// Peak RSS reported by the Node process.
    pub node_peak_rss_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRequest<'a> {
    protocol: &'a str,
    project_path: &'a str,
    typescript_entry: &'a str,
    policy: Policy<'a>,
    limits: Limits,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Policy<'a> {
    version: u32,
    project_root: String,
    typescript_lib_root: String,
    implementation_extensions: &'a [&'a str],
    declaration_suffixes: &'a [&'a str],
    unsupported_implementation_extensions: &'a [&'a str],
    metadata_basenames: &'a [&'a str],
    dependency_path_segment: &'a str,
    reasons: Reasons<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reasons<'a> {
    outside_boundary: &'a str,
    dependency_implementation: &'a str,
    symlink_identity: &'a str,
    virtual_identity: &'a str,
    unresolved_identity: &'a str,
    framework_identity: &'a str,
    plugins: &'a str,
    project_references: &'a str,
    emit_refused: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    max_sources: usize,
    max_edges: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeResponse {
    protocol: String,
    policy_version: u32,
    status: String,
    complete: bool,
    coverage: String,
    message: String,
    compiler_version: Option<String>,
    module_resolution: Option<String>,
    sources: Vec<BridgeSource>,
    edges: Vec<BridgeEdge>,
    diagnostics: Vec<BridgeDiagnostic>,
    resolution_failures: Vec<BridgeFailure>,
    unauthorized: Vec<UnauthorizedSource>,
    content_reads: Vec<String>,
    timings_ms: ResponseTimings,
    peak_rss_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeSource {
    logical_path: String,
    canonical_path: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeEdge {
    from: String,
    specifier: String,
    to: Option<String>,
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeDiagnostic {
    code: i32,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeFailure {
    from: String,
    specifier: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnauthorizedSource {
    logical_path: String,
    canonical_path: String,
    reason: String,
    from: Option<String>,
    specifier: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ResponseTimings {
    config: u128,
    program: u128,
    graph: u128,
}

struct ProtocolError(String);

/// Runs the experimental bridge and returns in-project sources Rust can add safely.
///
/// # Errors
///
/// Returns an error when Node or the project-local TypeScript package is unavailable, when the
/// compiler evidence is incomplete, unauthorized, unsupported, or malformed, or when a compiler
/// edge cannot be represented in the Rust graph. Native resolution is not used as a fallback.
pub fn reconcile_compiler_graph(
    tsconfig_path: &Path,
    config: &ProjectConfig,
    files: &[(String, String)],
) -> Result<BridgeReconciliation> {
    if let Some(message) = framework_blocker(tsconfig_path, files) {
        return Err(TypeTuneError::new(message));
    }
    let started = Instant::now();
    let response = run_bridge(tsconfig_path, &config.project_root)?;
    let process_ms = started.elapsed().as_millis();
    write_trace(&response, process_ms);
    if let Some(message) = unauthorized_content_read(&config.project_root, &response) {
        return Err(TypeTuneError::new(message));
    }
    if response.protocol != PROTOCOL || response.policy_version != POLICY_VERSION {
        return Err(TypeTuneError::new(
            "Compiler resolution bridge returned malformed output and the graph was not used: protocol version does not match.",
        ));
    }
    match response.status.as_str() {
        "unauthorized" => Err(TypeTuneError::new(unauthorized_message(
            &config.project_root,
            &response,
        ))),
        "unsupported" => Err(TypeTuneError::new(format!(
            "Compiler resolution bridge cannot establish a complete resolution graph: {}",
            response.message
        ))),
        "diagnostics" => Err(TypeTuneError::new(diagnostic_message(&response))),
        "malformed" => Err(TypeTuneError::new(format!(
            "Compiler resolution bridge returned malformed output and the graph was not used: {}",
            response.message
        ))),
        "ok" if response.complete && response.coverage == "complete" => {
            let extra_sources = reconcile_sources(&config.project_root, config, files, &response)?;
            Ok(BridgeReconciliation {
                extra_sources,
                timings: BridgeTimings {
                    process_ms,
                    config_ms: response.timings_ms.config,
                    program_ms: response.timings_ms.program,
                    graph_ms: response.timings_ms.graph,
                    node_peak_rss_bytes: response.peak_rss_bytes,
                },
            })
        }
        "ok" => Err(TypeTuneError::new(
            "Compiler resolution bridge returned a partial graph and it was not used as a complete analysis.",
        )),
        _ => Err(TypeTuneError::new(format!(
            "Compiler resolution bridge returned malformed output and the graph was not used: unknown status {}.",
            response.status
        ))),
    }
}

fn framework_blocker(tsconfig_path: &Path, files: &[(String, String)]) -> Option<String> {
    if files.iter().any(|(path, _)| path.ends_with(".vue"))
        || tsconfig_path
            .components()
            .any(|component| component.as_os_str() == ".nuxt")
    {
        return Some(
            "Compiler resolution bridge cannot represent Vue or Nuxt framework sources. Refusing an incomplete resolver result."
                .to_owned(),
        );
    }
    None
}

fn run_bridge(tsconfig_path: &Path, project_root: &Path) -> Result<BridgeResponse> {
    let tsconfig = fs::canonicalize(tsconfig_path).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not resolve {}: {error}. Native resolution was not used.",
            tsconfig_path.display()
        ))
    })?;
    let root = fs::canonicalize(project_root).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not resolve project boundary {}: {error}. Native resolution was not used.",
            project_root.display()
        ))
    })?;
    let entry = typescript_entry(&root)?;
    let version = typescript_package_version(&entry)?;
    ensure_supported_compiler(&version)?;
    let lib_root = entry.parent().ok_or_else(|| {
        TypeTuneError::new(
            "Compiler resolution bridge could not locate the TypeScript lib directory. Native resolution was not used.",
        )
    })?;
    let lib_root = fs::canonicalize(lib_root).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not canonicalize the TypeScript package: {error}. Native resolution was not used."
        ))
    })?;
    let node = node_program()?;
    let script_dir = tempfile::Builder::new()
        .prefix("type-tune-bridge-")
        .tempdir()
        .map_err(|error| {
            TypeTuneError::new(format!(
                "Compiler resolution bridge could not create a process directory: {error}. Native resolution was not used."
            ))
        })?;
    let script = script_dir.path().join("compiler-resolution-bridge.mjs");
    fs::write(&script, include_str!("../bridge/compiler-resolution-bridge.mjs")).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not write its script: {error}. Native resolution was not used."
        ))
    })?;
    let request = BridgeRequest {
        protocol: PROTOCOL,
        project_path: &tsconfig.to_string_lossy(),
        typescript_entry: &entry.to_string_lossy(),
        policy: policy(&root, &lib_root),
        limits: Limits {
            max_sources: MAX_SOURCES,
            max_edges: MAX_EDGES,
        },
    };
    let request_bytes = serde_json::to_vec(&request).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge request was malformed: {error}. Native resolution was not used."
        ))
    })?;
    let mut child = Command::new(&node)
        .arg(&script)
        .current_dir(&root)
        .env("NODE_OPTIONS", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            TypeTuneError::new(format!(
                "Compiler resolution bridge requires Node.js, but {node} could not be started: {error}. Native resolution was not used.",
                node = node.display()
            ))
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        TypeTuneError::new(
            "Compiler resolution bridge process failed and native resolution was not used: missing stdout.",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        TypeTuneError::new(
            "Compiler resolution bridge process failed and native resolution was not used: missing stderr.",
        )
    })?;
    let stdout_thread = thread::spawn(move || read_limited(stdout, MAX_OUTPUT_BYTES));
    let stderr_thread = thread::spawn(move || read_limited(stderr, MAX_OUTPUT_BYTES));
    child
        .stdin
        .take()
        .ok_or_else(|| {
            TypeTuneError::new(
                "Compiler resolution bridge process failed and native resolution was not used: missing stdin.",
            )
        })?
        .write_all(&request_bytes)
        .map_err(|error| {
            TypeTuneError::new(format!(
                "Compiler resolution bridge process failed and native resolution was not used: {error}"
            ))
        })?;
    let started = Instant::now();
    let status = loop {
        if started.elapsed() > BRIDGE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TypeTuneError::new(
                "Compiler resolution bridge timed out and native resolution was not used. The resolution graph is incomplete.",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                return Err(TypeTuneError::new(format!(
                    "Compiler resolution bridge process failed and native resolution was not used: {error}"
                )));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_thread.join().map_err(|_| {
        TypeTuneError::new(
            "Compiler resolution bridge process failed and native resolution was not used: stdout reader stopped.",
        )
    })?;
    let (stderr, stderr_truncated) = stderr_thread.join().map_err(|_| {
        TypeTuneError::new(
            "Compiler resolution bridge process failed and native resolution was not used: stderr reader stopped.",
        )
    })?;
    if stdout_truncated || stderr_truncated {
        return Err(TypeTuneError::new(
            "Compiler resolution bridge output exceeded the limit. The resolution graph is incomplete and was not used.",
        ));
    }
    if !status.success() && stdout.is_empty() {
        return Err(TypeTuneError::new(format!(
            "Compiler resolution bridge process failed and native resolution was not used: {status}; {}",
            String::from_utf8_lossy(&stderr).trim()
        )));
    }
    interpret_response(&stdout).map_err(|error| TypeTuneError::new(error.0))
}

fn interpret_response(stdout: &[u8]) -> std::result::Result<BridgeResponse, ProtocolError> {
    let text = String::from_utf8(stdout.to_vec()).map_err(|_| {
        ProtocolError(
            "Compiler resolution bridge returned malformed output and the graph was not used: stdout was not UTF-8.".to_owned(),
        )
    })?;
    serde_json::from_str(&text).map_err(|error| {
        ProtocolError(format!(
            "Compiler resolution bridge returned malformed output and the graph was not used: {error}"
        ))
    })
}

fn policy<'a>(root: &Path, lib_root: &Path) -> Policy<'a> {
    Policy {
        version: POLICY_VERSION,
        project_root: root.to_string_lossy().into_owned(),
        typescript_lib_root: lib_root.to_string_lossy().into_owned(),
        implementation_extensions: IMPLEMENTATION_EXTENSIONS,
        declaration_suffixes: DECLARATION_SUFFIXES,
        unsupported_implementation_extensions: &[".vue"],
        metadata_basenames: &["package.json", "jsconfig.json"],
        dependency_path_segment: "node_modules",
        reasons: Reasons {
            outside_boundary: OUTSIDE_BOUNDARY,
            dependency_implementation: DEPENDENCY_IMPLEMENTATION,
            symlink_identity: SYMLINK_IDENTITY,
            virtual_identity: "virtual source identity is unsupported",
            unresolved_identity: "implementation identity could not be canonicalized",
            framework_identity: "framework-generated or Vue source identity is unsupported",
            plugins: "TypeScript compiler plugins are not supported by the resolution bridge",
            project_references: "project reference traversal is not supported by the resolution bridge",
            emit_refused: "compiler resolution bridge refused to emit output files",
        },
    }
}

fn node_program() -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os("TYPE_TUNE_NODE") {
        let path = PathBuf::from(&explicit);
        if path.is_file() {
            return Ok(path);
        }
        return Err(TypeTuneError::new(format!(
            "Compiler resolution bridge requires Node.js, but TYPE_TUNE_NODE={} is not a file. Native resolution was not used.",
            path.display()
        )));
    }
    Ok(PathBuf::from("node"))
}

fn typescript_entry(project_root: &Path) -> Result<PathBuf> {
    let mut directory = Some(project_root);
    while let Some(current) = directory {
        let entry = current.join("node_modules/typescript/lib/typescript.js");
        let manifest = current.join("node_modules/typescript/package.json");
        if entry.is_file() && manifest.is_file() {
            return Ok(entry);
        }
        directory = current.parent();
    }
    Err(TypeTuneError::new(
        "Compiler resolution bridge requires the project-local TypeScript package, but node_modules/typescript was not found. Dependencies were not installed and native resolution was not used. Loading that package executes project-controlled code and must be trusted; it is not a filesystem sandbox.",
    ))
}

fn typescript_package_version(entry: &Path) -> Result<String> {
    let manifest = entry
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| TypeTuneError::new("TypeScript package layout is invalid.".to_owned()))?
        .join("package.json");
    let content = fs::read_to_string(&manifest).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not read {}: {error}. Native resolution was not used.",
            manifest.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|error| {
        TypeTuneError::new(format!(
            "Compiler resolution bridge could not parse {}: {error}. Native resolution was not used.",
            manifest.display()
        ))
    })?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            TypeTuneError::new(
                "Compiler resolution bridge could not read the TypeScript package version. Native resolution was not used.",
            )
        })
}

fn ensure_supported_compiler(version: &str) -> Result<()> {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse::<u64>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u64>().ok());
    let supported = matches!((major, minor), (Some(4), Some(minor)) if minor >= 9)
        || matches!(major, Some(5 | 6));
    if supported {
        return Ok(());
    }
    Err(TypeTuneError::new(format!(
        "Compiler resolution bridge does not support TypeScript {version}. The tested range is 4.9 through 6.x. Native resolution was not used."
    )))
}

fn read_limited(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return (output, false),
            Ok(count) => {
                if output.len().saturating_add(count) > limit {
                    return (output, true);
                }
                output.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return (output, false),
        }
    }
}

fn write_trace(response: &BridgeResponse, process_ms: u128) {
    let Some(path) = std::env::var_os("TYPE_TUNE_BRIDGE_TRACE_PATH") else {
        return;
    };
    let mut value = serde_json::to_value(TraceResponse {
        protocol: &response.protocol,
        status: &response.status,
        complete: response.complete,
        coverage: &response.coverage,
        message: &response.message,
        content_reads: &response.content_reads,
        unauthorized: &response.unauthorized,
        compiler_version: &response.compiler_version,
        module_resolution: &response.module_resolution,
        resolution_failures: response.resolution_failures.len(),
        source_count: response.sources.len(),
        edge_count: response.edges.len(),
        process_ms,
        timings_ms: &response.timings_ms,
        peak_rss_bytes: response.peak_rss_bytes,
    })
    .unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert(
            "responseMessage".to_owned(),
            Value::String(response.message.clone()),
        );
    }
    let _ = fs::write(path, value.to_string());
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceResponse<'a> {
    protocol: &'a str,
    status: &'a str,
    complete: bool,
    coverage: &'a str,
    message: &'a str,
    content_reads: &'a [String],
    unauthorized: &'a [UnauthorizedSource],
    compiler_version: &'a Option<String>,
    module_resolution: &'a Option<String>,
    resolution_failures: usize,
    source_count: usize,
    edge_count: usize,
    process_ms: u128,
    timings_ms: &'a ResponseTimings,
    peak_rss_bytes: u64,
}

fn unauthorized_content_read(root: &Path, response: &BridgeResponse) -> Option<String> {
    response.content_reads.iter().find_map(|read| {
        let path = PathBuf::from(read);
        if !is_implementation(&path) {
            return None;
        }
        match implementation_relative(root, &path) {
            Err(ImplementationAuth::Unauthorized { logical, canonical, reason }) => Some(format!(
                "Compiler resolution bridge read an unauthorized implementation source: {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
                logical.display(),
                canonical.display(),
                root.display()
            )),
            _ => None,
        }
    })
}

fn unauthorized_message(root: &Path, response: &BridgeResponse) -> String {
    let Some(item) = response.unauthorized.last() else {
        return format!(
            "Compiler resolution bridge rejected an unauthorized implementation source before reading it: {} Project boundary: {}.",
            response.message,
            root.display()
        );
    };
    let referrer = match (&item.from, &item.specifier) {
        (Some(from), Some(specifier)) => format!(
            "Import resolution cannot analyze {specifier:?} from {}: ",
            display_project_path(root, from)
        ),
        _ => String::new(),
    };
    format!(
        "Compiler resolution bridge rejected an unauthorized implementation source before reading it: {referrer}{}; logical path: {}; canonical path: {}; project boundary: {}.",
        item.reason,
        item.logical_path,
        item.canonical_path,
        root.display()
    )
}

fn diagnostic_message(response: &BridgeResponse) -> String {
    let details = response
        .diagnostics
        .iter()
        .map(|diagnostic| format!("TS{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "Compiler resolution bridge refused the project because the TypeScript configuration is unsupported or invalid: {} {details}",
        response.message
    )
}

fn reconcile_sources(
    root: &Path,
    config: &ProjectConfig,
    files: &[(String, String)],
    response: &BridgeResponse,
) -> Result<Vec<String>> {
    let root = fs::canonicalize(root).map_err(|error| {
        TypeTuneError::new(format!("Could not canonicalize project boundary: {error}"))
    })?;
    let mut known = files
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    let mut extras = Vec::new();
    for source in &response.sources {
        if source.role == "library" || source.role == "declaration" || source.role == "metadata" {
            continue;
        }
        let logical = PathBuf::from(&source.logical_path);
        if is_virtual(&logical)
            || source.role == "implementation"
                && logical.extension().is_some_and(|ext| ext == "vue")
        {
            return Err(TypeTuneError::new(
                "Compiler resolution bridge found an unsupported framework-generated or virtual source identity. The partial graph was not used.",
            ));
        }
        if !is_implementation(&logical) {
            return Err(TypeTuneError::new(format!(
                "Compiler resolution bridge returned an unsupported source role {} for {}. The graph was not used.",
                source.role, source.logical_path
            )));
        }
        let relative = match implementation_relative(&root, &logical) {
            Ok(relative) => relative,
            Err(ImplementationAuth::Unauthorized {
                logical,
                canonical,
                reason,
            }) => {
                return Err(TypeTuneError::new(format!(
                    "Compiler resolution bridge rejected an unauthorized implementation source before reading it: {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
                    logical.display(),
                    canonical.display(),
                    root.display()
                )));
            }
            Err(ImplementationAuth::Unsupported {
                logical,
                canonical,
                reason,
            }) => {
                return Err(TypeTuneError::new(format!(
                    "Compiler resolution bridge cannot represent {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
                    logical.display(),
                    canonical.display(),
                    root.display()
                )));
            }
        };
        let actual = fs::canonicalize(&logical).map_err(|error| {
            TypeTuneError::new(format!(
                "Compiler resolution evidence conflicts with filesystem identity for {}: {error}",
                logical.display()
            ))
        })?;
        let reported = fs::canonicalize(&source.canonical_path).map_err(|error| {
            TypeTuneError::new(format!(
                "Compiler resolution evidence conflicts with filesystem identity for {}: {error}",
                source.canonical_path
            ))
        })?;
        if actual != reported {
            return Err(TypeTuneError::new(format!(
                "Compiler resolution evidence has conflicting source identities for {}: compiler canonical path {}, filesystem canonical path {}.",
                source.logical_path,
                reported.display(),
                actual.display()
            )));
        }
        if known.insert(relative.clone()) {
            extras.push(relative);
        }
    }
    let mut unrepresentable = Vec::new();
    for edge in &response.edges {
        let Some(target) = edge.to.as_deref() else {
            continue;
        };
        let target_path = PathBuf::from(target);
        if !is_implementation(&target_path) || is_declaration(&target_path) {
            continue;
        }
        let relative = match implementation_relative(&root, &target_path) {
            Ok(relative) => relative,
            Err(auth) => {
                return Err(TypeTuneError::new(auth_edge_message(&root, edge, auth)));
            }
        };
        if !is_implementation(Path::new(&edge.from)) || is_declaration(Path::new(&edge.from)) {
            if !files.iter().any(|(path, _)| path == &relative)
                && !represented_by_implementation_edge(
                    &root,
                    config,
                    &known,
                    &response.edges,
                    &relative,
                )
            {
                unrepresentable.push(format!(
                    "{} {relative} via {} {:?}",
                    edge.kind,
                    display_project_path(&root, &edge.from),
                    edge.specifier
                ));
            }
            continue;
        }
        let from = match project_relative(&root, Path::new(&edge.from)) {
            Some(path) => path,
            None => {
                unrepresentable.push(format!("edge origin {} is outside the project", edge.from));
                continue;
            }
        };
        if !native_edge_matches(config, &known, &from, &edge.specifier, &relative) {
            return Err(TypeTuneError::new(format!(
                "Compiler resolution edge is omitted by native resolution and cannot be reconciled: {} imports {:?} as {relative}. Refusing incomplete referrer analysis.",
                from, edge.specifier
            )));
        }
    }
    if !unrepresentable.is_empty() {
        return Err(TypeTuneError::new(format!(
            "Compiler resolution found declaration-mediated project sources that the Rust referrer graph cannot represent: {}. Refusing incomplete candidates.",
            unrepresentable.join("; ")
        )));
    }
    for failure in &response.resolution_failures {
        if is_relative_module_specifier(&failure.specifier) {
            return Err(TypeTuneError::new(format!(
                "Compiler resolution failed for relative import {:?} from {}. Completeness could not be established.",
                failure.specifier,
                display_project_path(&root, &failure.from)
            )));
        }
    }
    Ok(extras)
}

fn represented_by_implementation_edge(
    root: &Path,
    config: &ProjectConfig,
    known: &BTreeSet<String>,
    edges: &[BridgeEdge],
    relative: &str,
) -> bool {
    edges.iter().any(|edge| {
        let Some(target) = edge.to.as_deref() else {
            return false;
        };
        if implementation_relative(root, Path::new(target))
            .ok()
            .as_deref()
            != Some(relative)
        {
            return false;
        }
        if !is_implementation(Path::new(&edge.from)) || is_declaration(Path::new(&edge.from)) {
            return false;
        }
        let Some(from) = project_relative(root, Path::new(&edge.from)) else {
            return false;
        };
        native_edge_matches(config, known, &from, &edge.specifier, relative)
    })
}

fn native_edge_matches(
    config: &ProjectConfig,
    known: &BTreeSet<String>,
    from: &str,
    specifier: &str,
    target: &str,
) -> bool {
    let known = known.iter().cloned().collect::<HashSet<_>>();
    resolve_relative_module_with_options(
        from,
        specifier,
        &known,
        &config.root_dirs,
        &config.module_suffixes,
    )
    .or_else(|| {
        resolve_scoped_path_alias_with_base_url_and_module_suffixes(
            from,
            specifier,
            config.base_url.as_deref(),
            &config.explicit_paths,
            &config.package_aliases,
            &known,
            &config.module_suffixes,
        )
    })
    .as_deref()
        == Some(target)
}

fn auth_edge_message(root: &Path, edge: &BridgeEdge, auth: ImplementationAuth) -> String {
    match auth {
        ImplementationAuth::Unauthorized {
            logical,
            canonical,
            reason,
        } => format!(
            "Compiler resolution bridge rejected an unauthorized implementation source before reading it: Import resolution cannot analyze {:?} from {}: {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
            edge.specifier,
            display_project_path(root, &edge.from),
            logical.display(),
            canonical.display(),
            root.display()
        ),
        ImplementationAuth::Unsupported {
            logical,
            canonical,
            reason,
        } => format!(
            "Compiler resolution bridge cannot represent {reason}; logical path: {}; canonical path: {}; project boundary: {}.",
            logical.display(),
            canonical.display(),
            root.display()
        ),
    }
}

#[derive(Debug)]
enum ImplementationAuth {
    Unauthorized {
        logical: PathBuf,
        canonical: PathBuf,
        reason: &'static str,
    },
    Unsupported {
        logical: PathBuf,
        canonical: PathBuf,
        reason: &'static str,
    },
}

fn implementation_relative(
    root: &Path,
    logical: &Path,
) -> std::result::Result<String, ImplementationAuth> {
    let root = fs::canonicalize(root).map_err(|_| ImplementationAuth::Unsupported {
        logical: logical.to_path_buf(),
        canonical: logical.to_path_buf(),
        reason: "project boundary",
    })?;
    let logical = if logical.is_absolute() {
        logical.to_path_buf()
    } else {
        root.join(logical)
    };
    if is_virtual(&logical) {
        return Err(ImplementationAuth::Unsupported {
            logical: logical.clone(),
            canonical: logical,
            reason: "virtual source identity is unsupported",
        });
    }
    let canonical = fs::canonicalize(&logical).map_err(|_| ImplementationAuth::Unsupported {
        logical: logical.clone(),
        canonical: logical.clone(),
        reason: "implementation identity could not be canonicalized",
    })?;
    if !canonical.starts_with(&root) {
        return Err(ImplementationAuth::Unauthorized {
            logical,
            canonical,
            reason: OUTSIDE_BOUNDARY,
        });
    }
    if has_node_modules(&logical) || has_node_modules(&canonical) {
        return Err(ImplementationAuth::Unauthorized {
            logical,
            canonical,
            reason: DEPENDENCY_IMPLEMENTATION,
        });
    }
    if !logical.starts_with(&root) {
        return Err(ImplementationAuth::Unsupported {
            logical,
            canonical,
            reason: SYMLINK_IDENTITY,
        });
    }
    if validate_discovery_path(&root, &logical).is_err() {
        return Err(ImplementationAuth::Unsupported {
            logical,
            canonical,
            reason: SYMLINK_IDENTITY,
        });
    }
    Ok(logical
        .strip_prefix(&root)
        .unwrap_or(logical.as_path())
        .to_string_lossy()
        .replace('\\', "/"))
}

fn is_virtual(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains('\0') || text.contains('?') || text.starts_with("virtual:")
}

fn is_implementation(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    !is_declaration(path)
        && IMPLEMENTATION_EXTENSIONS
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn is_declaration(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    DECLARATION_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn has_node_modules(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "node_modules")
}

fn project_relative(root: &Path, path: &Path) -> Option<String> {
    let root = fs::canonicalize(root).ok()?;
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = fs::canonicalize(&path).ok()?;
    canonical
        .strip_prefix(&root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn display_project_path(root: &Path, path: &str) -> String {
    project_relative(root, Path::new(path)).unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{implementation_relative, interpret_response};

    #[test]
    fn malformed_bridge_output_is_not_a_complete_graph() {
        let error = interpret_response(b"not-json").unwrap_err();
        assert!(error.0.contains("malformed"));
    }

    #[test]
    fn external_implementation_identity_is_unauthorized() {
        let parent =
            std::env::temp_dir().join(format!("type-tune-bridge-auth-{}", std::process::id()));
        let root = parent.join("project");
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(root.join("src")).unwrap();
        let outside = parent.join("outside.ts");
        std::fs::write(root.join("src/main.ts"), "export {}\n").unwrap();
        std::fs::write(&outside, "export {}\n").unwrap();
        let error = implementation_relative(&root, &outside).unwrap_err();
        assert!(matches!(
            error,
            super::ImplementationAuth::Unauthorized { .. }
        ));
        let _ = std::fs::remove_dir_all(&parent);
    }
}
