//! Experimental compiler resolution bridge.
//!
//! The declaration-chain regression is ignored by default `cargo test` because it needs Node.js
//! and the pinned TypeScript packages. CI runs this file with `--ignored` after those packages
//! are installed. The original reproduction in `bounded_import_resolution` stays ignored there.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Instant,
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "type-tune-bridge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn typescript_package(version: &str) -> PathBuf {
    let root = repo_root();
    if version == "60" {
        root.join("node_modules/typescript")
    } else {
        root.join(format!(
            "tests/compiler-versions/node_modules/typescript{version}"
        ))
    }
}

fn link_typescript(project: &Path, version: &str) {
    let package = typescript_package(version);
    assert!(
        package.join("lib/typescript.js").is_file(),
        "missing TypeScript {version} at {}",
        package.display()
    );
    fs::create_dir_all(project.join("node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink(&package, project.join("node_modules/typescript")).unwrap();
    std::os::unix::fs::symlink(
        "../typescript/bin/tsc",
        project.join("node_modules/.bin/tsc"),
    )
    .unwrap();
}

fn copy_declaration_fixture(version_label: &str) -> (Temp, PathBuf) {
    let temp = Temp::new();
    let destination = temp.path().join(version_label);
    let status = Command::new("cp")
        .args([
            "-a",
            repo_root()
                .join("fixtures/compiler-resolution-bridge/declaration-mediated-external")
                .to_str()
                .unwrap(),
            destination.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    (temp, destination.join("project"))
}

fn run(project: &Path, args: &[&str]) -> Output {
    let trace = project.join("bridge-trace.json");
    Command::new(env!("CARGO_BIN_EXE_type-tune"))
        .current_dir(project)
        .env("TYPE_TUNE_BRIDGE_TRACE_PATH", &trace)
        .args(args)
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, dir: &Path, result: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.file_name().is_some_and(|name| {
                name == "node_modules" || name == "bridge-trace.json" || name == ".type-tune"
            }) {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                result.push((
                    relative,
                    format!("link:{}", fs::read_link(&path).unwrap().display()).into_bytes(),
                ));
            } else if metadata.is_dir() {
                result.push((relative, b"directory".to_vec()));
                visit(root, &path, result);
            } else {
                result.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn missing_node_does_not_fall_back_to_native_resolution() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("node_modules/typescript/lib")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(project.join("src/main.ts"), "export type A = string;\n").unwrap();
    fs::write(
        project.join("node_modules/typescript/package.json"),
        r#"{"name":"typescript","version":"5.5.4"}"#,
    )
    .unwrap();
    fs::write(
        project.join("node_modules/typescript/lib/typescript.js"),
        "module.exports = {};\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_type-tune"))
        .current_dir(&project)
        .env("TYPE_TUNE_NODE", temp.path().join("missing-node"))
        .args(["scan", "--compiler-resolution-bridge"])
        .output()
        .unwrap();
    let message = text(&output);
    assert!(!output.status.success(), "{message}");
    assert!(
        message.contains("Native resolution was not used"),
        "{message}"
    );
    assert!(output.stdout.is_empty(), "{message}");
}

#[test]
fn missing_typescript_does_not_fall_back_to_native_resolution() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(project.join("src/main.ts"), "export type A = string;\n").unwrap();
    let output = run(&project, &["scan", "--compiler-resolution-bridge"]);
    let message = text(&output);
    assert!(!output.status.success(), "{message}");
    assert!(
        message.contains("project-local TypeScript package"),
        "{message}"
    );
    assert!(
        message
            .to_ascii_lowercase()
            .contains("native resolution was not used"),
        "{message}"
    );
    assert!(output.stdout.is_empty(), "{message}");
}

#[test]
#[ignore = "requires Node.js and the pinned TypeScript packages"]
fn declaration_chain_rejects_external_source_for_supported_compilers() {
    for version in ["49", "55", "58", "60"] {
        let (_temp, project) = copy_declaration_fixture(version);
        link_typescript(&project, version);
        let before = snapshot(project.parent().unwrap());
        let listed = Command::new(project.join("node_modules/.bin/tsc"))
            .current_dir(&project)
            .args([
                "--noEmit",
                "--listFilesOnly",
                "--pretty",
                "false",
                "--project",
                "tsconfig.json",
            ])
            .output()
            .unwrap();
        let listed_text = String::from_utf8_lossy(&listed.stdout);
        assert!(
            listed.status.success(),
            "{version}: {}",
            String::from_utf8_lossy(&listed.stderr)
        );
        assert!(
            listed_text.contains("hidden-source.ts"),
            "{version}: {listed_text}"
        );
        let output = run(
            &project,
            &["scan", "--compiler-resolution-bridge", "--format", "json"],
        );
        let message = text(&output);
        assert_eq!(output.status.code(), Some(1), "{version}: {message}");
        assert!(
            message.contains("outside the project boundary"),
            "{version}: {message}"
        );
        assert!(message.contains("hidden-source.ts"), "{version}: {message}");
        assert!(message.contains("canonical path:"), "{version}: {message}");
        assert!(output.stdout.is_empty(), "{version}: {message}");
        let trace: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(project.join("bridge-trace.json")).unwrap())
                .unwrap();
        let reads = trace["contentReads"].to_string();
        assert!(!reads.contains("hidden-source.ts"), "{version}: {reads}");
        assert!(trace["status"] == "unauthorized", "{version}: {trace}");
        assert_eq!(before, snapshot(project.parent().unwrap()), "{version}");
    }
}

#[test]
#[ignore = "requires Node.js and TypeScript 4.9"]
fn typescript_49_rejects_bundler_configuration_without_calling_it_a_boundary_success() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    link_typescript(&project, "49");
    fs::write(
        project.join("tsconfig.json"),
        r#"{"compilerOptions":{"module":"esnext","moduleResolution":"bundler"},"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(project.join("src/main.ts"), "export type A = string;\n").unwrap();
    let output = run(&project, &["scan", "--compiler-resolution-bridge"]);
    let message = text(&output);
    assert!(!output.status.success(), "{message}");
    assert!(message.contains("unsupported or invalid"), "{message}");
    assert!(
        !message.contains("outside the project boundary"),
        "{message}"
    );
}

#[test]
#[ignore = "requires Node.js and the pinned TypeScript packages"]
fn bridge_covers_resolution_patterns_without_reporting_a_partial_graph() {
    for version in ["49", "55", "58", "60"] {
        let temp = Temp::new();
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("src/a")).unwrap();
        fs::create_dir_all(project.join("src/b")).unwrap();
        fs::create_dir_all(project.join("node_modules")).unwrap();
        link_typescript(&project, version);
        let module_resolution = if version == "49" { "node" } else { "bundler" };
        let module_kind = if version == "49" {
            "commonjs"
        } else {
            "esnext"
        };
        fs::write(
            project.join("tsconfig.json"),
            format!(
                r#"{{"compilerOptions":{{"strict":true,"target":"ES2019","module":"{module_kind}","moduleResolution":"{module_resolution}","rootDirs":["src/a","src/b"]}},"include":["src"]}}"#
            ),
        )
        .unwrap();
        fs::write(
            project.join("src/a/main.ts"),
            "import type { Shared } from './model';\nexport type A = Shared;\n",
        )
        .unwrap();
        fs::write(
            project.join("src/a/model.ts"),
            "import type { Nested } from './nested';\nexport interface Shared { nested: Nested }\n",
        )
        .unwrap();
        fs::write(
            project.join("src/a/nested.ts"),
            "export interface Nested { value: string }\n",
        )
        .unwrap();
        fs::write(
            project.join("src/b/other.ts"),
            "import type { Shared } from './model';\nexport type B = Shared;\n",
        )
        .unwrap();
        fs::write(
            project.join("src/a/import-type.ts"),
            "export type Alias = import('./model').Shared;\n",
        )
        .unwrap();
        fs::write(
            project.join("src/a/reference.ts"),
            "/// <reference path=\"./model.ts\" />\nexport type Referenced = string;\n",
        )
        .unwrap();
        let dependency = temp.path().join("dependency");
        fs::create_dir(&dependency).unwrap();
        std::os::unix::fs::symlink(&dependency, project.join("node_modules/declared-pkg")).unwrap();
        fs::write(
            dependency.join("package.json"),
            r#"{"name":"declared-pkg","types":"index.d.ts"}"#,
        )
        .unwrap();
        fs::write(
            dependency.join("a.d.ts"),
            "export type { Left } from './b';\nexport interface Left { value: string }\n",
        )
        .unwrap();
        fs::write(
            dependency.join("b.d.ts"),
            "export type { Left } from './a';\n",
        )
        .unwrap();
        fs::write(
            dependency.join("index.d.ts"),
            "export type { Left } from './a';\n",
        )
        .unwrap();
        fs::write(
            project.join("src/a/package-user.ts"),
            "import type { Left } from 'declared-pkg';\nexport type Use = Left;\n",
        )
        .unwrap();
        let output = run(
            &project,
            &[
                "scan",
                "--compiler-resolution-bridge",
                "--min-referrers",
                "2",
                "--format",
                "json",
            ],
        );
        let message = text(&output);
        assert!(output.status.success(), "{version}: {message}");
        assert!(message.contains("Shared"), "{version}: {message}");
        let trace: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(project.join("bridge-trace.json")).unwrap())
                .unwrap();
        assert_eq!(trace["status"], "ok", "{version}: {trace}");
        assert_eq!(trace["complete"], true, "{version}: {trace}");
        assert_eq!(trace["coverage"], "complete", "{version}: {trace}");
    }
}

#[test]
#[ignore = "requires Node.js and the pinned TypeScript packages"]
fn package_exports_types_versions_and_external_sources_are_rejected() {
    for version in ["49", "55", "58", "60"] {
        let temp = Temp::new();
        let project = temp.path().join("project");
        let dependency = temp.path().join("dependency");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(dependency.join("src")).unwrap();
        fs::create_dir_all(project.join("node_modules")).unwrap();
        link_typescript(&project, version);
        std::os::unix::fs::symlink(&dependency, project.join("node_modules/external-pkg")).unwrap();
        let module_resolution = if version == "49" { "node16" } else { "bundler" };
        let module_kind = if version == "49" { "node16" } else { "esnext" };
        fs::write(
            project.join("tsconfig.json"),
            format!(r#"{{"compilerOptions":{{"module":"{module_kind}","moduleResolution":"{module_resolution}"}},"files":["src/main.ts"]}}"#),
        )
        .unwrap();
        fs::write(dependency.join("index.ts"), "export interface Shared {}\n").unwrap();
        for (manifest, specifier) in [
            (
                r#"{"name":"external-pkg","exports":{".":{"types":"./index.ts"}}}"#,
                "external-pkg",
            ),
            (
                r#"{"name":"external-pkg","typesVersions":{"*":{"*":["src/*.ts"]}}}"#,
                "external-pkg/item",
            ),
        ] {
            fs::write(dependency.join("package.json"), manifest).unwrap();
            if specifier.ends_with("item") {
                fs::write(
                    dependency.join("src/item.ts"),
                    "export interface Shared {}\n",
                )
                .unwrap();
            }
            fs::write(
                project.join("src/main.ts"),
                format!(
                    "import type {{ Shared }} from '{specifier}';\nexport type Use = Shared;\n"
                ),
            )
            .unwrap();
            let output = run(&project, &["scan", "--compiler-resolution-bridge"]);
            let message = text(&output);
            assert!(!output.status.success(), "{version} {specifier}: {message}");
            assert!(
                message.contains("outside the project boundary")
                    || message.contains("not authorized"),
                "{version} {specifier}: {message}"
            );
            assert!(output.stdout.is_empty(), "{version} {specifier}: {message}");
        }
    }
}

#[test]
#[ignore = "requires Node.js and the pinned TypeScript packages"]
fn workflows_fail_before_mutation_when_resolution_changes() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    link_typescript(&project, "60");
    fs::write(project.join("tsconfig.json"), r#"{"include":["src"]}"#).unwrap();
    fs::write(
        project.join("src/model.ts"),
        "export interface Shared { id: string }\n",
    )
    .unwrap();
    fs::write(
        project.join("src/a.ts"),
        "import type { Shared } from './model';\nexport type A = Shared;\n",
    )
    .unwrap();
    fs::write(
        project.join("src/b.ts"),
        "import type { Shared } from './model';\nexport type B = Shared;\n",
    )
    .unwrap();
    let scan = run(
        &project,
        &["scan", "--compiler-resolution-bridge", "--format", "json"],
    );
    assert!(scan.status.success(), "{}", text(&scan));
    assert!(String::from_utf8_lossy(&scan.stdout).contains("Shared"));
    let explain = run(
        &project,
        &["scan", "--explain", "--compiler-resolution-bridge"],
    );
    assert!(explain.status.success(), "{}", text(&explain));
    let plan = run(
        &project,
        &[
            "plan",
            "--compiler-resolution-bridge",
            "--default-status",
            "apply",
            "--min-referrers",
            "2",
        ],
    );
    assert!(plan.status.success(), "{}", text(&plan));
    assert!(project.join(".type-tune/plan.json").is_file());
    let before = snapshot(&project);
    let dry_run = run(
        &project,
        &[
            "apply",
            "--plan",
            ".type-tune/plan.json",
            "--accept-unreviewed",
            "--dry-run",
            "--compiler-resolution-bridge",
        ],
    );
    assert!(dry_run.status.success(), "{}", text(&dry_run));
    assert_eq!(before, snapshot(&project));
    let extract = run(
        &project,
        &[
            "extract",
            "Shared",
            "--from",
            "src/model.ts",
            "--dry-run",
            "--compiler-resolution-bridge",
        ],
    );
    assert!(extract.status.success(), "{}", text(&extract));
    assert_eq!(before, snapshot(&project));
    let dependency = temp.path().join("dependency");
    fs::create_dir_all(project.join("node_modules")).unwrap();
    fs::create_dir(&dependency).unwrap();
    std::os::unix::fs::symlink(&dependency, project.join("node_modules/external-pkg")).unwrap();
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        dependency.join("index.d.ts"),
        "export type { Hidden } from './hidden-source';\n",
    )
    .unwrap();
    fs::write(
        dependency.join("hidden-source.ts"),
        "export interface Hidden { leaked: true }\n",
    )
    .unwrap();
    fs::write(project.join("src/a.ts"), "import type { Shared } from './model';\nimport type { Hidden } from 'external-pkg';\nexport type A = [Shared, Hidden];\n").unwrap();
    let changed = snapshot(&project);
    let apply = run(
        &project,
        &[
            "apply",
            "--plan",
            ".type-tune/plan.json",
            "--accept-unreviewed",
            "--compiler-resolution-bridge",
        ],
    );
    let message = text(&apply);
    assert!(!apply.status.success(), "{message}");
    assert!(
        message.contains("outside the project boundary"),
        "{message}"
    );
    assert_eq!(changed, snapshot(&project), "{message}");
    let reads = fs::read_to_string(project.join("bridge-trace.json")).unwrap_or_default();
    assert!(
        !reads.contains("hidden-source.ts") || reads.contains("\"status\":\"unauthorized\""),
        "{reads}"
    );
    assert!(!reads.contains("leaked"), "{reads}");
}

#[test]
#[ignore = "requires Node.js and TypeScript 6"]
fn vue_sources_are_not_sent_through_an_incomplete_bridge() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    link_typescript(&project, "60");
    fs::write(project.join("tsconfig.json"), r#"{"include":["src"]}"#).unwrap();
    fs::write(project.join("src/main.ts"), "export type A = string;\n").unwrap();
    fs::write(
        project.join("src/App.vue"),
        "<script lang=\"ts\">export type A = string;</script>\n",
    )
    .unwrap();
    let output = run(&project, &["scan", "--compiler-resolution-bridge"]);
    let message = text(&output);
    assert!(!output.status.success(), "{message}");
    assert!(
        message.contains("Vue") || message.contains("framework"),
        "{message}"
    );
    assert!(output.stdout.is_empty(), "{message}");
}

#[test]
#[ignore = "requires Node.js and TypeScript 6"]
fn repeated_scans_record_bridge_cost_when_results_match() {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    link_typescript(&project, "60");
    fs::write(project.join("tsconfig.json"), r#"{"include":["src"]}"#).unwrap();
    fs::write(
        project.join("src/model.ts"),
        "export interface Shared { id: string }\n",
    )
    .unwrap();
    for name in ["a", "b", "c", "d"] {
        fs::write(
            project.join(format!("src/{name}.ts")),
            "import type { Shared } from './model';\nexport type View = Shared;\n",
        )
        .unwrap();
    }
    let native = run(
        &project,
        &["scan", "--format", "json", "--min-referrers", "2"],
    );
    let hybrid = run(
        &project,
        &[
            "scan",
            "--format",
            "json",
            "--min-referrers",
            "2",
            "--compiler-resolution-bridge",
        ],
    );
    assert!(native.status.success(), "{}", text(&native));
    assert!(hybrid.status.success(), "{}", text(&hybrid));
    let native_json: serde_json::Value = serde_json::from_slice(&native.stdout).unwrap();
    let hybrid_json: serde_json::Value = serde_json::from_slice(&hybrid.stdout).unwrap();
    assert_eq!(native_json["candidates"], hybrid_json["candidates"]);
    let mut native_ms = Vec::new();
    let mut hybrid_ms = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let output = run(
            &project,
            &["scan", "--format", "json", "--min-referrers", "2"],
        );
        assert!(output.status.success());
        native_ms.push(started.elapsed().as_millis());
        let started = Instant::now();
        let output = run(
            &project,
            &[
                "scan",
                "--format",
                "json",
                "--min-referrers",
                "2",
                "--compiler-resolution-bridge",
            ],
        );
        assert!(output.status.success());
        hybrid_ms.push(started.elapsed().as_millis());
    }
    let trace: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project.join("bridge-trace.json")).unwrap())
            .unwrap();
    eprintln!(
        "bridge-bench native_ms={native_ms:?} hybrid_ms={hybrid_ms:?} process_ms={} config_ms={} program_ms={} graph_ms={} node_peak_rss={}",
        trace["processMs"],
        trace["timingsMs"]["config"],
        trace["timingsMs"]["program"],
        trace["timingsMs"]["graph"],
        trace["peakRssBytes"]
    );
}
