use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "type-tune-vue-referrers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let fixture = Self(root);
        fixture.write(
            "tsconfig.json",
            r#"{"compilerOptions":{"strict":true,"noEmit":true},"include":["src"]}"#,
        );
        fixture.write("src/model.ts", "export interface User { id: string }\n");
        fixture.write(
            "src/one.ts",
            "import type { User } from './model';\nexport type One = User;\n",
        );
        fixture.write(
            "src/two.ts",
            "import type { User } from './model';\nexport type Two = User;\n",
        );
        fixture.write(
            "src/Card.vue",
            "<template><p>User</p></template>\n<script setup lang=\"ts\">\nimport type { User } from \"./model\";\ndefineProps<{ user: User }>();\n</script>\n<style>/* User */</style>\n",
        );
        fixture
    }

    fn write(&self, path: &str, content: &str) {
        let file = self.0.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        for path in ["src/model.ts", "src/one.ts", "src/two.ts", "src/Card.vue"] {
            files.push((self.0.join(path), fs::read(self.0.join(path)).unwrap()));
        }
        files
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

#[test]
fn scan_reports_vue_referrers_and_apply_updates_script_specifiers() {
    let fixture = Fixture::new();
    let scan = fixture.run(&[
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "2",
    ]);
    assert!(scan.status.success(), "{}", output_text(&scan));
    let json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let user = json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["symbolName"] == "User")
        .expect("user candidate");
    assert_eq!(user["referrerCount"], 3);
    assert_eq!(user["readonlyReferrers"][0]["path"], "src/Card.vue");
    assert_eq!(user["readonlyReferrers"][0]["reason"], "vueSfc");
    assert!(
        user["referrerStrategies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|strategy| strategy["referrerPath"] != "src/Card.vue")
    );

    let plan = fixture.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
        "--min-referrers",
        "2",
    ]);
    assert!(plan.status.success(), "{}", output_text(&plan));
    let markdown = fs::read_to_string(fixture.0.join("plan.md")).unwrap();
    assert!(markdown.contains("src/Card.vue"), "{markdown}");
    assert!(markdown.contains("vue script referrers"), "{markdown}");
    let plan_json: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("plan.json")).unwrap()).unwrap();
    let user = plan_json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == "User")
        .unwrap();
    let id = user["candidate"]["id"].as_str().unwrap();
    let destination = user["candidate"]["destinationPath"].as_str().unwrap();
    let mut reviewed = markdown.replace(
        &format!("- [ignore] {id} `User`"),
        &format!("- [apply] {id} `User`"),
    );
    reviewed = reviewed.replace(
        &format!("  - to: `{destination}`"),
        "  - to: `src/domain/user.ts`",
    );
    fixture.write("plan.md", &reviewed);

    write_compiler_stub(&fixture.0, "tsc", "#!/bin/sh\nexit 0\n");
    write_compiler_stub(
        &fixture.0,
        "vue-tsc",
        "#!/bin/sh\necho 'Card.vue(4,1): error TS2322: bad' >&2\nexit 1\n",
    );
    let before = fixture.snapshot();
    for args in [
        vec![
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ],
        vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"],
    ] {
        let failed = fixture.run_with_path(&args, Some(""));
        assert!(!failed.status.success(), "{}", output_text(&failed));
        let failed_text = output_text(&failed);
        assert!(
            failed_text.contains("vue-tsc validation failed"),
            "{failed_text}"
        );
        assert!(
            failed_text.contains("rolled back file changes"),
            "{failed_text}"
        );
        assert!(!failed_text.contains("rollback failed"), "{failed_text}");
        assert!(failed_text.contains("TS2322"), "{failed_text}");
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.0.join("src/domain").exists());
        assert!(fixture_has_no_transaction_files(&fixture.0));
    }

    write_compiler_stub(&fixture.0, "vue-tsc", "#!/bin/sh\nexit 0\n");
    let dry_run = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry_run.status.success(), "{}", output_text(&dry_run));
    assert_eq!(fixture.snapshot(), before);
    assert!(!fixture.0.join("src/domain").exists());

    let extract = fixture.run(&[
        "extract",
        "User",
        "--from",
        "src/model.ts",
        "--to",
        "src/domain/user.ts",
        "--project",
        "tsconfig.json",
        "--dry-run",
    ]);
    assert!(extract.status.success(), "{}", output_text(&extract));
    assert_eq!(fixture.snapshot(), before);

    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let vue = fs::read_to_string(fixture.0.join("src/Card.vue")).unwrap();
    assert!(vue.contains("<template><p>User</p></template>\n"), "{vue}");
    assert!(vue.contains("<style>/* User */</style>\n"), "{vue}");
    assert!(vue.contains("\"./domain/user\""), "{vue}");
    assert!(!vue.contains("\"./model\""), "{vue}");
    assert!(fixture.0.join("src/domain/user.ts").is_file());
}

fn write_compiler_stub(root: &std::path::Path, name: &str, body: &str) {
    let directory = root.join("node_modules/.bin");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

#[test]
fn unresolved_vue_file_blocks_an_otherwise_safe_move_without_writing() {
    let fixture = Fixture::new();
    fixture.write(
        "src/Card.vue",
        "<script setup lang=\"ts\">\nconst label = \"unused\";\n</script>\n",
    );
    fixture.write(
        "src/Plain.vue",
        "<template><p>label</p></template>\n<script>\nexport default { name: 'Plain' }\n</script>\n",
    );
    let before = fixture.snapshot();
    let plain_before = fs::read(fixture.0.join("src/Plain.vue")).unwrap();

    let scan = fixture.run(&[
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "2",
    ]);
    assert!(scan.status.success(), "{}", output_text(&scan));
    let scan_json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert!(scan_json.get("analysis_limitations").is_none());
    assert!(scan_json.get("diagnostics").is_none());
    let limitations = scan_json["analysisLimitations"].as_array().unwrap();
    assert_eq!(limitations.len(), 1);
    assert_eq!(limitations[0]["path"], "src/Plain.vue");
    assert_eq!(limitations[0]["reason"], "unresolvedVueSfc");
    assert!(
        limitations[0]["message"]
            .as_str()
            .unwrap()
            .contains("without lang=\"ts\"")
    );
    let user = scan_json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["symbolName"] == "User")
        .unwrap();
    assert_eq!(user["referrerCount"], 2);
    assert!(user.get("readonlyReferrers").is_none());
    assert_eq!(
        user["analysisLimitations"],
        scan_json["analysisLimitations"]
    );
    assert!(
        user["referrerPaths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| path != "src/Plain.vue")
    );

    let explain = fixture.run(&[
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "2",
        "--explain",
    ]);
    assert!(explain.status.success(), "{}", output_text(&explain));
    let explain_json: serde_json::Value = serde_json::from_slice(&explain.stdout).unwrap();
    assert_eq!(
        explain_json["analysisLimitations"],
        scan_json["analysisLimitations"]
    );
    assert!(explain_json.get("analysis_limitations").is_none());
    assert!(explain_json["diagnostics"].is_array());

    for args in [
        vec!["scan", "--project", "tsconfig.json", "--min-referrers", "2"],
        vec![
            "scan",
            "--project",
            "tsconfig.json",
            "--min-referrers",
            "2",
            "--explain",
        ],
    ] {
        let text = fixture.run(&args);
        assert!(text.status.success(), "{}", output_text(&text));
        let rendered = output_text(&text);
        assert!(rendered.contains("Vue analysis limitations:"), "{rendered}");
        assert!(rendered.contains("src/Plain.vue"), "{rendered}");
        assert!(rendered.contains("without lang=\"ts\""), "{rendered}");
    }

    let plan = fixture.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
        "--min-referrers",
        "2",
    ]);
    assert!(plan.status.success(), "{}", output_text(&plan));
    let markdown = fs::read_to_string(fixture.0.join("plan.md")).unwrap();
    let plan_json: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("plan.json")).unwrap()).unwrap();
    let user = plan_json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == "User")
        .unwrap();
    let id = user["candidate"]["id"].as_str().unwrap();
    let destination = user["candidate"]["destinationPath"].as_str().unwrap();
    let mut reviewed = markdown.replace(
        &format!("- [ignore] {id} `User`"),
        &format!("- [apply] {id} `User`"),
    );
    reviewed = reviewed.replace(
        &format!("  - to: `{destination}`"),
        "  - to: `src/domain/user.ts`",
    );
    fixture.write("plan.md", &reviewed);

    for args in [
        vec![
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ],
        vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"],
    ] {
        let apply = fixture.run(&args);
        assert!(!apply.status.success(), "{}", output_text(&apply));
        let text = output_text(&apply);
        assert!(text.contains("before any files were changed"), "{text}");
        assert!(text.contains("src/Plain.vue"), "{text}");
        assert_eq!(fixture.snapshot(), before);
        assert_eq!(
            fs::read(fixture.0.join("src/Plain.vue")).unwrap(),
            plain_before
        );
        assert!(!fixture.0.join("src/domain").exists());
    }
}

#[test]
fn missing_vue_tsc_rolls_back_a_vue_edit_and_typescript_only_moves_do_not_need_it() {
    let fixture = Fixture::new();
    let plan = fixture.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
        "--min-referrers",
        "2",
    ]);
    assert!(plan.status.success(), "{}", output_text(&plan));
    select_user(&fixture, "src/domain/user.ts");
    write_compiler_stub(&fixture.0, "tsc", "#!/bin/sh\nexit 0\n");
    assert!(!fixture.0.join("node_modules/.bin/vue-tsc").exists());
    let before = fixture.snapshot();

    for args in [
        vec![
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ],
        vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"],
    ] {
        let missing = fixture.run_with_path(&args, Some(""));
        assert!(!missing.status.success(), "{}", output_text(&missing));
        let text = output_text(&missing);
        assert!(text.contains("Vue script edits require vue-tsc"), "{text}");
        assert!(text.contains("could not be started"), "{text}");
        assert!(text.contains("Install vue-tsc"), "{text}");
        assert!(!text.contains("The rewrite was rolled back."), "{text}");
        assert!(text.contains("rolled back file changes"), "{text}");
        assert!(!text.contains("rollback failed"), "{text}");
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.0.join("src/domain").exists());
        assert!(fixture_has_no_transaction_files(&fixture.0));
    }

    let typescript_only = Fixture::new();
    typescript_only.write(
        "src/Card.vue",
        "<script setup lang=\"ts\">\nconst label = \"unused\";\n</script>\n",
    );
    let plan = typescript_only.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
        "--min-referrers",
        "2",
    ]);
    assert!(plan.status.success(), "{}", output_text(&plan));
    select_user(&typescript_only, "src/domain/user.ts");
    write_compiler_stub(&typescript_only.0, "tsc", "#!/bin/sh\nexit 0\n");
    let vue_before = fs::read(typescript_only.0.join("src/Card.vue")).unwrap();
    let dry_run = typescript_only.run_with_path(
        &[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ],
        Some(""),
    );
    assert!(dry_run.status.success(), "{}", output_text(&dry_run));
    assert!(!typescript_only.0.join("src/domain").exists());
    assert_eq!(
        fs::read(typescript_only.0.join("src/Card.vue")).unwrap(),
        vue_before
    );
    let applied = typescript_only.run_with_path(
        &["apply", "--plan", "plan.json", "--markdown", "plan.md"],
        Some(""),
    );
    assert!(applied.status.success(), "{}", output_text(&applied));
    assert!(typescript_only.0.join("src/domain/user.ts").is_file());
    assert_eq!(
        fs::read(typescript_only.0.join("src/Card.vue")).unwrap(),
        vue_before
    );
}

#[test]
fn vue_validation_failure_reports_a_failed_rollback_without_claiming_restoration() {
    for dry_run in [true, false] {
        let fixture = Fixture::new();
        let plan = fixture.run(&[
            "plan",
            "--project",
            "tsconfig.json",
            "--out",
            "plan.json",
            "--markdown",
            "plan.md",
            "--min-referrers",
            "2",
        ]);
        assert!(plan.status.success(), "{}", output_text(&plan));
        select_user(&fixture, "src/domain/user.ts");
        write_compiler_stub(&fixture.0, "tsc", "#!/bin/sh\nexit 0\n");
        write_compiler_stub(
            &fixture.0,
            "vue-tsc",
            "#!/bin/sh\n/bin/rm -f src/Card.vue\n/bin/mkdir src/Card.vue\necho 'Card.vue(4,1): error TS2322: blocked' >&2\nexit 1\n",
        );
        let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
        if dry_run {
            args.push("--dry-run");
        }
        let failed = fixture.run_with_path(&args, Some(""));
        assert!(!failed.status.success(), "{}", output_text(&failed));
        let text = output_text(&failed);
        assert!(text.contains("vue-tsc validation failed"), "{text}");
        assert!(text.contains("TS2322"), "{text}");
        assert!(text.contains("rollback failed"), "{text}");
        assert!(!text.contains("rolled back file changes"), "{text}");
        assert!(!text.contains("The rewrite was rolled back."), "{text}");
        assert!(
            fixture.0.join("src/Card.vue").is_dir(),
            "restoration must remain incomplete when rollback fails"
        );
    }
}

fn fixture_has_no_transaction_files(root: &std::path::Path) -> bool {
    fn walk(dir: &std::path::Path) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return true;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "node_modules" {
                continue;
            }
            if name.contains(".type-tune-") {
                return false;
            }
            if entry.path().is_dir() && !walk(&entry.path()) {
                return false;
            }
        }
        true
    }
    walk(root)
}

fn select_user(fixture: &Fixture, destination: &str) {
    let markdown = fs::read_to_string(fixture.0.join("plan.md")).unwrap();
    let plan_json: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.0.join("plan.json")).unwrap()).unwrap();
    let user = plan_json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == "User")
        .unwrap();
    let id = user["candidate"]["id"].as_str().unwrap();
    let current = user["candidate"]["destinationPath"].as_str().unwrap();
    let reviewed = markdown
        .replace(
            &format!("- [ignore] {id} `User`"),
            &format!("- [apply] {id} `User`"),
        )
        .replace(
            &format!("  - to: `{current}`"),
            &format!("  - to: `{destination}`"),
        );
    fixture.write("plan.md", &reviewed);
}
