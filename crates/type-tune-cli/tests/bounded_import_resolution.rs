#![cfg(unix)]

use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "type-tune-import-boundary-{}-{}",
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
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(project: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_type-tune"))
        .current_dir(project)
        .args(args)
        .output()
        .unwrap()
}

fn snapshot(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &Path,
        dir: &Path,
        result: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.file_type().is_symlink() {
                result.insert(
                    relative,
                    format!("link:{}", fs::read_link(path).unwrap().display()).into_bytes(),
                );
            } else if metadata.is_dir() {
                result.insert(relative, b"directory".to_vec());
                visit(root, &path, result);
            } else {
                result.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut result = std::collections::BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn rejects_import_only_external_sources_and_expands_internal_sources() {
    for mode in [
        "directory",
        "file",
        "relative",
        "transitive",
        "internal",
        "internal-alias",
    ] {
        let temp = Fixture::new();
        let root = temp.path();
        let project = root.join("project");
        let src = project.join("src");
        let outside = root.join("outside");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(
            outside.join("model.ts"),
            "export interface Shared { value: string }\n",
        )
        .unwrap();
        fs::write(
            project.join("tsconfig.json"),
            r#"{"files":["src/main.ts"]}"#,
        )
        .unwrap();
        let specifier = match mode {
            "directory" => {
                symlink(&outside, src.join("linked")).unwrap();
                "./linked/model"
            }
            "file" => {
                symlink(outside.join("model.ts"), src.join("linked.ts")).unwrap();
                "./linked"
            }
            "relative" => "../../outside/model",
            "transitive" => {
                fs::write(
                    src.join("bridge.ts"),
                    "export type { Shared } from '../../outside/model';\n",
                )
                .unwrap();
                "./bridge"
            }
            "internal" => {
                fs::write(
                    src.join("model.ts"),
                    "export interface Shared { value: string }\n",
                )
                .unwrap();
                "./model"
            }
            _ => {
                fs::write(
                    src.join("model.ts"),
                    "export interface Shared { value: string }\n",
                )
                .unwrap();
                symlink(src.join("model.ts"), src.join("linked.ts")).unwrap();
                "./linked"
            }
        };
        fs::write(
            src.join("main.ts"),
            format!("import type {{ Shared }} from '{specifier}'; export type Use = Shared;\n"),
        )
        .unwrap();
        let before_main = fs::read(src.join("main.ts")).unwrap();
        let before_outside = fs::read(outside.join("model.ts")).unwrap();
        let output = run(
            &project,
            &["scan", "--format", "json", "--min-referrers", "1"],
        );
        if mode == "internal" {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("src/model.ts"));
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{mode}: {error}");
            assert!(
                error.contains("Import resolution cannot analyze"),
                "{mode}: {error}"
            );
            assert!(
                error.contains("canonical path:") && error.contains("project boundary:"),
                "{error}"
            );
            assert!(output.stdout.is_empty());
        }
        assert_eq!(fs::read(src.join("main.ts")).unwrap(), before_main);
        assert_eq!(fs::read(outside.join("model.ts")).unwrap(), before_outside);
    }
}

#[test]
fn changed_import_environment_rejects_workflows_before_writes() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    let outside = temp.path().join("outside");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("model.ts"), "export interface External {}\n").unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"include":["src/**/*.ts"],"exclude":["src/linked/**"]}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/model.ts"),
        "export interface Shared { name: string }\n",
    )
    .unwrap();
    for name in ["first", "second"] {
        fs::write(project.join(format!("src/{name}.ts")), format!("import type {{ Shared }} from './model'; export const {name} = (x: Shared) => x.name;\n")).unwrap();
    }
    let plan = run(
        &project,
        &[
            "plan",
            "--project",
            "tsconfig.json",
            "--out",
            "plan.json",
            "--default-status",
            "apply",
        ],
    );
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    symlink(&outside, project.join("src/linked")).unwrap();
    let original = fs::read_to_string(project.join("src/first.ts")).unwrap();
    fs::write(
        project.join("src/first.ts"),
        format!("{original}import type {{ External }} from './linked/model';\n"),
    )
    .unwrap();
    let before = fs::read(project.join("src/first.ts")).unwrap();
    let plan_before = fs::read(project.join("plan.json")).unwrap();
    let tree_before = snapshot(temp.path());
    for args in [
        vec!["scan", "--format", "json"],
        vec!["scan", "--explain"],
        vec!["plan", "--out", "rejected.json"],
        vec!["extract", "Shared", "--from", "src/model.ts"],
        vec!["apply", "--plan", "plan.json", "--accept-unreviewed"],
        vec![
            "apply",
            "--plan",
            "plan.json",
            "--accept-unreviewed",
            "--dry-run",
        ],
    ] {
        let output = run(&project, &args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Import resolution"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read(project.join("src/first.ts")).unwrap(), before);
    assert_eq!(fs::read(project.join("plan.json")).unwrap(), plan_before);
    assert_eq!(
        fs::read_to_string(outside.join("model.ts")).unwrap(),
        "export interface External {}\n"
    );
    assert_eq!(fs::read_link(project.join("src/linked")).unwrap(), outside);
    assert!(!project.join("rejected.json").exists());
    assert_eq!(snapshot(temp.path()), tree_before);
}

#[test]
fn checks_alias_side_effect_reexports_js_extensions_and_symlink_chains() {
    for (mode, statement) in [
        (
            "alias",
            "import type { Shared } from '@/linked/model'; export type Use = Shared;",
        ),
        ("side-effect", "import './linked/model';"),
        ("re-export", "export type { Shared } from './linked/model';"),
        (
            "js-extension",
            "import type { Shared } from './linked/model.js'; export type Use = Shared;",
        ),
        ("index", "export type { Shared } from './linked';"),
        (
            "chain",
            "import type { Shared } from './linked/model'; export type Use = Shared;",
        ),
    ] {
        let temp = Fixture::new();
        let project = temp.path().join("project");
        let outside = temp.path().join("outside");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(
            outside.join(if mode == "index" {
                "index.ts"
            } else {
                "model.ts"
            }),
            "export interface Shared {}\n",
        )
        .unwrap();
        if mode == "chain" {
            symlink(&outside, temp.path().join("link-two")).unwrap();
            symlink(temp.path().join("link-two"), project.join("src/linked")).unwrap();
        } else {
            symlink(&outside, project.join("src/linked")).unwrap();
        }
        fs::write(
            project.join("tsconfig.json"),
            r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}},"files":["src/main.ts"]}"#,
        )
        .unwrap();
        fs::write(project.join("src/main.ts"), statement).unwrap();
        let output = run(&project, &["scan"]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("outside the project boundary"),
            "{mode}: {diagnostic}"
        );
        assert!(diagnostic.contains("src/main.ts") && diagnostic.contains("canonical path:"));
    }
}

#[test]
fn unresolved_relative_imports_and_declarations_keep_existing_handling() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Missing } from './missing'; export type A = Missing;\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(
        project.join("src/missing.d.ts"),
        "export interface Missing {}\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn asset_side_effect_import_does_not_treat_file_as_a_directory() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(project.join("src/main.ts"), "import './style.css';\n").unwrap();
    fs::write(project.join("src/style.css"), "body {}\n").unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "declaration imports are not yet traversed; this demonstrates an unresolved boundary gap"]
fn dependency_declaration_cannot_hide_external_source() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    let dependency = temp.path().join("dependency");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("node_modules")).unwrap();
    fs::create_dir(&dependency).unwrap();
    symlink(&dependency, project.join("node_modules/pkg")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'pkg'; export type Use = Shared;\n",
    )
    .unwrap();
    fs::write(dependency.join("package.json"), r#"{"types":"index.d.ts"}"#).unwrap();
    fs::write(
        dependency.join("index.d.ts"),
        "export type { Shared } from './hidden';\n",
    )
    .unwrap();
    fs::write(dependency.join("hidden.ts"), "export interface Shared {}\n").unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        !output.status.success(),
        "external source through dependency declaration was silently omitted"
    );
}

#[test]
fn simple_symlinked_package_source_is_rejected_but_declarations_are_allowed() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    let dependency = temp.path().join("dependency");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("node_modules")).unwrap();
    fs::create_dir(&dependency).unwrap();
    symlink(&dependency, project.join("node_modules/external-pkg")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/main.ts"]}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'external-pkg'; export type Use = Shared;\n",
    )
    .unwrap();
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","types":"index.ts"}"#,
    )
    .unwrap();
    fs::write(dependency.join("index.ts"), "export interface Shared {}\n").unwrap();
    let output = run(&project, &["scan"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        dependency.join("index.d.ts"),
        "export interface Shared {}\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn conditional_package_export_must_not_silently_omit_external_source() {
    let temp = Fixture::new();
    let project = temp.path().join("project");
    let dependency = temp.path().join("dependency");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("node_modules")).unwrap();
    fs::create_dir(&dependency).unwrap();
    symlink(&dependency, project.join("node_modules/external-pkg")).unwrap();
    fs::write(dependency.join("index.ts"), "export interface Shared {}\n").unwrap();
    fs::write(project.join("tsconfig.json"), r#"{"compilerOptions":{"module":"esnext","moduleResolution":"bundler"},"files":["src/main.ts"]}"#).unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'external-pkg'; export type Use = Shared;\n",
    )
    .unwrap();
    for manifest in [
        r#"{"name":"external-pkg","exports":{".":{"types":"./index.ts"}}}"#,
        r#"{"name":"external-pkg","exports":{".":{"import":{"types":"./index.ts"}}}}"#,
        r#"{"name":"external-pkg","exports":"./index.ts"}"#,
    ] {
        fs::write(dependency.join("package.json"), manifest).unwrap();
        let output = run(&project, &["scan"]);
        assert!(
            !output.status.success(),
            "conditional export source was silently omitted"
        );
    }
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","exports":{"./thing":{"types":"./index.ts"}}}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'external-pkg/thing'; export type Use = Shared;\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        !output.status.success(),
        "package subpath source was silently omitted"
    );
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","exports":{"./*":{"types":"./*.ts"}}}"#,
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'external-pkg/index'; export type Use = Shared;\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        !output.status.success(),
        "wildcard export source was silently omitted"
    );
    fs::write(
        dependency.join("package.json"),
        r#"{"name":"external-pkg","typesVersions":{"*":{"*":["src/*.ts"]}}}"#,
    )
    .unwrap();
    fs::create_dir(dependency.join("src")).unwrap();
    fs::write(
        dependency.join("src/item.ts"),
        "export interface Shared {}\n",
    )
    .unwrap();
    fs::write(
        project.join("src/main.ts"),
        "import type { Shared } from 'external-pkg/item'; export type Use = Shared;\n",
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert!(
        !output.status.success(),
        "typesVersions source was silently omitted"
    );
}
