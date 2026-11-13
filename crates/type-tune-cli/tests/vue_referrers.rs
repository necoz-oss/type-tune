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
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
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
    write_compiler_stub(&fixture.0, "vue-tsc", "#!/bin/sh\nexit 1\n");
    let before = fixture.snapshot();
    let failed = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(!failed.status.success(), "{}", output_text(&failed));
    assert!(
        output_text(&failed).contains("vue-tsc validation failed"),
        "{}",
        output_text(&failed)
    );
    assert_eq!(fixture.snapshot(), before);
    assert!(!fixture.0.join("src/domain").exists());

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
