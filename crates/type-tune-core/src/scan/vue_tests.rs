use std::{fs, path::Path};

use super::{
    filter_min_referrers, scan_project_files, scan_tsconfig, scan_tsconfig_project,
    scan_tsconfig_with_generated, scan_tsconfig_with_generated_and_diagnostics,
};
use crate::{
    model::READONLY_REFERRER_VUE_SFC, rewrite::plan_extraction_rewrite,
    source::read_typescript_source_text,
};

fn write_project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("creates project");
    for (path, content) in files {
        let file = root.path().join(path);
        fs::create_dir_all(file.parent().expect("parent")).expect("creates directories");
        fs::write(file, content).expect("writes file");
    }
    root
}

fn user_type() -> &'static str {
    "export interface User { id: string }\nexport interface Other { id: string }\n"
}

fn referrers_for(root: &Path, symbol: &str) -> (Vec<String>, Vec<String>, usize) {
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans");
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == symbol)
        .unwrap_or_else(|| panic!("missing {symbol} in {candidates:?}"));
    (
        candidate.referrer_paths.clone(),
        candidate
            .readonly_referrers
            .iter()
            .map(|referrer| referrer.path.clone())
            .collect(),
        candidate.referrer_count,
    )
}

#[test]
fn counts_script_setup_define_props_and_define_emits_once_per_file() {
    let root = write_project(&[
        ("tsconfig.json", r#"{"include":["src"]}"#),
        ("src/types.ts", user_type()),
        (
            "src/Card.vue",
            r#"
<template><p>{{ user.name }}</p></template>
<script setup lang="ts">
import type { User } from "./types";
defineProps<{ user: User }>();
defineEmits<{ save: [user: User] }>();
</script>
<style>.user { color: User; }</style>
"#,
        ),
    ]);
    let (writable, readonly, count) = referrers_for(root.path(), "User");
    assert!(writable.is_empty());
    assert_eq!(readonly, ["src/Card.vue"]);
    assert_eq!(count, 1);
    let candidates = scan_tsconfig(&root.path().join("tsconfig.json")).unwrap();
    let user = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert!(user.referrer_strategies.is_empty());
    assert!(
        user.readonly_referrers
            .iter()
            .all(|referrer| referrer.reason == READONLY_REFERRER_VUE_SFC)
    );
}

#[test]
fn counts_standard_script_blocks_aliases_and_distinct_files_deterministically() {
    let root = write_project(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}},"include":["src"]}"#,
        ),
        ("src/types.ts", user_type()),
        (
            "src/app.ts",
            "import type { User } from './types';\nexport type App = User;\n",
        ),
        (
            "src/b/List.vue",
            "<script lang=\"ts\">\nimport type { User as Person } from \"@/types\";\nexport default { props: {} as { user: Person } };\n</script>\n",
        ),
        (
            "src/a/Card.vue",
            "<script setup lang='tsx'>\nimport type { User } from \"../types\";\nconst View = (props: { user: User }) => <span>{props.user.id}</span>;\n</script>\n",
        ),
        (
            "src/Unused.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\nconst label = \"unused\";\n</script>\n",
        ),
    ]);
    let first = referrers_for(root.path(), "User");
    let second = referrers_for(root.path(), "User");
    assert_eq!(first, second);
    assert_eq!(first.0, ["src/app.ts"]);
    assert_eq!(first.1, ["src/a/Card.vue", "src/b/List.vue"]);
    assert_eq!(first.2, 3);
    let (other_writable, other_readonly, other_count) = referrers_for(root.path(), "Other");
    assert!(other_writable.is_empty());
    assert!(other_readonly.is_empty());
    assert_eq!(other_count, 0);
}

#[test]
fn respects_include_exclude_and_generated_vue_files() {
    let root = write_project(&[
        (
            "tsconfig.json",
            r#"{"include":["src/**/*.ts","src/**/*.vue"],"exclude":["src/legacy"]}"#,
        ),
        ("src/types.ts", user_type()),
        (
            "src/Card.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
        (
            "src/legacy/Old.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"../types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
        (
            "src/generated/Hidden.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"../types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
        (
            "src/Widget.generated.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
        (
            "node_modules/pkg/Dep.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"../../src/types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
    ]);
    let (_, readonly, count) = referrers_for(root.path(), "User");
    assert_eq!(readonly, ["src/Card.vue"]);
    assert_eq!(count, 1);

    let generated = scan_tsconfig_with_generated(&root.path().join("tsconfig.json"), true).unwrap();
    let user = generated
        .iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert_eq!(
        user.readonly_referrers
            .iter()
            .map(|referrer| referrer.path.as_str())
            .collect::<Vec<_>>(),
        [
            "src/Card.vue",
            "src/Widget.generated.vue",
            "src/generated/Hidden.vue"
        ]
    );
}

#[test]
fn explicit_typescript_includes_do_not_pull_in_vue_files() {
    let root = write_project(&[
        ("tsconfig.json", r#"{"include":["src/**/*.ts"]}"#),
        ("src/types.ts", user_type()),
        (
            "src/app.ts",
            "import type { User } from './types';\nexport type App = User;\n",
        ),
        (
            "src/Card.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
    ]);
    let (writable, readonly, count) = referrers_for(root.path(), "User");
    assert_eq!(writable, ["src/app.ts"]);
    assert!(readonly.is_empty());
    assert_eq!(count, 1);
}

#[test]
fn reports_malformed_vue_files_without_counting_partial_usage() {
    let root = write_project(&[
        ("tsconfig.json", r#"{"include":["src"]}"#),
        ("src/types.ts", user_type()),
        (
            "src/app.ts",
            "import type { User } from './types';\nexport type App = User;\n",
        ),
        (
            "src/Broken.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n",
        ),
    ]);
    let scan = scan_tsconfig_project(&root.path().join("tsconfig.json"), false).unwrap();
    let user = scan
        .candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert_eq!(user.referrer_paths, ["src/app.ts"]);
    assert!(user.readonly_referrers.is_empty());
    assert_eq!(user.referrer_count, 1);
    assert!(
        scan.analysis_limitations
            .iter()
            .any(|limitation| limitation.path == "src/Broken.vue"
                && limitation.message.contains("unclosed"))
    );
    let (_, diagnostics) =
        scan_tsconfig_with_generated_and_diagnostics(&root.path().join("tsconfig.json"), false)
            .unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reason == "unresolvedVueSfc"
                && diagnostic.source_path == "src/Broken.vue")
    );
}

#[test]
fn rejects_vue_referrer_rewrites_before_producing_edits() {
    let root = write_project(&[
        ("tsconfig.json", r#"{"include":["src"]}"#),
        ("src/types.ts", user_type()),
        (
            "src/app.ts",
            "import type { User } from './types';\nexport type App = User;\n",
        ),
        (
            "src/page.ts",
            "import type { User } from './types';\nexport type Page = User;\n",
        ),
        (
            "src/Card.vue",
            "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n</script>\n",
        ),
    ]);
    let before = fs::read(root.path().join("src/types.ts")).unwrap();
    let candidates = scan_tsconfig(&root.path().join("tsconfig.json")).unwrap();
    let user = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert_eq!(user.referrer_count, 3);
    let eligible = filter_min_referrers(vec![user], 2);
    assert_eq!(eligible.len(), 1);
    let mut sources = std::collections::BTreeMap::new();
    for path in ["src/types.ts", "src/app.ts", "src/page.ts"] {
        sources.insert(
            path.to_owned(),
            read_typescript_source_text(&root.path().join(path))
                .unwrap()
                .unwrap(),
        );
    }
    let error = plan_extraction_rewrite(&eligible, &sources).expect_err("rejects vue referrer");
    let message = error.to_string();
    assert!(
        message.contains("Vue rewriting is not supported yet"),
        "{message}"
    );
    assert!(message.contains("src/Card.vue"), "{message}");
    assert!(
        message.contains("before any files were changed"),
        "{message}"
    );
    assert_eq!(fs::read(root.path().join("src/types.ts")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(root.path().join("src/Card.vue")).unwrap(),
        "<script setup lang=\"ts\">\nimport type { User } from \"./types\";\ndefineProps<{ user: User }>();\n</script>\n"
    );
}

#[test]
fn rejects_unresolved_vue_files_and_forged_vue_strategies() {
    let root = write_project(&[
        ("tsconfig.json", r#"{"include":["src"]}"#),
        ("src/types.ts", "export interface User { id: string }\n"),
        (
            "src/app.ts",
            "import type { User } from './types';\nexport type App = User;\n",
        ),
        (
            "src/page.ts",
            "import type { User } from './types';\nexport type Page = User;\n",
        ),
        ("src/Plain.vue", "<script>\nexport default {}\n</script>\n"),
    ]);
    let candidates = scan_tsconfig(&root.path().join("tsconfig.json")).unwrap();
    let user = candidates
        .into_iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert!(
        user.analysis_limitations
            .iter()
            .any(|limitation| limitation.path == "src/Plain.vue")
    );
    let error = plan_extraction_rewrite(
        std::slice::from_ref(&user),
        &std::collections::BTreeMap::new(),
    )
    .expect_err("rejects unresolved vue");
    assert!(
        error.to_string().contains("lang=\\\"ts\\\"") || error.to_string().contains("lang=\"ts\""),
        "{}",
        error
    );

    let mut forged = user.clone();
    forged.analysis_limitations.clear();
    forged.readonly_referrers.clear();
    forged.referrer_strategies[0].referrer_path = "src/Card.vue".into();
    let error = plan_extraction_rewrite(
        std::slice::from_ref(&forged),
        &std::collections::BTreeMap::new(),
    )
    .expect_err("rejects forged vue strategy");
    assert!(error.to_string().contains("src/Card.vue"), "{error}");
}

#[test]
fn leaves_typescript_and_tsx_referrers_unchanged() {
    let candidates = scan_project_files(vec![
        ("src/types.ts".into(), "export interface User { id: string }\n".into()),
        ("src/app.ts".into(), "import type { User } from './types';\nexport type App = User;\n".into()),
        ("src/view.tsx".into(), "import type { User } from './types';\nexport const View = (props: { user: User }) => props.user;\n".into()),
    ])
    .unwrap();
    let user = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "User")
        .unwrap();
    assert_eq!(user.referrer_paths, ["src/app.ts", "src/view.tsx"]);
    assert_eq!(user.referrer_count, 2);
    assert!(user.readonly_referrers.is_empty());
    assert!(user.analysis_limitations.is_empty());
    assert_eq!(user.referrer_strategies.len(), 2);
}
