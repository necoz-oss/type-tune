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

const STATIC_JS_COMPONENT: &str = "<template>\r\n<div v-pre class=\"card\"><!-- static text --><p title=\"Greeting\">Hello World!</p></div>\r\n</template>\r\n<script>export default { name: 'Plain' }</script>\r\n";

#[test]
#[ignore = "requires the pinned vue and vue-tsc devDependencies; run cargo test --test vue_tsc_apply -- --ignored"]
fn javascript_template_jsdoc_dependency_blocks_moves() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let fixture = RealFixture {
        root: temp.path().to_path_buf(),
    };
    fixture.write("tsconfig.json", r#"{"compilerOptions":{"strict":true,"allowJs":true,"checkJs":true,"noEmit":true,"target":"ESNext","module":"ESNext","moduleResolution":"Bundler","skipLibCheck":true},"include":["src/**/*"]}"#);
    fixture.write("src/model.ts", "export interface User { id: string }\n");
    fixture.write(
        "src/one.ts",
        "import type { User } from './model'; export type One = User;\n",
    );
    fixture.write("src/Card.vue", "<template>{{ (/** @type {import('./model').User} */ ({})).id }}</template><script>export default {}</script>");
    link_repository_modules(&repo, &fixture.root);
    let initial = run_vue_tsc(&fixture.root);
    assert!(initial.status.success(), "{}", output_text(&initial));
    fixture.write("src/model.ts", "export interface User { other: string }\n");
    let broken = run_vue_tsc(&fixture.root);
    assert!(
        !broken.status.success(),
        "vue-tsc must validate the template type import"
    );
    assert!(
        output_text(&broken).contains("Property 'id'"),
        "{}",
        output_text(&broken)
    );
    fixture.write("src/model.ts", "export interface User { id: string }\n");
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
    assert_eq!(json["analysisLimitations"][0]["path"], "src/Card.vue");
    let user = json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["symbolName"] == "User")
        .unwrap();
    assert_eq!(user["analysisLimitations"], json["analysisLimitations"]);
}

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
    let plain = "<script>export default { name: 'Plain' }</script>\n";
    fixture.write("src/Plain.vue", plain);
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
    assert_eq!(
        fs::read(fixture.root.join("src/Plain.vue")).unwrap(),
        plain.as_bytes()
    );
    assert_no_transaction_artifacts(&after);
    assert!(!after.directories.iter().any(|path| path == "src/domain"));

    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let updated = fs::read(fixture.root.join("src/Card.vue")).unwrap();
    let expected = replace_once(&before_vue, b"\"./model\"", b"\"./domain/user\"");
    assert_eq!(updated, expected);
    assert_eq!(
        fs::read(fixture.root.join("src/Plain.vue")).unwrap(),
        plain.as_bytes()
    );
    assert!(fixture.root.join("src/domain/user.ts").is_file());

    let checked = run_vue_tsc(&fixture.root);
    assert!(
        checked.status.success(),
        "rewritten fixture failed vue-tsc:\n{}",
        output_text(&checked)
    );
}

#[test]
#[ignore = "requires the pinned vue and vue-tsc devDependencies; run cargo test --test vue_tsc_apply -- --ignored"]
fn apply_moves_one_approved_vue_candidate_while_another_stays_ignored() {
    if !cfg!(unix) {
        panic!("the real vue-tsc apply test runs on the Linux CI runner");
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let root = std::env::temp_dir().join(format!(
        "type-tune-vue-partial-{}-{}",
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
    fixture.write("src/alpha/a.ts", "export interface A { id: string }\n");
    fixture.write(
        "src/alpha/use-a.ts",
        "import type { A } from './a';\nexport type UseA = A;\n",
    );
    fixture.write(
        "src/alpha/A.vue",
        "<script setup lang=\"ts\">\nimport type { A } from \"./a\";\ndefineProps<{ item: A }>();\n</script>\n<template><p>A</p></template>\n",
    );
    fixture.write("src/beta/b.ts", "export interface B { id: string }\n");
    fixture.write(
        "src/beta/use-b.ts",
        "import type { B } from './b';\nexport type UseB = B;\n",
    );
    fixture.write(
        "src/beta/B.vue",
        "<script setup lang=\"ts\">\nimport type { B } from \"./b\";\ndefineProps<{ item: B }>();\n</script>\n<template><p>B</p></template>\n",
    );
    fixture.write("src/Plain.vue", STATIC_JS_COMPONENT);
    link_repository_modules(&repo, &fixture.root);
    let scan_args = [
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "2",
    ];
    fixture.write("src/Blocked.vue", "<template>{{ (/** @type {import('./alpha/a').A} */ ({})).id }}</template><script>export default {}</script>");
    let blocked_scan = fixture.run(&scan_args);
    assert!(
        blocked_scan.status.success(),
        "{}",
        output_text(&blocked_scan)
    );
    let blocked_json: serde_json::Value = serde_json::from_slice(&blocked_scan.stdout).unwrap();
    assert_eq!(
        blocked_json["analysisLimitations"][0]["path"],
        "src/Blocked.vue"
    );
    assert_eq!(blocked_json["candidates"].as_array().unwrap().len(), 2);
    assert!(
        blocked_json["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| !candidate["analysisLimitations"]
                .as_array()
                .unwrap()
                .is_empty())
    );
    let blocked_explain = fixture.run(&[
        "scan",
        "--project",
        "tsconfig.json",
        "--format",
        "json",
        "--min-referrers",
        "2",
        "--explain",
    ]);
    assert!(
        blocked_explain.status.success(),
        "{}",
        output_text(&blocked_explain)
    );
    let blocked_explain_json: serde_json::Value =
        serde_json::from_slice(&blocked_explain.stdout).unwrap();
    assert_eq!(
        blocked_explain_json["analysisLimitations"],
        blocked_json["analysisLimitations"]
    );
    let blocked_plan = fixture.run(&[
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
    assert!(
        blocked_plan.status.success(),
        "{}",
        output_text(&blocked_plan)
    );
    select_symbol(&fixture, "A");
    let blocked_before = project_snapshot(&fixture.root);
    let blocked_apply = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(
        !blocked_apply.status.success(),
        "{}",
        output_text(&blocked_apply)
    );
    assert!(output_text(&blocked_apply).contains("src/Blocked.vue"));
    assert_eq!(project_snapshot(&fixture.root), blocked_before);
    fs::remove_file(fixture.root.join("src/Blocked.vue")).unwrap();
    let scan = fixture.run(&scan_args);
    assert!(scan.status.success(), "{}", output_text(&scan));
    let scan_json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert!(scan_json.get("analysisLimitations").is_none());
    let candidates = scan_json["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.get("analysisLimitations").is_none())
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
    assert!(explain_json.get("analysisLimitations").is_none());
    assert_eq!(explain_json["candidates"], scan_json["candidates"]);
    let custom_delimiter_check = Command::new("node").current_dir(&fixture.root).args(["-e", r#"const {parse,compileTemplate}=require('vue/compiler-sfc'); const fs=require('fs'); const source=fs.readFileSync('src/Plain.vue','utf8'); const t=parse(source).descriptor.template.content; const r=compileTemplate({source:t,id:'x',filename:'Plain.vue',compilerOptions:{delimiters:['Hello ','!']}}); const plain=compileTemplate({source:t.replace(' v-pre',''),id:'x',filename:'Plain.vue',compilerOptions:{delimiters:['Hello ','!']}}); if(r.errors.length || plain.errors.length || r.code.includes('_ctx.World') || !plain.code.includes('_ctx.World')) process.exit(1);"#]).output().expect("checks Vue custom delimiters");
    assert!(
        custom_delimiter_check.status.success(),
        "{}",
        output_text(&custom_delimiter_check)
    );
    let initial_tsc = Command::new(fixture.root.join("node_modules/.bin/tsc"))
        .current_dir(&fixture.root)
        .args([
            "--noEmit",
            "--pretty",
            "false",
            "--project",
            "tsconfig.json",
        ])
        .output()
        .expect("starts tsc");
    assert!(
        initial_tsc.status.success(),
        "original fixture failed tsc:\n{}",
        output_text(&initial_tsc)
    );
    let initial_vue = run_vue_tsc(&fixture.root);
    assert!(
        initial_vue.status.success(),
        "original fixture failed vue-tsc:\n{}",
        output_text(&initial_vue)
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
    select_symbol(&fixture, "A");
    let before = project_snapshot(&fixture.root);
    let before_a = fs::read(fixture.root.join("src/alpha/A.vue")).unwrap();
    let before_b = fs::read(fixture.root.join("src/beta/B.vue")).unwrap();
    let before_b_source = fs::read(fixture.root.join("src/beta/b.ts")).unwrap();
    let before_use_b = fs::read(fixture.root.join("src/beta/use-b.ts")).unwrap();
    let before_plain = fs::read(fixture.root.join("src/Plain.vue")).unwrap();

    let dry_run = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry_run.status.success(), "{}", output_text(&dry_run));
    assert_eq!(project_snapshot(&fixture.root), before);
    assert_no_transaction_artifacts(&project_snapshot(&fixture.root));

    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    let text = output_text(&apply);
    assert!(text.contains("Applied 1 candidate"), "{text}");
    assert!(text.contains("1 ignored"), "{text}");
    let updated_a = fs::read(fixture.root.join("src/alpha/A.vue")).unwrap();
    let expected = replace_once(&before_a, b"\"./a\"", b"\"./types/alpha\"");
    assert_eq!(updated_a, expected);
    assert_eq!(
        fs::read(fixture.root.join("src/beta/B.vue")).unwrap(),
        before_b
    );
    assert_eq!(
        fs::read(fixture.root.join("src/beta/b.ts")).unwrap(),
        before_b_source
    );
    assert_eq!(
        fs::read(fixture.root.join("src/beta/use-b.ts")).unwrap(),
        before_use_b
    );
    assert_eq!(
        fs::read(fixture.root.join("src/Plain.vue")).unwrap(),
        before_plain
    );
    let beta_files = fs::read_dir(fixture.root.join("src/beta"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        beta_files,
        ["B.vue", "b.ts", "use-b.ts"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    let checked_tsc = Command::new(fixture.root.join("node_modules/.bin/tsc"))
        .current_dir(&fixture.root)
        .args([
            "--noEmit",
            "--pretty",
            "false",
            "--project",
            "tsconfig.json",
        ])
        .output()
        .expect("starts tsc");
    assert!(
        checked_tsc.status.success(),
        "rewritten fixture failed tsc:\n{}",
        output_text(&checked_tsc)
    );
    let checked_vue = run_vue_tsc(&fixture.root);
    assert!(
        checked_vue.status.success(),
        "rewritten fixture failed vue-tsc:\n{}",
        output_text(&checked_vue)
    );
}

#[test]
#[ignore = "requires the pinned vue and vue-tsc devDependencies; run cargo test --test vue_tsc_apply -- --ignored"]
fn valid_default_import_fixtures_typecheck_and_keep_named_imports_independent() {
    if !cfg!(unix) {
        panic!("the real vue-tsc apply test runs on the Linux CI runner");
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let root = std::env::temp_dir().join(format!(
        "type-tune-vue-default-{}-{}",
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
    fixture.write(
        "src/models.ts",
        "export interface User { id: string }\nexport default class RuntimeModel {}\n",
    );
    fixture.write(
        "src/forward.ts",
        "export type { User as default } from \"./models\";\n",
    );
    fixture.write(
        "src/use.ts",
        "import type { User } from './models';\nexport type App = User;\n",
    );
    fixture.write(
        "src/Separate.vue",
        "<script setup lang=\"ts\">\nimport RuntimeModel from \"./models\";\nimport type { User } from \"./models\";\nconst model = new RuntimeModel();\ndefineProps<{ user: User }>();\n</script>\n",
    );
    fixture.write(
        "src/Mixed.vue",
        "<script setup lang=\"ts\">\nimport RuntimeModel, { type User } from \"./models\";\nconst model = new RuntimeModel();\ndefineProps<{ user: User }>();\n</script>\n",
    );
    fixture.write(
        "src/Forward.vue",
        "<script setup lang=\"ts\">\nimport type User from \"./forward\";\ndefineProps<{ user: User }>();\n</script>\n",
    );
    link_repository_modules(&repo, &fixture.root);
    let checked = run_vue_tsc(&fixture.root);
    assert!(
        checked.status.success(),
        "default-import fixture failed vue-tsc:\n{}",
        output_text(&checked)
    );
    let scan = fixture.run(&["scan", "--project", "tsconfig.json", "--format", "json"]);
    assert!(scan.status.success(), "{}", output_text(&scan));
    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let candidates = report["candidates"].as_array().unwrap();
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["symbolName"] != "RuntimeModel")
    );
    let user = candidates
        .iter()
        .find(|candidate| candidate["symbolName"] == "User")
        .expect("user");
    assert!(
        user["vueScriptStrategies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|strategy| strategy["referrerPath"] == "src/Separate.vue"),
        "{user}"
    );
    assert!(
        user["vueRewriteBlockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["path"] == "src/Mixed.vue"),
        "{user}"
    );
    assert!(
        user["vueRewriteBlockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["path"] == "src/Forward.vue"),
        "{user}"
    );
}

fn artifact_fixture(options: &str, vue: bool) -> RealFixture {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "type-tune-artifact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fixture = RealFixture { root };
    fixture.write(
        "tsconfig.json",
        &format!(r#"{{"compilerOptions":{{{options}}},"include":["src/**/*.ts","src/**/*.vue"]}}"#),
    );
    fixture.write("src/model.ts", "export interface User { id: string }\n");
    fixture.write(
        "src/one.ts",
        "import type { User } from './model'; export type One = User;\n",
    );
    fixture.write(
        "src/two.ts",
        "import type { User } from './model'; export type Two = User;\n",
    );
    if vue {
        fixture.write("src/Card.vue", CARD);
    }
    link_repository_modules(&repo, &fixture.root);
    fixture
}

fn approve_artifact_fixture(fixture: &RealFixture) {
    let plan = fixture.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "plan.json",
        "--markdown",
        "plan.md",
    ]);
    assert!(plan.status.success(), "{}", output_text(&plan));
    select_symbol(fixture, "User");
}

fn validate_artifact_fixture(fixture: &RealFixture, vue: bool) -> Output {
    Command::new(fixture.root.join(if vue {
        "node_modules/.bin/vue-tsc"
    } else {
        "node_modules/.bin/tsc"
    }))
    .current_dir(&fixture.root)
    .args(["--noEmit", "--project", "tsconfig.json"])
    .output()
    .unwrap()
}

#[test]
#[ignore = "requires pinned TypeScript; run cargo test --test vue_tsc_apply -- --ignored"]
fn compiler_artifacts_are_isolated_for_reviewed_apply() {
    for options in [
        r#""composite":true,"outDir":"dist""#,
        r#""incremental":true,"rootDir":"src","tsBuildInfoFile":"cache/nested/state.tsbuildinfo""#,
    ] {
        let fixture = artifact_fixture(options, false);
        approve_artifact_fixture(&fixture);
        let before = project_snapshot(&fixture.root);
        for _ in 0..2 {
            let dry = fixture.run(&[
                "apply",
                "--plan",
                "plan.json",
                "--markdown",
                "plan.md",
                "--dry-run",
            ]);
            assert!(dry.status.success(), "{}", output_text(&dry));
            assert!(
                project_snapshot(&fixture.root) == before,
                "dry-run changed project snapshot"
            );
        }
        let native = validate_artifact_fixture(&fixture, false);
        assert!(native.status.success(), "{}", output_text(&native));
        let artifact = if options.contains("composite") {
            "dist/tsconfig.tsbuildinfo"
        } else {
            "cache/nested/state.tsbuildinfo"
        };
        let artifact_bytes =
            fs::read(fixture.root.join(artifact)).expect("native compiler writes build info");
        let with_artifact = project_snapshot(&fixture.root);
        let dry = fixture.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(dry.status.success(), "{}", output_text(&dry));
        assert!(
            project_snapshot(&fixture.root) == with_artifact,
            "dry-run changed existing artifact or another file"
        );
        let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
        assert!(apply.status.success(), "{}", output_text(&apply));
        assert_eq!(
            fs::read(fixture.root.join(artifact)).unwrap(),
            artifact_bytes
        );
        assert!(fixture.root.join("src/types/src.ts").is_file());
        assert!(validate_artifact_fixture(&fixture, false).status.success());
    }
    let fixture = artifact_fixture(r#""composite":true,"outDir":"dist""#, false);
    approve_artifact_fixture(&fixture);
    let apply = fixture.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
    assert!(apply.status.success(), "{}", output_text(&apply));
    assert!(fixture.root.join("src/types/src.ts").is_file());
    assert!(
        !fixture.root.join("dist").exists(),
        "successful apply left build info"
    );
}

#[test]
#[ignore = "requires pinned TypeScript; run cargo test --test vue_tsc_apply -- --ignored"]
fn compiler_failure_restores_project_without_build_info() {
    let fixture = artifact_fixture(
        r#""incremental":true,"tsBuildInfoFile":"cache/nested/state.tsbuildinfo""#,
        false,
    );
    fixture.write("src/broken.ts", "export const broken: string = 42;\n");
    approve_artifact_fixture(&fixture);
    let before = project_snapshot(&fixture.root);
    for dry in [true, false] {
        let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
        if dry {
            args.push("--dry-run");
        }
        let result = fixture.run(&args);
        assert!(!result.status.success(), "compiler error must fail");
        assert!(
            output_text(&result).contains("TS2322"),
            "{}",
            output_text(&result)
        );
        assert!(
            project_snapshot(&fixture.root) == before,
            "failed validation changed project snapshot"
        );
    }
}

#[test]
#[ignore = "requires pinned Vue and vue-tsc; run cargo test --test vue_tsc_apply -- --ignored"]
fn vue_compiler_validation_isolates_build_info_on_success_and_failure() {
    let options = r#""composite":true,"outDir":"cache/nested","target":"ESNext","module":"ESNext","moduleResolution":"Bundler","jsx":"preserve","skipLibCheck":true"#;
    let fixture = artifact_fixture(options, true);
    approve_artifact_fixture(&fixture);
    let before = project_snapshot(&fixture.root);
    let dry = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry.status.success(), "{}", output_text(&dry));
    assert!(
        project_snapshot(&fixture.root) == before,
        "Vue dry-run changed snapshot"
    );
    let native = validate_artifact_fixture(&fixture, true);
    assert!(native.status.success(), "{}", output_text(&native));
    assert!(
        fixture
            .root
            .join("cache/nested/tsconfig.tsbuildinfo")
            .is_file(),
        "real vue-tsc must write build info"
    );
    let with_artifact = project_snapshot(&fixture.root);
    let dry = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry.status.success(), "{}", output_text(&dry));
    assert!(
        project_snapshot(&fixture.root) == with_artifact,
        "Vue dry-run changed existing build info"
    );

    let broken = artifact_fixture(options, true);
    broken.write("src/Plain.vue", STATIC_JS_COMPONENT);
    broken.write("src/Card.vue", &CARD.replace("user.id", "user.missing"));
    let tsc = validate_artifact_fixture(&broken, false);
    assert!(tsc.status.success(), "{}", output_text(&tsc));
    approve_artifact_fixture(&broken);
    let before = project_snapshot(&broken.root);
    for dry in [true, false] {
        let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
        if dry {
            args.push("--dry-run");
        }
        let result = broken.run(&args);
        assert!(!result.status.success());
        assert!(
            output_text(&result).contains("vue-tsc validation failed"),
            "{}",
            output_text(&result)
        );
        assert!(
            project_snapshot(&broken.root) == before,
            "failed Vue validation changed snapshot"
        );
    }
}

#[test]
#[ignore = "requires pinned TypeScript; run cargo test --test vue_tsc_apply -- --ignored"]
fn redirects_inherited_and_external_artifact_paths() {
    let external = artifact_fixture(r#""noEmit":true"#, false);
    external.write("sentinel.tsbuildinfo", "do not overwrite");
    let sentinel = external.root.join("sentinel.tsbuildinfo");
    for artifact in [
        sentinel.clone(),
        external.root.join("missing/nested/output.tsbuildinfo"),
    ] {
        let value = serde_json::to_string(&artifact.to_string_lossy().to_string()).unwrap();
        let fixture = artifact_fixture(
            &format!(r#""incremental":true,"tsBuildInfoFile":{value}"#),
            false,
        );
        approve_artifact_fixture(&fixture);
        let before = project_snapshot(&fixture.root);
        let outside = project_snapshot(&external.root);
        let result = fixture.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(result.status.success(), "{}", output_text(&result));
        assert!(project_snapshot(&fixture.root) == before);
        assert!(
            project_snapshot(&external.root) == outside,
            "external file or directory changed"
        );
    }
    let fixture = artifact_fixture(r#""noEmit":true"#, false);
    let outside_name = external.root.file_name().unwrap().to_string_lossy();
    fixture.write("tsconfig.json", &format!(r#"{{"compilerOptions":{{"incremental":true,"tsBuildInfoFile":"../{outside_name}/sentinel.tsbuildinfo"}},"include":["src"]}}"#));
    approve_artifact_fixture(&fixture);
    let before = project_snapshot(&fixture.root);
    let outside = project_snapshot(&external.root);
    let result = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(result.status.success(), "{}", output_text(&result));
    assert!(
        project_snapshot(&fixture.root) == before && project_snapshot(&external.root) == outside
    );

    let fixture = artifact_fixture(r#""noEmit":true"#, false);
    fixture.write(
        "base.json",
        r#"{"compilerOptions":{"incremental":true,"tsBuildInfoFile":"cache/state.tsbuildinfo"}}"#,
    );
    fixture.write(
        "tsconfig.json",
        r#"{"extends":"./base.json","include":["src"]}"#,
    );
    approve_artifact_fixture(&fixture);
    let before = project_snapshot(&fixture.root);
    let result = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(result.status.success(), "{}", output_text(&result));
    assert!(
        project_snapshot(&fixture.root) == before,
        "inherited artifact leaked"
    );
    let native = validate_artifact_fixture(&fixture, false);
    assert!(native.status.success(), "{}", output_text(&native));
    assert!(fixture.root.join("cache/state.tsbuildinfo").is_file());
}

#[cfg(unix)]
#[test]
#[ignore = "requires pinned TypeScript; run cargo test --test vue_tsc_apply -- --ignored"]
fn redirects_build_info_through_symlink_without_touching_target() {
    let outside = artifact_fixture(r#""noEmit":true"#, false);
    outside.write("state.tsbuildinfo", "preserve this user file");
    let fixture = artifact_fixture(
        r#""incremental":true,"tsBuildInfoFile":"cache/state.tsbuildinfo""#,
        false,
    );
    std::os::unix::fs::symlink(&outside.root, fixture.root.join("cache")).unwrap();
    approve_artifact_fixture(&fixture);
    let before = project_snapshot(&fixture.root);
    let outside_before = project_snapshot(&outside.root);
    let result = fixture.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(result.status.success(), "{}", output_text(&result));
    assert!(project_snapshot(&fixture.root) == before);
    assert!(project_snapshot(&outside.root) == outside_before);
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
        let temporary = self.root.join("temporary");
        fs::create_dir_all(&temporary).unwrap();
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.root)
            .env("TMPDIR", temporary)
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

fn select_symbol(fixture: &RealFixture, symbol: &str) {
    let markdown = fs::read_to_string(fixture.root.join("plan.md")).unwrap();
    let plan_json: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root.join("plan.json")).unwrap()).unwrap();
    let entry = plan_json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["candidate"]["symbolName"] == symbol)
        .unwrap_or_else(|| panic!("missing {symbol}"));
    let id = entry["candidate"]["id"].as_str().unwrap();
    let reviewed = markdown.replace(
        &format!("- [ignore] {id} `{symbol}`"),
        &format!("- [apply] {id} `{symbol}`"),
    );
    assert_ne!(reviewed, markdown, "could not approve {symbol}");
    fixture.write("plan.md", &reviewed);
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
    symlinks: BTreeMap<String, PathBuf>,
}

fn project_snapshot(root: &Path) -> ProjectSnapshot {
    let mut snapshot = ProjectSnapshot {
        directories: BTreeSet::new(),
        files: BTreeMap::new(),
        symlinks: BTreeMap::new(),
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
        if dir == root && name == "node_modules" {
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
            snapshot
                .symlinks
                .insert(relative, fs::read_link(&path).unwrap());
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
        .chain(snapshot.symlinks.keys())
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
