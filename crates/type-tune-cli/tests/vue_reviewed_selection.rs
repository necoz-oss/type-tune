//! Partial Vue plan approval. Ignored candidates stay in validation and stay out of the rewrite.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Fixture {
    fn pair() -> Self {
        let root = std::env::temp_dir().join(format!(
            "type-tune-vue-selection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let fixture = Self(root);
        fixture.write(
            "tsconfig.json",
            r#"{"compilerOptions":{"strict":true,"noEmit":true,"module":"ESNext","moduleResolution":"Bundler"},"include":["src"]}"#,
        );
        fixture.write("src/alpha/a.ts", "export type A = { id: string }\n");
        fixture.write(
            "src/alpha/use-a.ts",
            "import type { A } from './a';\nexport const useA: A = { id: 'a' };\n",
        );
        fixture.write(
            "src/alpha/A.vue",
            "<script setup lang=\"ts\">\nimport type { A } from \"./a\";\ndefineProps<{ item: A }>();\n</script>\n<template><p>A</p></template>\n",
        );
        fixture.write("src/beta/b.ts", "export type B = { id: string }\n");
        fixture.write(
            "src/beta/use-b.ts",
            "import type { B } from './b';\nexport const useB: B = { id: 'b' };\n",
        );
        fixture.write(
            "src/beta/B.vue",
            "<script setup lang=\"ts\">\nimport type { B } from \"./b\";\ndefineProps<{ item: B }>();\n</script>\n<template><p>B</p></template>\n",
        );
        fixture
    }

    fn write(&self, path: &str, content: &str) {
        let file = self.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_path(args, None)
    }

    fn run_with_path(&self, args: &[&str], path: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_type-tune"));
        command.current_dir(&self.0).args(args);
        if let Some(path) = path {
            command.env("PATH", path);
        }
        command.output().unwrap()
    }
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn stub_compilers(root: &Path) {
    for name in ["tsc", "vue-tsc"] {
        let directory = root.join("node_modules/.bin");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, "#!/bin/sh\nif [ \"$1\" = --showConfig ]; then echo '{\"compilerOptions\":{}}'; fi\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).unwrap();
        }
    }
}

fn plan(fixture: &Fixture) {
    let planned = fixture.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
        "--min-referrers",
        "2",
        "--default-status",
        "ignore",
    ]);
    assert!(planned.status.success(), "{}", output_text(&planned));
}

fn plan_json(fixture: &Fixture) -> serde_json::Value {
    serde_json::from_slice(&fs::read(fixture.0.join("plan.json")).unwrap()).unwrap()
}

fn set_json_status(fixture: &Fixture, statuses: &[(&str, &str)]) {
    let mut plan = plan_json(fixture);
    for entry in plan["entries"].as_array_mut().unwrap() {
        let name = entry["candidate"]["symbolName"].as_str().unwrap();
        if let Some((_, status)) = statuses.iter().find(|(symbol, _)| *symbol == name) {
            entry["status"] = serde_json::Value::String((*status).to_owned());
        }
    }
    fs::write(
        fixture.0.join("plan.json"),
        serde_json::to_vec_pretty(&plan).unwrap(),
    )
    .unwrap();
}

fn approve_markdown(fixture: &Fixture, symbol: &str) {
    let markdown = fs::read_to_string(fixture.0.join("plan.md")).unwrap();
    let plan = plan_json(fixture);
    let entry = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == symbol)
        .unwrap();
    let id = entry["candidate"]["id"].as_str().unwrap();
    let reviewed = markdown.replace(
        &format!("- [ignore] {id} `{symbol}`"),
        &format!("- [apply] {id} `{symbol}`"),
    );
    assert_ne!(reviewed, markdown);
    fixture.write("plan.md", &reviewed);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectSnapshot {
    directories: BTreeSet<String>,
    files: BTreeMap<String, Vec<u8>>,
}

fn snapshot(root: &Path) -> ProjectSnapshot {
    let mut snapshot = ProjectSnapshot {
        directories: BTreeSet::new(),
        files: BTreeMap::new(),
    };
    collect(root, root, &mut snapshot);
    snapshot
}

fn collect(root: &Path, dir: &Path, snapshot: &mut ProjectSnapshot) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if entry.file_name() == "node_modules" {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("could not inspect {}: {error}", path.display()));
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if metadata.file_type().is_symlink() {
            panic!("unexpected symlink {relative}");
        }
        if metadata.is_dir() {
            snapshot.directories.insert(relative);
            collect(root, &path, snapshot);
            continue;
        }
        snapshot.files.insert(
            relative,
            fs::read(&path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display())),
        );
    }
}

fn assert_unchanged(fixture: &Fixture, before: &ProjectSnapshot) {
    assert_eq!(snapshot(&fixture.0), *before);
    let leftovers = snapshot(&fixture.0)
        .files
        .keys()
        .chain(snapshot(&fixture.0).directories.iter())
        .filter(|path| path.contains(".type-tune-"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn markdown_approval_moves_only_a_and_restores_the_dry_run() {
    let fixture = Fixture::pair();
    stub_compilers(&fixture.0);
    plan(&fixture);
    approve_markdown(&fixture, "A");
    let before = snapshot(&fixture.0);
    let dry_run = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry_run.status.success(), "{}", output_text(&dry_run));
    assert_unchanged(&fixture, &before);

    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let text = output_text(&apply);
    assert!(text.contains("Applied 1 candidate"), "{text}");
    assert!(text.contains("1 ignored"), "{text}");
    let updated = fs::read_to_string(fixture.0.join("src/alpha/A.vue")).unwrap();
    assert!(updated.contains("./types/alpha"), "{updated}");
    assert!(updated.contains("<p>A</p>"), "{updated}");
    assert_eq!(
        fs::read(fixture.0.join("src/beta/B.vue")).unwrap(),
        before.files["src/beta/B.vue"]
    );
    assert_eq!(
        fs::read(fixture.0.join("src/beta/b.ts")).unwrap(),
        before.files["src/beta/b.ts"]
    );
    assert_eq!(
        fs::read(fixture.0.join("src/beta/use-b.ts")).unwrap(),
        before.files["src/beta/use-b.ts"]
    );
}

#[test]
fn json_approval_moves_only_b() {
    let fixture = Fixture::pair();
    stub_compilers(&fixture.0);
    plan(&fixture);
    set_json_status(&fixture, &[("B", "apply")]);
    let before_a = fs::read(fixture.0.join("src/alpha/A.vue")).unwrap();
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(output_text(&apply).contains("Applied 1 candidate"));
    let updated = fs::read_to_string(fixture.0.join("src/beta/B.vue")).unwrap();
    assert!(updated.contains("./types/beta"), "{updated}");
    assert_eq!(
        fs::read(fixture.0.join("src/alpha/A.vue")).unwrap(),
        before_a
    );
}

#[test]
fn json_approval_moves_both_vue_candidates() {
    let fixture = Fixture::pair();
    stub_compilers(&fixture.0);
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply"), ("B", "apply")]);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(output_text(&apply).contains("Applied 2 candidates"));
    assert!(
        fs::read_to_string(fixture.0.join("src/alpha/A.vue"))
            .unwrap()
            .contains("./types/alpha")
    );
    assert!(
        fs::read_to_string(fixture.0.join("src/beta/B.vue"))
            .unwrap()
            .contains("./types/beta")
    );
}

#[test]
fn ignored_plan_is_a_noop() {
    let fixture = Fixture::pair();
    plan(&fixture);
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(output_text(&apply).contains("Applied 0 candidates"));
    assert!(output_text(&apply).contains("2 ignored"));
    assert_unchanged(&fixture, &before);
}

#[test]
fn typescript_only_approval_ignores_vue_candidates_without_vue_tsc() {
    let fixture = Fixture::pair();
    fixture.write("src/gamma/c.ts", "export type C = { id: string }\n");
    fixture.write(
        "src/gamma/one.ts",
        "import type { C } from './c';\nexport type One = C;\n",
    );
    fixture.write(
        "src/gamma/two.ts",
        "import type { C } from './c';\nexport type Two = C;\n",
    );
    let directory = fixture.0.join("node_modules/.bin");
    fs::create_dir_all(&directory).unwrap();
    let tsc = directory.join("tsc");
    fs::write(&tsc, "#!/bin/sh\nif [ \"$1\" = --showConfig ]; then echo '{\"compilerOptions\":{}}'; fi\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&tsc).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&tsc, permissions).unwrap();
    }
    plan(&fixture);
    set_json_status(&fixture, &[("C", "apply")]);
    let before_vue = fs::read(fixture.0.join("src/alpha/A.vue")).unwrap();
    let before_beta = fs::read(fixture.0.join("src/beta/B.vue")).unwrap();
    let apply = fixture.run_with_path(
        &["apply", "--plan", "plan.json", "--accept-unreviewed"],
        Some(""),
    );
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(output_text(&apply).contains("Applied 1 candidate"));
    assert!(fixture.0.join("src/gamma/types/gamma.ts").is_file());
    assert_eq!(
        fs::read(fixture.0.join("src/alpha/A.vue")).unwrap(),
        before_vue
    );
    assert_eq!(
        fs::read(fixture.0.join("src/beta/B.vue")).unwrap(),
        before_beta
    );
}

#[test]
fn ignored_namespace_import_does_not_block_an_approved_named_import() {
    let fixture = Fixture::pair();
    fixture.write(
        "src/beta/B.vue",
        "<script setup lang=\"ts\">\nimport type * as Beta from \"./b\";\ndefineProps<{ item: Beta.B }>();\n</script>\n<template><p>B</p></template>\n",
    );
    stub_compilers(&fixture.0);
    let scan = fixture.run(&[
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "1",
    ]);
    assert!(scan.status.success(), "{}", output_text(&scan));
    let json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let beta = json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["symbolName"] == "B")
        .unwrap();
    assert!(beta.get("vueScriptStrategies").is_none());
    assert!(
        beta["vueRewriteBlockers"][0]["message"]
            .as_str()
            .unwrap()
            .contains("namespace or default")
    );
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    let before_b = fs::read(fixture.0.join("src/beta/B.vue")).unwrap();
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert_eq!(
        fs::read(fixture.0.join("src/beta/B.vue")).unwrap(),
        before_b
    );
    assert!(
        fs::read_to_string(fixture.0.join("src/alpha/A.vue"))
            .unwrap()
            .contains("./types/alpha")
    );
}

#[test]
fn approved_namespace_import_is_rejected_without_writes() {
    let fixture = Fixture::pair();
    fixture.write(
        "src/beta/B.vue",
        "<script setup lang=\"ts\">\nimport type * as Beta from \"./b\";\ndefineProps<{ item: Beta.B }>();\n</script>\n<template><p>B</p></template>\n",
    );
    plan(&fixture);
    set_json_status(&fixture, &[("B", "apply")]);
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    let text = output_text(&apply);
    assert!(text.contains("namespace or default import"), "{text}");
    assert!(text.contains("before any files were changed"), "{text}");
    assert_unchanged(&fixture, &before);
}

#[test]
fn unresolved_vue_component_still_rejects_every_approved_move() {
    let fixture = Fixture::pair();
    fixture.write(
        "src/Plain.vue",
        "<template><p>plain</p></template>\n<script>\nexport default { name: getName() }\n</script>\n",
    );
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    let text = output_text(&apply);
    assert!(text.contains("src/Plain.vue"), "{text}");
    assert!(
        text.contains("dependencies cannot be established safely"),
        "{text}"
    );
    assert!(text.contains("Correct the unsupported script"), "{text}");
    assert!(text.contains("exclude the component"), "{text}");
    assert!(
        !text.contains("Vue rewriting is not supported yet"),
        "{text}"
    );
    assert_unchanged(&fixture, &before);
}

#[test]
fn approving_one_import_in_a_shared_component_leaves_the_other_specifier() {
    let fixture = Fixture::pair();
    fixture.write(
        "src/shared/Card.vue",
        "<script setup lang=\"ts\">\nimport type { A } from \"../alpha/a\";\nimport type { B } from \"../beta/b\";\ndefineProps<{ a: A; b: B }>();\n</script>\n<template><p>shared</p></template>\n",
    );
    stub_compilers(&fixture.0);
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    let before_b = fs::read_to_string(fixture.0.join("src/beta/B.vue")).unwrap();
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let card = fs::read_to_string(fixture.0.join("src/shared/Card.vue")).unwrap();
    assert!(card.contains("../alpha/types/alpha"), "{card}");
    assert!(card.contains("from \"../beta/b\""), "{card}");
    assert!(card.contains("<p>shared</p>"), "{card}");
    assert_eq!(
        fs::read_to_string(fixture.0.join("src/beta/B.vue")).unwrap(),
        before_b
    );
}

#[test]
fn mixed_import_with_an_unmoved_symbol_is_still_rejected() {
    let fixture = Fixture::pair();
    fixture.write(
        "src/alpha/a.ts",
        "export type A = { id: string }\nexport type Kept = { id: string }\n",
    );
    fixture.write(
        "src/alpha/A.vue",
        "<script setup lang=\"ts\">\nimport type { A, Kept } from \"./a\";\ndefineProps<{ item: A; kept: Kept }>();\n</script>\n<template><p>A</p></template>\n",
    );
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    let text = output_text(&apply);
    assert!(
        text.contains("not part of this move") || text.contains("without a Vue script strategy"),
        "{text}"
    );
    assert!(text.contains("before any files were changed"), "{text}");
    assert_unchanged(&fixture, &before);
}

fn mutate_entry(fixture: &Fixture, symbol: &str, mutate: impl FnOnce(&mut serde_json::Value)) {
    let mut plan = plan_json(fixture);
    let entry = plan["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["candidate"]["symbolName"] == symbol)
        .unwrap_or_else(|| panic!("missing {symbol}"));
    mutate(entry);
    fs::write(
        fixture.0.join("plan.json"),
        serde_json::to_vec_pretty(&plan).unwrap(),
    )
    .unwrap();
}

#[test]
fn stale_fingerprint_of_an_approved_candidate_is_rejected() {
    let fixture = Fixture::pair();
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    mutate_entry(&fixture, "A", |entry| {
        entry["sourceFingerprint"] = serde_json::Value::String("tampered".into());
    });
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("Stale plan entry"),
        "{}",
        output_text(&apply)
    );
    assert_unchanged(&fixture, &before);
}

#[test]
fn stale_fingerprint_of_an_ignored_candidate_is_rejected() {
    let fixture = Fixture::pair();
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    mutate_entry(&fixture, "B", |entry| {
        entry["sourceFingerprint"] = serde_json::Value::String("tampered".into());
    });
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("Stale plan entry"),
        "{}",
        output_text(&apply)
    );
    assert_unchanged(&fixture, &before);
}

#[test]
fn stale_ignored_source_content_is_rejected() {
    let fixture = Fixture::pair();
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    fixture.write(
        "src/beta/b.ts",
        "export type B = { id: string; extra: string }\n",
    );
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("Stale plan entry"),
        "{}",
        output_text(&apply)
    );
    assert_unchanged(&fixture, &before);
}

#[test]
fn invalid_ignored_candidate_metadata_is_rejected() {
    let fixture = Fixture::pair();
    plan(&fixture);
    set_json_status(&fixture, &[("A", "apply")]);
    mutate_entry(&fixture, "B", |entry| {
        entry["candidate"]["symbolName"] = serde_json::Value::String("Tampered".into());
    });
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("Invalid candidate metadata"),
        "{}",
        output_text(&apply)
    );
    assert_unchanged(&fixture, &before);
}

#[test]
fn ignored_renamed_barrel_does_not_block_an_approved_move() {
    let fixture = Fixture::pair();
    stub_compilers(&fixture.0);
    fixture.write("src/models.ts", "export interface User { id: string }\n");
    fixture.write(
        "src/index.ts",
        "export type { User as PublicUser } from \"./models\";\n",
    );
    fixture.write(
        "src/use-user.ts",
        "import type { User } from './models';\nexport type App = User;\n",
    );
    fixture.write(
        "src/Card.vue",
        "<script setup lang=\"ts\">\nimport type { PublicUser } from \"./index\";\ndefineProps<{ user: PublicUser }>();\n</script>\n",
    );
    plan(&fixture);
    let planned = plan_json(&fixture);
    let user = planned["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == "User")
        .expect("user");
    assert!(
        user["candidate"]
            .get("vueScriptStrategies")
            .is_none_or(|strategies| strategies.as_array().is_some_and(Vec::is_empty)),
        "{user}"
    );
    assert!(
        user["candidate"]["vueRewriteBlockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["path"] == "src/Card.vue"),
        "{user}"
    );
    set_json_status(&fixture, &[("A", "apply")]);
    let before_card = fs::read(fixture.0.join("src/Card.vue")).unwrap();
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("Applied 1 candidate"),
        "{}",
        output_text(&apply)
    );
    assert!(
        fs::read_to_string(fixture.0.join("src/alpha/A.vue"))
            .unwrap()
            .contains("./types/alpha")
    );
    assert_eq!(
        fs::read(fixture.0.join("src/Card.vue")).unwrap(),
        before_card
    );
}

#[test]
fn approved_renamed_barrel_is_rejected_without_writes() {
    let fixture = Fixture::pair();
    fixture.write("src/models.ts", "export interface User { id: string }\n");
    fixture.write(
        "src/index.ts",
        "export type { User as PublicUser } from \"./models\";\n",
    );
    fixture.write(
        "src/use-user.ts",
        "import type { User } from './models';\nexport type App = User;\n",
    );
    fixture.write(
        "src/Card.vue",
        "<script setup lang=\"ts\">\nimport type { PublicUser } from \"./index\";\ndefineProps<{ user: PublicUser }>();\n</script>\n",
    );
    plan(&fixture);
    set_json_status(&fixture, &[("User", "apply")]);
    let before = snapshot(&fixture.0);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--accept-unreviewed"]);
    assert!(!apply.status.success(), "{}", output_text(&apply));
    assert!(
        output_text(&apply).contains("barrel"),
        "{}",
        output_text(&apply)
    );
    assert!(
        output_text(&apply).contains("before any files were changed"),
        "{}",
        output_text(&apply)
    );
    assert_unchanged(&fixture, &before);
}
