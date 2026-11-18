#![cfg(unix)]

use std::{fs, os::unix::fs::symlink, process::Command};

fn run(project: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_type-tune"))
        .current_dir(project)
        .args(arguments)
        .output()
        .unwrap()
}

fn new_fixture() -> tempfile::TempDir {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("project/src")).unwrap();
    fs::create_dir(root.join("external")).unwrap();
    fs::write(
        root.join("external/model.ts"),
        "export interface External {}\n",
    )
    .unwrap();
    fs::write(
        root.join("project/tsconfig.json"),
        r#"{"include":["src/**/*.ts"]}"#,
    )
    .unwrap();
    fs::write(
        root.join("project/src/model.ts"),
        "export interface Shared { name: string }\n",
    )
    .unwrap();
    for name in ["first", "second"] {
        fs::write(root.join(format!("project/src/{name}.ts")), format!("import type {{ Shared }} from './model'; export const {name} = (x: Shared) => x.name;\n")).unwrap();
    }
    fixture
}

fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        entries: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                entries.insert(
                    relative,
                    format!("link:{}", fs::read_link(path).unwrap().display()).into_bytes(),
                );
            } else if meta.is_dir() {
                entries.insert(relative, b"directory".to_vec());
                walk(root, &path, entries);
            } else {
                entries.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut entries = std::collections::BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

#[test]
fn selected_symlinks_reject_all_workflows_before_compilation() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
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
    symlink(root.join("external"), project.join("src/external")).unwrap();
    fs::create_dir_all(project.join("node_modules/.bin")).unwrap();
    let compiler = project.join("node_modules/.bin/tsc");
    fs::write(&compiler, "#!/bin/sh\ntouch compiler-ran\nexit 0\n").unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
    let before = snapshot(root);
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
            output.stdout.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    }
    assert_eq!(snapshot(root), before);
    assert!(!project.join("compiler-ran").exists());
    assert!(!project.join("rejected.json").exists());
    assert!(!project.join("src/types").exists());
}

#[test]
fn excluded_and_unrelated_links_are_skipped_but_selected_aliases_fail() {
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
    symlink(root.join("external"), project.join("src/external")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"include":["src/**/*.ts"],"exclude":["src/external/**"]}"#,
    )
    .unwrap();
    let output = run(
        &project,
        &["scan", "--format", "json", "--min-referrers", "1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["candidates"].as_array().unwrap().len(), 1);
    fs::write(
        project.join("tsconfig.json"),
        r#"{"include":["src/model.ts","src/first.ts","src/second.ts"]}"#,
    )
    .unwrap();
    assert!(run(&project, &["scan"]).status.success());
    symlink(project.join("src/model.ts"), project.join("src/alias.ts")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/alias.ts"]}"#,
    )
    .unwrap();
    let output = run(&project, &["scan"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported source symlink"));
}

// Opening a FIFO for reading blocks until a writer connects. No writer exists here, so an
// accidental source-content read would make the child time out instead of reaching the diagnostic.
#[cfg(target_os = "linux")]
#[test]
fn external_file_is_rejected_without_opening_its_contents() {
    use std::{
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
    let fifo = root.join("external/fifo.ts");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    symlink(&fifo, project.join("src/fifo.ts")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_type-tune"))
        .current_dir(&project)
        .args(["scan", "--format", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("scanner blocked opening the external FIFO");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
}

#[test]
fn explicit_external_file_and_internal_cycles_are_rejected() {
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
    symlink(
        root.join("external/model.ts"),
        project.join("src/linked.ts"),
    )
    .unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["src/linked.ts"]}"#,
    )
    .unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    fs::remove_file(project.join("src/linked.ts")).unwrap();
    fs::write(project.join("tsconfig.json"), r#"{"include":["src"]}"#).unwrap();
    symlink(project.join("src"), project.join("src/cycle")).unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported source symlink"));
}

#[test]
fn rejects_chains_aliases_and_selected_broken_links() {
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
    symlink(root.join("external"), project.join("src/first-link")).unwrap();
    symlink("first-link", project.join("src/second-link")).unwrap();
    let output = run(&project, &["scan"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    fs::remove_file(project.join("src/first-link")).unwrap();
    fs::remove_file(project.join("src/second-link")).unwrap();
    symlink(project.join("src"), project.join("src/alias-a")).unwrap();
    symlink(project.join("src"), project.join("src/alias-b")).unwrap();
    let output = run(&project, &["scan"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported source symlink"));
    fs::remove_file(project.join("src/alias-a")).unwrap();
    fs::remove_file(project.join("src/alias-b")).unwrap();
    symlink(root.join("missing.ts"), project.join("src/missing.ts")).unwrap();
    let output = run(&project, &["scan"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot resolve relevant symlink"));
}

#[test]
fn explicitly_included_package_sources_are_bounded_but_excluded_packages_are_skipped() {
    let fixture = new_fixture();
    let root = fixture.path();
    let project = root.join("project");
    fs::create_dir(project.join("node_modules")).unwrap();
    symlink(
        root.join("external"),
        project.join("node_modules/linked-package"),
    )
    .unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"include":["node_modules/linked-package/**/*.ts"]}"#,
    )
    .unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    fs::write(project.join("tsconfig.json"), r#"{"include":["src/**/*.ts","node_modules/linked-package/**/*.ts"],"exclude":["node_modules/linked-package"]}"#).unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_file(project.join("node_modules/linked-package")).unwrap();
    symlink(
        root.join("missing-package"),
        project.join("node_modules/linked-package"),
    )
    .unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"files":["node_modules/linked-package/model.ts"]}"#,
    )
    .unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot resolve relevant path"));
    symlink(root.join("external"), project.join("src/node_modules")).unwrap();
    fs::write(
        project.join("tsconfig.json"),
        r#"{"include":["src/**/*.ts","**/node_modules/**/*.ts"],"exclude":["**/node_modules"]}"#,
    )
    .unwrap();
    let output = run(&project, &["scan", "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rejects_external_directory_and_file_sources_before_scanning() {
    for directory in [true, false] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let project = root.join("project");
        let external = root.join("external");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(
            project.join("tsconfig.json"),
            r#"{"include":["src/**/*.ts"]}"#,
        )
        .unwrap();
        fs::write(
            external.join("model.ts"),
            "export interface Shared { name: string }\n",
        )
        .unwrap();
        let import = if directory {
            symlink(&external, project.join("src/shared")).unwrap();
            "./shared/model"
        } else {
            symlink(external.join("model.ts"), project.join("src/model.ts")).unwrap();
            "./model"
        };
        for name in ["first", "second"] {
            fs::write(project.join(format!("src/{name}.ts")), format!("import type {{ Shared }} from '{import}'; export const {name} = (x: Shared) => x.name;\n")).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&project)
            .args([
                "scan",
                "--project",
                "tsconfig.json",
                "--format",
                "json",
                "--min-referrers",
                "1",
            ])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "scan unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("outside the project boundary"));
    }
}
