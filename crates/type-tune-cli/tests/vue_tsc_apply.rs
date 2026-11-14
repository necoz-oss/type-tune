//! Applies a Vue specifier rewrite and checks it with the real `vue-tsc`.
//!
//! This test is ignored by `cargo test` so the Rust suite can run without the Vue toolchain.
//! CI and `pnpm test:vue-tsc` run it with `--ignored` after installing the pinned devDependencies.
//! A missing compiler fails the test.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const CARD: &str = r#"<template>
  <div>{{ user.id }}</div>
</template>

<script setup lang="ts">
import type { User } from "./model";

defineProps<{
  user: User;
}>();
</script>

<style scoped>
div {
  color: red;
}
</style>
"#;

#[test]
#[ignore = "requires the pinned vue and vue-tsc devDependencies; run cargo test --test vue_tsc_apply -- --ignored"]
fn apply_keeps_a_vue_specifier_rewrite_when_real_vue_tsc_accepts_it() {
    if !cfg!(unix) {
        panic!("the real vue-tsc apply test runs on the Linux CI runner");
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let vue_tsc = repo.join("node_modules/.bin/vue-tsc");
    let tsc = repo.join("node_modules/.bin/tsc");
    assert!(
        vue_tsc.is_file(),
        "vue-tsc is required at {}. Install the devDependencies with pnpm install.",
        vue_tsc.display()
    );
    assert!(tsc.is_file(), "typescript is required at {}", tsc.display());
    assert!(
        repo.join("node_modules/vue").is_dir(),
        "vue is required in node_modules"
    );

    let root = std::env::temp_dir().join(format!(
        "type-tune-vue-tsc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fixture = RealFixture { root };
    fixture.write(
        "tsconfig.json",
        r#"{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "preserve",
    "skipLibCheck": true
  },
  "include": ["src/**/*.ts", "src/**/*.vue"]
}
"#,
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
    fixture.write("src/Card.vue", CARD);
    link_repository_modules(&repo, &fixture.root);

    let initial = run_vue_tsc(&fixture.root);
    assert!(
        initial.status.success(),
        "original fixture failed vue-tsc:\n{}",
        output_text(&initial)
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
    select_user(&fixture, "src/domain/user.ts");
    let before_vue = fs::read(fixture.root.join("src/Card.vue")).unwrap();
    let before = project_snapshot(&fixture.root);

    let dry_run = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry_run.status.success(), "{}", output_text(&dry_run));
    let after = project_snapshot(&fixture.root);
    assert_eq!(after, before);
    assert_no_transaction_artifacts(&after);
    assert!(!after.directories.iter().any(|path| path == "src/domain"));

    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let updated = fs::read(fixture.root.join("src/Card.vue")).unwrap();
    let expected = replace_once(&before_vue, b"\"./model\"", b"\"./domain/user\"");
    assert_eq!(updated, expected);
    assert!(fixture.root.join("src/domain/user.ts").is_file());

    let checked = run_vue_tsc(&fixture.root);
    assert!(
        checked.status.success(),
        "rewritten fixture failed vue-tsc:\n{}",
        output_text(&checked)
    );
}

struct RealFixture {
    root: PathBuf,
}

impl Drop for RealFixture {
    fn drop(&mut self) {
        let modules = self.root.join("node_modules");
        if modules
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            let _ = fs::remove_file(modules);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl RealFixture {
    fn write(&self, path: &str, content: &str) {
        let file = self.root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.root)
            .args(args)
            .output()
            .unwrap()
    }
}

fn link_repository_modules(repo: &Path, fixture: &Path) {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(repo.join("node_modules"), fixture.join("node_modules"))
            .expect("links repository node_modules");
    }
    #[cfg(not(unix))]
    {
        let _ = (repo, fixture);
        panic!("the real vue-tsc apply test runs on the Linux CI runner");
    }
}

fn run_vue_tsc(project: &Path) -> Output {
    Command::new(project.join("node_modules/.bin/vue-tsc"))
        .current_dir(project)
        .args(["--noEmit", "--project", "tsconfig.json"])
        .output()
        .expect("starts the real vue-tsc")
}

fn select_user(fixture: &RealFixture, destination: &str) {
    let markdown = fs::read_to_string(fixture.root.join("plan.md")).unwrap();
    let plan_json: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root.join("plan.json")).unwrap()).unwrap();
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

fn replace_once(source: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let start = source
        .windows(from.len())
        .position(|window| window == from)
        .expect("original specifier");
    let mut updated = Vec::with_capacity(source.len() - from.len() + to.len());
    updated.extend_from_slice(&source[..start]);
    updated.extend_from_slice(to);
    updated.extend_from_slice(&source[start + from.len()..]);
    assert!(
        !updated[start + to.len()..]
            .windows(from.len())
            .any(|window| window == from),
        "the specifier replacement must be the only change"
    );
    updated
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectSnapshot {
    directories: BTreeSet<String>,
    files: BTreeMap<String, Vec<u8>>,
    symlinks: BTreeSet<String>,
}

fn project_snapshot(root: &Path) -> ProjectSnapshot {
    let mut snapshot = ProjectSnapshot {
        directories: BTreeSet::new(),
        files: BTreeMap::new(),
        symlinks: BTreeSet::new(),
    };
    collect_project_entries(root, root, &mut snapshot);
    snapshot
}

fn collect_project_entries(root: &Path, dir: &Path, snapshot: &mut ProjectSnapshot) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("could not read directory {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "could not read directory entry in {}: {error}",
                dir.display()
            )
        });
        let path = entry.path();
        let name = entry.file_name();
        if name == "node_modules" {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("could not inspect {}: {error}", path.display()));
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|error| {
                panic!("{} is outside {}: {error}", path.display(), root.display())
            })
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            snapshot.symlinks.insert(relative);
            continue;
        }
        if file_type.is_dir() {
            snapshot.directories.insert(relative);
            collect_project_entries(root, &path, snapshot);
            continue;
        }
        let contents = fs::read(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        snapshot.files.insert(relative, contents);
    }
}

fn assert_no_transaction_artifacts(snapshot: &ProjectSnapshot) {
    let leftovers = snapshot
        .directories
        .iter()
        .chain(snapshot.files.keys())
        .chain(snapshot.symlinks.iter())
        .filter(|path| path.contains(".type-tune-"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        leftovers.is_empty(),
        "unexpected transaction artifacts: {leftovers:?}"
    );
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
