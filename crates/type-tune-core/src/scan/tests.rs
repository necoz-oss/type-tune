use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    ScanFilterOptions, apply_placement_with_aliases, apply_referrer_count_exclusions,
    filter_candidates_with_diagnostics, filter_existing_types, filter_ignored_candidates,
    filter_min_referrers, filter_noop_destinations, scan_declarations, scan_file,
    scan_project_files, scan_project_files_with_aliases, scan_tsconfig,
    scan_tsconfig_with_generated_and_diagnostics, unsupported_dependency_diagnostics,
};
use crate::model::ImportStrategy;
use crate::parser::parse_source;

#[test]
fn yields_deterministically_ordered_candidates() {
    let candidates = scan_declarations([
        (
            String::from("src/z.ts"),
            String::from("export type Z = {};"),
        ),
        (
            String::from("src/a.ts"),
            String::from("export interface A {}"),
        ),
    ])
    .expect("scans");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["A", "Z"]
    );
}

#[test]
fn excludes_directories_matched_by_wildcard_patterns() {
    let root = tempfile::tempdir().expect("creates project");
    fs::create_dir_all(root.path().join("src/dist")).expect("creates directories");
    fs::write(root.path().join("src/app.ts"), "export type App = string;").expect("writes source");
    fs::write(
        root.path().join("src/dist/ignored.ts"),
        "export type Ignored = string;",
    )
    .expect("writes excluded source");
    fs::write(
        root.path().join("tsconfig.json"),
        r#"{"include":["**/*.ts"],"exclude":["**/dist"]}"#,
    )
    .expect("writes config");
    let candidates = scan_tsconfig(&root.path().join("tsconfig.json")).expect("scans");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App"]
    );
}

#[test]
fn non_segment_globstars_do_not_include_nested_files() {
    let root = tempfile::tempdir().expect("creates project");
    fs::create_dir_all(root.path().join("src/sub")).expect("creates directories");
    fs::write(root.path().join("src/one.ts"), "export type One = string;").expect("writes source");
    fs::write(
        root.path().join("src/sub/two.ts"),
        "export type Two = string;",
    )
    .expect("writes nested source");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"include":["src/**.ts"]}"#).expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["One"]
    );
}

#[test]
fn excludes_nested_package_directories_by_default_but_honors_explicit_selection() {
    let root = tempfile::tempdir().expect("creates project");
    for directory in ["src/bower_components", "src/jspm_packages"] {
        fs::create_dir_all(root.path().join(directory)).expect("creates directory");
        fs::write(
            root.path().join(directory).join("dependency.ts"),
            "export type Dependency = string;",
        )
        .expect("writes dependency");
    }
    fs::write(root.path().join("src/app.ts"), "export type App = string;").expect("writes source");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, "{}").expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans default project");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App"]
    );

    fs::write(&config, r#"{"include":["src/**/*.ts"]}"#).expect("writes wildcard include");
    let candidates = scan_tsconfig(&config).expect("scans wildcard include");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App"]
    );

    fs::write(&config, r#"{"include":["src/bower_components/**/*.ts"]}"#)
        .expect("writes explicit include");
    let candidates = scan_tsconfig(&config).expect("scans explicit include");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Dependency"]
    );

    fs::write(
        &config,
        r#"{"include":["src/**/*.ts","src/bower_components/**/*.ts"]}"#,
    )
    .expect("writes combined includes");
    let candidates = scan_tsconfig(&config).expect("scans combined includes");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App", "Dependency"]
    );

    fs::write(
        &config,
        r#"{"files":["src/bower_components/dependency.ts"]}"#,
    )
    .expect("writes explicit files");
    let candidates = scan_tsconfig(&config).expect("scans explicit file");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Dependency"]
    );
}

#[test]
fn honors_explicit_root_package_includes_and_excludes() {
    let root = tempfile::tempdir().expect("creates project");
    fs::create_dir_all(root.path().join("bower_components/pkg")).expect("creates directory");
    fs::write(
        root.path().join("bower_components/pkg/model.ts"),
        "export type Model = string;",
    )
    .expect("writes source");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"include":["bower_components/**/*.ts"]}"#).expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans explicit include");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol_name, "Model");

    fs::write(
        &config,
        r#"{"include":["bower_components/**/*.ts"],"exclude":["bower_components"]}"#,
    )
    .expect("writes excluded config");
    assert!(
        scan_tsconfig(&config)
            .expect("scans explicit exclude")
            .is_empty()
    );
}

#[test]
fn scans_explicit_package_files_and_their_local_imports() {
    let root = tempfile::tempdir().expect("creates project");
    let package = root.path().join("node_modules/@scope/pkg");
    fs::create_dir_all(&package).expect("creates package");
    fs::write(
        package.join("model.ts"),
        "import type { Helper } from './helper'; export type Model = Helper;",
    )
    .expect("writes model");
    fs::write(package.join("helper.ts"), "export type Helper = string;").expect("writes helper");
    fs::write(package.join("unused.ts"), "export type Unused = string;").expect("writes unused");
    fs::write(
        package.join("schema.generated.ts"),
        "export type Generated = string;",
    )
    .expect("writes generated");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"files":["node_modules/@scope/pkg/model.ts","node_modules/@scope/pkg/schema.generated.ts"],"include":["**/*.ts"],"exclude":[]}"#).expect("writes config");
    let names = |candidates: Vec<crate::model::Candidate>| {
        candidates
            .into_iter()
            .map(|candidate| candidate.symbol_name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(scan_tsconfig(&config).expect("scans")),
        ["Helper", "Model"]
    );
    assert_eq!(
        names(super::scan_tsconfig_with_generated(&config, true).expect("scans generated")),
        ["Helper", "Model", "Generated"]
    );
}

#[test]
fn package_glob_exclusion_does_not_hide_a_matching_include() {
    let root = tempfile::tempdir().expect("creates project");
    let package = root.path().join("node_modules/pkg");
    fs::create_dir_all(&package).expect("creates package");
    fs::write(package.join("foo.ts"), "export interface Included {}")
        .expect("writes included source");
    fs::write(package.join("foo1.ts"), "export interface Excluded {}")
        .expect("writes excluded source");
    let config = root.path().join("tsconfig.json");
    fs::write(
        &config,
        r#"{"include":["node_modules/pkg/foo*.ts"],"exclude":["node_modules/pkg/foo?*.ts"]}"#,
    )
    .expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans project");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Included"]
    );
    fs::write(
        &config,
        r#"{"files":["node_modules/pkg/foo1.ts"],"exclude":["node_modules"]}"#,
    )
    .expect("writes explicit file config");
    let candidates = scan_tsconfig(&config).expect("scans explicit file");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Excluded"]
    );
}

#[test]
fn scans_explicit_package_includes_and_respects_excludes() {
    let root = tempfile::tempdir().expect("creates project");
    for (directory, name) in [
        ("node_modules/pkg", "Root"),
        ("node_modules/other", "Other"),
        ("src/node_modules/pkg", "Nested"),
        ("src/.hidden/node_modules/pkg", "Hidden"),
    ] {
        fs::create_dir_all(root.path().join(directory)).expect("creates package");
        fs::write(
            root.path().join(directory).join("model.ts"),
            format!("export type {name} = string;"),
        )
        .expect("writes source");
    }
    fs::write(
        root.path().join("node_modules/pkg/schema.generated.ts"),
        "export type Generated = string;",
    )
    .expect("writes generated source");
    let config = root.path().join("tsconfig.json");
    let names = |candidates: Vec<crate::model::Candidate>| {
        candidates
            .into_iter()
            .map(|candidate| candidate.symbol_name)
            .collect::<Vec<_>>()
    };
    fs::write(&config, r#"{"include":["node_modules/pkg/**/*.ts"]}"#).expect("writes config");
    assert_eq!(
        names(scan_tsconfig(&config).expect("scans package")),
        ["Root"]
    );
    assert_eq!(
        names(super::scan_tsconfig_with_generated(&config, true).expect("scans generated")),
        ["Root", "Generated"]
    );
    fs::write(&config, r#"{"include":["**/node_modules/pkg/**/*.ts"]}"#)
        .expect("writes wildcard config");
    assert_eq!(
        names(scan_tsconfig(&config).expect("scans packages")),
        ["Root", "Nested"]
    );
    fs::write(
        &config,
        r#"{"include":["**/node_modules/pkg/**/*.ts"],"exclude":["node_modules"]}"#,
    )
    .expect("writes exclusion");
    assert_eq!(
        names(scan_tsconfig(&config).expect("scans exclusion")),
        ["Nested"]
    );

    let deep = root
        .path()
        .join("node_modules/outer/node_modules/@scope/inner");
    fs::create_dir_all(&deep).expect("creates nested package");
    fs::write(deep.join("model.ts"), "export type Deep = string;").expect("writes nested source");
    fs::write(
        &config,
        r#"{"include":["node_modules/outer/node_modules/@scope/inner/**/*.ts"]}"#,
    )
    .expect("writes nested package include");
    assert_eq!(
        names(scan_tsconfig(&config).expect("scans nested package")),
        ["Deep"]
    );
}

#[test]
fn requires_each_nested_package_directory_to_be_explicit() {
    let root = tempfile::tempdir().expect("creates project");
    let package = root.path().join("src/bower_components");
    fs::create_dir_all(package.join("bower_components")).expect("creates directories");
    fs::write(package.join("direct.ts"), "export type Direct = string;").expect("writes source");
    fs::write(
        package.join("bower_components/nested.ts"),
        "export type Nested = string;",
    )
    .expect("writes nested source");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"include":["**/bower_components/**/*.ts"]}"#).expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans one explicit directory");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Direct"]
    );
    fs::write(
        &config,
        r#"{"include":["**/bower_components/**/bower_components/**/*.ts"]}"#,
    )
    .expect("writes nested config");
    let candidates = scan_tsconfig(&config).expect("scans two explicit directories");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Nested"]
    );
}

#[test]
fn wildcard_discovery_skips_hidden_paths_unless_explicitly_included() {
    let root = tempfile::tempdir().expect("creates project");
    fs::create_dir_all(root.path().join("src/.hidden")).expect("creates directories");
    for (path, source) in [
        ("src/app.ts", "export type App = string;"),
        ("src/.secret.ts", "export type Secret = string;"),
        ("src/.hidden/model.ts", "export type Model = string;"),
    ] {
        fs::write(root.path().join(path), source).expect("writes source");
    }
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"include":["**/*.ts"]}"#).expect("writes wildcard config");
    let candidates = scan_tsconfig(&config).expect("scans wildcard");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App"]
    );
    fs::write(
        &config,
        r#"{"include":["src/.hidden/**/*.ts","src/.*.ts"]}"#,
    )
    .expect("writes explicit config");
    let candidates = scan_tsconfig(&config).expect("scans explicit paths");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Model", "Secret"]
    );
    fs::write(&config, r#"{"files":["src/.hidden/model.ts"]}"#).expect("writes files config");
    let candidates = scan_tsconfig(&config).expect("scans explicit file");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol_name, "Model");
}

#[test]
fn scans_a_typescript_file_from_disk() {
    let path = std::env::temp_dir().join(format!(
        "type-tune-{}.ts",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::write(&path, "export type Shared = {};").expect("writes fixture");
    let candidates = scan_file(&path, "src/shared.ts".into()).expect("scans");
    fs::remove_file(path).expect("removes fixture");
    assert_eq!(candidates[0].symbol_name, "Shared");
}

#[test]
fn excludes_compiler_out_dir_by_default() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-outdir-scan-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::create_dir_all(root.join("dist/src")).expect("creates output");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"outDir":"dist"}}"#,
    )
    .expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "export type Shared = { id: string };",
    )
    .expect("writes source");
    fs::write(
        root.join("src/app.ts"),
        "import type { Shared } from './shared'; export type App = Shared;",
    )
    .expect("writes app");
    fs::write(
        root.join("dist/src/shared.ts"),
        "export type Shared = { stale: true };",
    )
    .expect("writes emitted source");
    fs::write(
        root.join("dist/src/app.ts"),
        "import type { Shared } from './shared'; export type App = Shared;",
    )
    .expect("writes emitted app");
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans");
    assert!(
        candidates
            .iter()
            .all(|candidate| !candidate.source_path.starts_with("dist/"))
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.symbol_name == "Shared")
    );
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn excludes_inherited_declaration_dir_unless_exclude_is_explicit() {
    let root = tempfile::tempdir().expect("creates project");
    fs::create_dir_all(root.path().join("types-out")).expect("creates output directory");
    fs::write(root.path().join("app.ts"), "export type App = string;").expect("writes source");
    fs::write(
        root.path().join("types-out/model.ts"),
        "export type Model = string;",
    )
    .expect("writes output");
    fs::write(
        root.path().join("base.json"),
        r#"{"compilerOptions":{"declarationDir":"types-out"}}"#,
    )
    .expect("writes base config");
    let config = root.path().join("tsconfig.json");
    fs::write(&config, r#"{"extends":"./base.json"}"#).expect("writes config");
    let candidates = scan_tsconfig(&config).expect("scans default exclusions");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App"]
    );
    fs::write(&config, r#"{"extends":"./base.json","exclude":[]}"#)
        .expect("writes explicit exclusions");
    let candidates = scan_tsconfig(&config).expect("scans explicit exclusions");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["App", "Model"]
    );
}

#[test]
fn includes_imported_files_omitted_by_exclude() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-exclude-import-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"exclude":["src/hidden.ts"]}"#,
    )
    .expect("writes config");
    fs::write(
        root.join("src/hidden.ts"),
        "export type Hidden = { id: string };",
    )
    .expect("writes hidden");
    fs::write(
        root.join("src/shared.ts"),
        "import type { Hidden } from './hidden'; export type Shared = Hidden;",
    )
    .expect("writes shared");
    fs::write(
        root.join("src/app.ts"),
        "import type { Shared } from './shared'; export type App = Shared;",
    )
    .expect("writes app");
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans");
    let symbols = candidates
        .iter()
        .map(|candidate| candidate.symbol_name.as_str())
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Hidden"));
    assert!(symbols.contains(&"Shared"));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn includes_directories_without_glob_patterns() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-include-dir-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src/nested")).expect("creates source");
    fs::create_dir_all(root.join("scripts")).expect("creates scripts");
    fs::write(root.join("tsconfig.json"), r#"{"include":["src"]}"#).expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "export type Shared = { id: string };",
    )
    .expect("writes shared");
    fs::write(
        root.join("src/nested/inner.ts"),
        "export type Inner = { id: string };",
    )
    .expect("writes inner");
    fs::write(
        root.join("scripts/tool.ts"),
        "export type Tool = { id: string };",
    )
    .expect("writes tool");
    let symbols = scan_tsconfig(&root.join("tsconfig.json"))
        .expect("scans")
        .into_iter()
        .map(|candidate| candidate.symbol_name)
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Shared".into()));
    assert!(symbols.contains(&"Inner".into()));
    assert!(!symbols.iter().any(|symbol| symbol == "Tool"));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn includes_imports_outside_an_explicit_include_file() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-include-import-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::create_dir_all(root.join("scripts")).expect("creates scripts");
    fs::write(root.join("tsconfig.json"), r#"{"include":["src/app.ts"]}"#).expect("writes config");
    fs::write(
        root.join("scripts/tool.ts"),
        "export type Tool = { id: string };",
    )
    .expect("writes tool");
    fs::write(
        root.join("src/app.ts"),
        "import type { Tool } from '../scripts/tool'; export type App = Tool;",
    )
    .expect("writes app");
    fs::write(
        root.join("src/other.ts"),
        "export type Other = { id: string };",
    )
    .expect("writes other");
    let symbols = scan_tsconfig(&root.join("tsconfig.json"))
        .expect("scans")
        .into_iter()
        .map(|candidate| candidate.symbol_name)
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Tool".into()));
    assert!(symbols.contains(&"App".into()));
    assert!(!symbols.iter().any(|symbol| symbol == "Other"));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn respects_explicit_tsconfig_files() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-files-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates fixture");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"files":["src/included.ts"]}"#,
    )
    .expect("writes config");
    fs::write(root.join("src/included.ts"), "export type Included = {};").expect("writes included");
    fs::write(root.join("src/excluded.ts"), "export type Excluded = {};").expect("writes excluded");
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans config");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol_name, "Included");
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn treats_an_explicit_empty_files_list_as_no_roots() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-empty-files-scan-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::write(root.join("tsconfig.json"), r#"{"files":[]}"#).expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "export type Shared = { id: string };",
    )
    .expect("writes shared");
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans");
    assert!(candidates.is_empty());
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn unions_explicit_files_with_include_patterns() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-files-include-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::create_dir_all(root.join("lib")).expect("creates lib");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"files":["src/shared.ts"],"include":["lib"],"exclude":["src/shared.ts"]}"#,
    )
    .expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "export type Shared = { id: string };",
    )
    .expect("writes shared");
    fs::write(
        root.join("src/other.ts"),
        "export type Other = { id: string };",
    )
    .expect("writes other");
    fs::write(
        root.join("lib/extra.ts"),
        "export type Extra = { id: string };",
    )
    .expect("writes extra");
    let symbols = scan_tsconfig(&root.join("tsconfig.json"))
        .expect("scans")
        .into_iter()
        .map(|candidate| candidate.symbol_name)
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Shared".into()));
    assert!(symbols.contains(&"Extra".into()));
    assert!(!symbols.iter().any(|symbol| symbol == "Other"));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn includes_imports_outside_an_explicit_files_list() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-files-import-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates source");
    fs::write(root.join("tsconfig.json"), r#"{"files":["src/app.ts"]}"#).expect("writes config");
    fs::write(
        root.join("src/hidden.ts"),
        "export type Hidden = { id: string };",
    )
    .expect("writes hidden");
    fs::write(
        root.join("src/app.ts"),
        "import type { Hidden } from './hidden'; export type App = Hidden;",
    )
    .expect("writes app");
    fs::write(
        root.join("src/other.ts"),
        "export type Other = { id: string };",
    )
    .expect("writes other");
    let symbols = scan_tsconfig(&root.join("tsconfig.json"))
        .expect("scans")
        .into_iter()
        .map(|candidate| candidate.symbol_name)
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Hidden".into()));
    assert!(symbols.contains(&"App".into()));
    assert!(!symbols.iter().any(|symbol| symbol == "Other"));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn expands_package_imports_from_newly_discovered_scopes() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-nested-package-import-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src/feature")).expect("creates source");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"module":"esnext","moduleResolution":"bundler"},"files":["src/app.ts"]}"#,
    )
    .expect("writes config");
    fs::write(
        root.join("src/feature/package.json"),
        r##"{"imports":{"#shared":"./shared.ts"}}"##,
    )
    .expect("writes package config");
    fs::write(
        root.join("src/feature/shared.ts"),
        "export type Shared = { id: string };",
    )
    .expect("writes shared");
    fs::write(
        root.join("src/feature/module.ts"),
        "import type { Shared } from '#shared'; export type Feature = Shared;",
    )
    .expect("writes feature");
    fs::write(
        root.join("src/app.ts"),
        "import type { Feature } from './feature/module'; export type App = Feature;",
    )
    .expect("writes app");
    let symbols = scan_tsconfig(&root.join("tsconfig.json"))
        .expect("scans")
        .into_iter()
        .map(|candidate| candidate.symbol_name)
        .collect::<Vec<_>>();
    assert!(symbols.contains(&"Shared".into()));
    assert!(symbols.contains(&"Feature".into()));
    assert!(symbols.contains(&"App".into()));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn omits_namespace_merge_declarations_from_unsupported_explain_diagnostics() {
    let candidates = scan_project_files(vec![
        (
            "src/shared.ts".into(),
            "export interface Shared { id: string }\nexport namespace Shared { export type Nested = { id: string } }\n"
                .into(),
        ),
        (
            "src/consumer.ts".into(),
            "import type { Shared } from './shared'; export type Consumer = Shared;\n".into(),
        ),
    ])
    .expect("scans");
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.symbol_name == "Shared")
    );
    let parsed = vec![(
        "src/shared.ts".into(),
        parse_source(
            "export interface Shared { id: string }\nexport namespace Shared { export type Nested = { id: string } }\n",
        )
        .expect("parses"),
    )];
    assert!(unsupported_dependency_diagnostics(&parsed).is_empty());
}

#[test]
fn reports_unsupported_dependencies_for_explain_candidates() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-unsupported-dependency-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates fixture");
    fs::write(root.join("tsconfig.json"), r#"{"include":["src/**/*.ts"]}"#).expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "const runtime = 'value'; export type RuntimeDependent = typeof runtime; export type Safe = { id: string };",
    )
    .expect("writes declarations");

    let (candidates, diagnostics) =
        scan_tsconfig_with_generated_and_diagnostics(&root.join("tsconfig.json"), false)
            .expect("scans config");
    fs::remove_dir_all(root).expect("removes fixture");

    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["RuntimeDependent", "Safe"]
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].reason, "unsupportedDependency");
    assert_eq!(diagnostics[0].symbol_name, "RuntimeDependent");
    assert_eq!(
        diagnostics[0].message,
        "Declaration depends on a runtime value, type query, or unsupported same-file declaration."
    );
}

#[test]
fn scans_parent_tsconfig_sources_from_a_nested_project() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-extends-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("packages/app/src")).expect("creates app");
    fs::create_dir_all(root.join("shared")).expect("creates shared");
    fs::write(
        root.join("base.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@shared/*":["shared/*"]}},"include":["shared/**/*.ts","packages/app/src/**/*.ts"]}"#,
    )
    .expect("writes base config");
    fs::write(
        root.join("packages/app/tsconfig.json"),
        r#"{"extends":"../../base"}"#,
    )
    .expect("writes app config");
    fs::write(
        root.join("shared/model.ts"),
        "export interface SharedModel { id: string; }",
    )
    .expect("writes shared source");
    fs::write(
        root.join("packages/app/src/consumer.ts"),
        "import type { SharedModel } from '@shared/model'; export interface Consumer { model: SharedModel; }",
    )
    .expect("writes consumer");

    let candidates =
        scan_tsconfig(&root.join("packages/app/tsconfig.json")).expect("scans inherited config");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.source_path.as_str())
            .collect::<Vec<_>>(),
        ["packages/app/src/consumer.ts", "shared/model.ts"]
    );
    assert_eq!(
        candidates[1].referrer_paths,
        ["packages/app/src/consumer.ts"]
    );
    assert!(matches!(
        candidates[1].referrer_strategies[0].strategy,
        ImportStrategy::Alias { ref module_specifier, .. } if module_specifier == "@shared/types/shared"
    ));
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn skips_mts_and_cts_files_to_match_the_typescript_cli() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-module-kinds-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates fixture");
    fs::write(root.join("tsconfig.json"), "{}").expect("writes config");
    fs::write(root.join("src/schema.mts"), "export type Schema = {};").expect("writes ESM source");
    fs::write(root.join("src/legacy.cts"), "export interface Legacy {}")
        .expect("writes CJS source");
    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans config");
    assert!(candidates.is_empty());
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn attaches_only_used_importers_as_referrers() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
        (
            "src/unused.ts".into(),
            "import type { Shared } from './shared';".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative {
            module_specifier: "./types/src".into()
        }
    );
}

#[test]
fn resolves_and_preserves_javascript_module_specifiers() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared.js'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert!(matches!(
        &candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative { module_specifier } if module_specifier == "./types/src.js"
    ));
}

#[test]
fn keeps_each_referrers_module_specifier_extension() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/javascript.ts".into(),
            "import type { Shared } from './shared.js'; type App = Shared;".into(),
        ),
        (
            "src/typescript.ts".into(),
            "import type { Shared } from './shared.ts'; type App = Shared;".into(),
        ),
        (
            "src/plain.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    let specifier = |referrer: &str| {
        candidates[0]
            .referrer_strategies
            .iter()
            .find(|strategy| strategy.referrer_path == referrer)
            .map(|strategy| match &strategy.strategy {
                ImportStrategy::Relative { module_specifier } => module_specifier.as_str(),
                ImportStrategy::Alias { .. } | ImportStrategy::Barrel { .. } => "",
            })
            .expect("finds referrer")
    };
    assert_eq!(specifier("src/javascript.ts"), "./types/src.js");
    assert_eq!(specifier("src/typescript.ts"), "./types/src.ts");
    assert_eq!(specifier("src/plain.ts"), "./types/src");
}

#[test]
fn uses_javascript_specifiers_for_nodenext_package_imports() {
    let fixture = tempfile::tempdir().expect("creates fixture project");
    let root = fixture.path();
    fs::create_dir_all(root.join("src")).expect("creates source directory");
    fs::write(
        root.join("package.json"),
        r##"{"type":"module","imports":{"#model":"./src/model.ts"}}"##,
    )
    .expect("writes package metadata");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"module":"NodeNext","moduleResolution":"NodeNext"},"include":["src/**/*.ts"]}"#,
    )
    .expect("writes tsconfig");
    fs::write(root.join("src/model.ts"), "export type Model = {};\n")
        .expect("writes declaration source");
    fs::write(
        root.join("src/app.ts"),
        "import type { Model } from '#model'; type App = Model;\n",
    )
    .expect("writes referrer");

    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans config");

    assert!(matches!(
        &candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative { module_specifier } if module_specifier == "./types/src.js"
    ));
}

#[test]
#[cfg(unix)]
fn rejects_selected_dangling_source_symlinks() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-scan-orphan-symlink-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("creates fixture");
    fs::write(root.join("tsconfig.json"), "{}\n").expect("writes config");
    fs::write(
        root.join("src/shared.ts"),
        "export type Shared = { id: string };\n",
    )
    .expect("writes shared");
    fs::write(
        root.join("src/consumer.ts"),
        "import type { Shared } from './shared'; export type Consumer = Shared;\n",
    )
    .expect("writes consumer");
    std::os::unix::fs::symlink(root.join("missing.ts"), root.join("src/orphan.ts"))
        .expect("creates dangling symlink");
    let error = scan_tsconfig(&root.join("tsconfig.json")).expect_err("rejects dangling source");
    assert!(
        error
            .to_string()
            .contains("cannot resolve relevant symlink")
    );
    fs::remove_dir_all(root).expect("removes fixture");
}

#[test]
fn allows_type_alias_and_interface_with_the_same_exported_name() {
    let candidates = scan_project_files(vec![
        (
            "src/shared.ts".into(),
            "export type Shared = { id: string };\nexport interface Shared { label: string }\n"
                .into(),
        ),
        (
            "src/consumer.ts".into(),
            "import type { Shared } from './shared'; export type Consumer = Shared;\n".into(),
        ),
    ])
    .expect("scans");
    let shared_candidates = candidates
        .iter()
        .filter(|candidate| candidate.source_path == "src/shared.ts")
        .collect::<Vec<_>>();
    assert_eq!(shared_candidates.len(), 2);
    assert_eq!(
        shared_candidates
            .iter()
            .map(|candidate| candidate.kind.as_str())
            .collect::<Vec<_>>(),
        ["interface", "typeAlias"],
    );
}

#[test]
fn excludes_candidates_with_unsupported_dependencies_or_merges() {
    for source in [
        "const fallback = { id: 'fallback' }; export type Shared = typeof fallback;",
        "import { Runtime } from './runtime'; export type Shared = Runtime;",
        "interface Local { id: string } interface Local { label: string } export type Shared = Local;",
        "export interface Shared { id: string } export interface Shared { label: string }",
        "class Local {} export type Shared = Local;",
        "namespace Local { export type Nested = {} } export type Shared = Local.Nested;",
    ] {
        let candidates = scan_project_files(vec![
            ("src/runtime.ts".into(), "export class Runtime {}".into()),
            ("src/shared.ts".into(), source.into()),
            (
                "src/app.ts".into(),
                "import type { Shared } from './shared'; type App = Shared;".into(),
            ),
        ])
        .expect("scans");
        assert!(
            candidates.is_empty(),
            "expected no candidates for {source}: {candidates:?}"
        );
    }
}

#[test]
fn records_transitive_same_file_type_dependencies() {
    let candidates = scan_project_files(vec![
        (
            "src/shared.ts".into(),
            "type Detail = Id; type Id = string; export type Shared = Detail;".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].local_dependency_names, ["Detail", "Id"]);
    assert_eq!(candidates[0].local_dependency_declaration_keys.len(), 2);
}

#[test]
fn attaches_alias_importers_and_preserves_alias_output() {
    let candidates = scan_project_files_with_aliases(
        vec![
            (
                "src/feature/schema.ts".into(),
                "export type Schema = {};".into(),
            ),
            (
                "src/app.ts".into(),
                "import type { Schema } from '@/feature/schema'; type App = Schema;".into(),
            ),
        ],
        Some("."),
        &std::collections::BTreeMap::from([("@/*".into(), vec!["src/*".into()])]),
    )
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: "@/feature/types/feature".into(),
        }
    );
}

#[test]
fn treats_dot_prefixed_aliases_as_non_relative() {
    let candidates = scan_project_files_with_aliases(
        vec![
            (
                "src/feature/schema.ts".into(),
                "export type Schema = {};".into(),
            ),
            (
                "src/.types/feature/schema.ts".into(),
                "export type Schema = {};".into(),
            ),
            (
                "src/app.ts".into(),
                "import type { Schema } from '.types/feature/schema'; type App = Schema;".into(),
            ),
        ],
        Some("."),
        &BTreeMap::from([(".types/*".into(), vec!["src/*".into()])]),
    )
    .expect("scans");
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.source_path == "src/feature/schema.ts")
        .expect("finds alias target");
    assert_eq!(candidate.referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: ".types/feature/types/feature".into(),
        }
    );
}

#[test]
fn overrides_an_alias_candidate_destination() {
    let aliases = BTreeMap::from([("@/*".into(), vec!["src/*".into()])]);
    let candidate = scan_project_files_with_aliases(
        vec![
            ("src/shared.ts".into(), "export type Shared = {};".into()),
            (
                "src/app.ts".into(),
                "import type { Shared } from '@/shared'; type App = Shared;".into(),
            ),
        ],
        Some("."),
        &aliases,
    )
    .expect("scans")
    .pop()
    .expect("finds candidate");
    let candidate = super::override_candidate_destination(
        candidate,
        "src/extracted/shared.ts".into(),
        Some("."),
        &aliases,
    );

    assert_eq!(candidate.destination_path, "src/extracted/shared.ts");
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: "@/extracted/shared".into(),
        }
    );
}

#[test]
fn falls_back_to_relative_when_destination_alias_is_shadowed() {
    let aliases = BTreeMap::from([
        ("@/*".into(), vec!["src/*".into()]),
        ("@/types/*".into(), vec!["other/*".into()]),
    ]);
    let candidate = scan_project_files_with_aliases(
        vec![
            ("src/shared.ts".into(), "export type Shared = {};".into()),
            (
                "src/app.ts".into(),
                "import type { Shared } from '@/shared'; type App = Shared;".into(),
            ),
        ],
        Some("."),
        &aliases,
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Shared")
    .expect("finds candidate");
    let candidate = super::override_candidate_destination(
        candidate,
        "src/types/shared.ts".into(),
        Some("."),
        &aliases,
    );
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Relative {
            module_specifier: "./types/shared".into(),
        }
    );
}

#[test]
fn preserves_aliases_targeting_a_sibling_of_base_url() {
    let aliases = BTreeMap::from([("@/*".into(), vec!["../shared/*".into()])]);
    let candidate = scan_project_files_with_aliases(
        vec![
            ("shared/model.ts".into(), "export type Model = {};".into()),
            (
                "src/app.ts".into(),
                "import type { Model } from '@/model'; type App = Model;".into(),
            ),
        ],
        Some("src"),
        &aliases,
    )
    .expect("scans")
    .into_iter()
    .find(|candidate| candidate.symbol_name == "Model")
    .expect("finds model");
    let candidate = super::override_candidate_destination(
        candidate,
        "shared/types/model.ts".into(),
        Some("src"),
        &aliases,
    );
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: "@/types/model".into(),
        }
    );
}

#[test]
fn attaches_barrel_importers_and_preserves_the_barrel() {
    let candidates = scan_project_files(vec![
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export type { Schema } from './schema';".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from './feature'; type App = Schema;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].destination_index_path.as_deref(),
        Some("src/feature/index.ts")
    );
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Barrel {
            barrel_file_path: "src/feature/index.ts".into(),
            export_module_specifier: "./types/feature".into(),
        }
    );
}

#[test]
fn keeps_a_tilde_alias_on_a_barrel_reexport() {
    let candidates = scan_project_files_with_aliases(
        vec![
            (
                "services/domains/csv-order/types.ts".into(),
                "export type CsvOrder = { id: number };".into(),
            ),
            (
                "features/csv-order/types/group.ts".into(),
                "export type { CsvOrder } from '~/services/domains/csv-order/types';\n".into(),
            ),
            (
                "src/app.ts".into(),
                "import type { CsvOrder } from '../features/csv-order/types/group'; type App = CsvOrder;"
                    .into(),
            ),
        ],
        Some("."),
        &BTreeMap::from([("~/*".into(), vec!["./*".into()])]),
    )
    .expect("scans");
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "CsvOrder")
        .expect("finds csv order");
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Barrel {
            barrel_file_path: "features/csv-order/types/group.ts".into(),
            export_module_specifier: "~/services/domains/csv-order/types/csv-order".into(),
        }
    );
}

#[test]
fn attaches_wildcard_barrel_importers() {
    let candidates = scan_project_files(vec![
        (
            "src/feature/schema.ts".into(),
            "export type Schema = {};".into(),
        ),
        (
            "src/feature/index.ts".into(),
            "export * from './schema';".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Schema } from './feature'; type App = Schema;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Barrel {
            barrel_file_path: "src/feature/index.ts".into(),
            export_module_specifier: "./types/feature".into(),
        }
    );
}

#[test]
fn uses_a_relative_strategy_for_aliased_barrel_exports() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/index.ts".into(),
            "export type { Shared as PublicShared } from './shared';".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { PublicShared } from './index'; type App = PublicShared;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative {
            module_specifier: "./types/src".into(),
        }
    );
    assert_eq!(candidates[0].destination_index_path, None);
}

#[test]
fn attaches_namespace_type_importers() {
    let candidates = scan_project_files(vec![
        ("src/schema.ts".into(), "export type Schema = {};".into()),
        (
            "src/app.ts".into(),
            "import type * as Types from './schema'; type App = Types.Schema;".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative {
            module_specifier: "./types/src".into(),
        }
    );
}

#[test]
fn preserves_explicit_alias_without_base_url() {
    let fixture = tempfile::tempdir().expect("creates fixture");
    let root = fixture.path();
    fs::create_dir_all(root.join("app/hooks/cart")).unwrap();
    fs::create_dir_all(root.join("app/components/detail")).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"include":["app/**/*.ts"],"compilerOptions":{"paths":{"~/*":["./app/*"]}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("app/hooks/cart/use-cart.ts"),
        "export type Cart = { id: string };\n",
    )
    .unwrap();
    fs::write(
        root.join("app/components/detail/view.ts"),
        "import type { Cart } from '~/hooks/cart/use-cart'; export type View = Cart;\n",
    )
    .unwrap();
    let candidate = scan_tsconfig(&root.join("tsconfig.json"))
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.symbol_name == "Cart")
        .unwrap();
    assert_eq!(
        candidate.referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: "~/hooks/cart/types/cart".into()
        }
    );
}

#[test]
fn uses_tsconfig_path_aliases_for_project_referrers() {
    let root = std::env::temp_dir().join(format!(
        "type-tune-project-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src/feature")).expect("creates fixture");
    fs::write(
        root.join("tsconfig.json"),
        r#"{"include":["src/**/*.ts"],"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    )
    .expect("writes config");
    fs::write(
        root.join("src/feature/schema.ts"),
        "export type Schema = {};",
    )
    .expect("writes declaration");
    fs::write(
        root.join("src/app.ts"),
        "import type { Schema } from '@/feature/schema'; type App = Schema;",
    )
    .expect("writes referrer");

    let candidates = scan_tsconfig(&root.join("tsconfig.json")).expect("scans config");
    fs::remove_dir_all(root).expect("removes fixture");

    assert_eq!(candidates[0].referrer_paths, ["src/app.ts"]);
    assert!(matches!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Alias { .. }
    ));
}

#[test]
fn filters_candidates_by_minimum_referrers() {
    let candidates =
        scan_declarations([("src/shared.ts".into(), "export type Shared = {};".into())])
            .expect("scans");
    assert!(filter_min_referrers(candidates, 1).is_empty());
}

#[test]
fn records_command_level_omission_diagnostics() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    let (candidates, diagnostics) = filter_candidates_with_diagnostics(
        candidates,
        ScanFilterOptions {
            minimum_referrers: 2,
            referrer_count_exclusions: &[],
            include_kinds: &["typeAlias".into(), "interface".into()],
            ignore_entries: &[],
            include_existing_types: false,
            placement_style: crate::destination::PlacementStyle::Collocation,
            base_url: None,
            path_aliases: &BTreeMap::new(),
        },
    );

    assert!(candidates.is_empty());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].reason, "insufficientReferrers");
    assert_eq!(diagnostics[0].referrer_count, Some(1));
    assert_eq!(diagnostics[0].min_referrers, Some(2));
}

#[test]
fn recomputes_relative_referrer_strategies_for_repo_types_placement() {
    let candidates = scan_project_files(vec![
        (
            "src/feature/shared.ts".into(),
            "export type Shared = { id: string };".into(),
        ),
        (
            "src/app.ts".into(),
            "import type { Shared } from './feature/shared'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    let (candidates, _) = filter_candidates_with_diagnostics(
        candidates,
        ScanFilterOptions {
            minimum_referrers: 1,
            referrer_count_exclusions: &[],
            include_kinds: &["typeAlias".into(), "interface".into()],
            ignore_entries: &[],
            include_existing_types: false,
            placement_style: crate::destination::PlacementStyle::RepoTypes,
            base_url: None,
            path_aliases: &BTreeMap::new(),
        },
    );

    assert_eq!(
        candidates[0].destination_path,
        "types/src/feature/types/feature.ts"
    );
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Relative {
            module_specifier: "../types/src/feature/types/feature".into(),
        }
    );
}

#[test]
fn preserves_alias_referrer_strategies_for_repo_types_placement() {
    let aliases = BTreeMap::from([("@types/*".into(), vec!["types/*".into()])]);
    let candidates = scan_project_files_with_aliases(
        vec![
            (
                "types/shared.ts".into(),
                "export type Shared = { id: string };".into(),
            ),
            (
                "src/app.ts".into(),
                "import type { Shared } from '@types/shared'; type App = Shared;".into(),
            ),
        ],
        Some("."),
        &aliases,
    )
    .expect("scans");
    let candidates = apply_placement_with_aliases(
        candidates,
        crate::destination::PlacementStyle::RepoTypes,
        Some("."),
        &aliases,
    );

    assert_eq!(candidates[0].destination_path, "types/shared.ts");
    assert_eq!(
        candidates[0].referrer_strategies[0].strategy,
        ImportStrategy::Alias {
            module_specifier: "@types/shared".into(),
        }
    );
}

#[test]
fn excludes_noop_destinations_from_plan_candidates() {
    let candidates = scan_project_files(vec![
        ("types/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from '@types/shared'; type App = Shared;".into(),
        ),
    ])
    .expect("scans");
    let placed = apply_placement_with_aliases(
        candidates,
        crate::destination::PlacementStyle::RepoTypes,
        Some("."),
        &BTreeMap::from([("@types/*".into(), vec!["types/*".into()])]),
    );
    let filtered = filter_noop_destinations(placed);
    assert!(filtered.is_empty());
}

#[test]
fn filters_candidates_matched_by_ignore_entries() {
    let candidates =
        scan_declarations([("src/shared.ts".into(), "export type Shared = {};".into())])
            .expect("scans");
    assert!(filter_ignored_candidates(candidates, &["src/shared.ts#Shared".into()]).is_empty());
}

#[test]
fn excludes_only_the_eligibility_count() {
    let candidates = scan_project_files(vec![
        ("src/shared.ts".into(), "export type Shared = {};".into()),
        (
            "src/app.ts".into(),
            "import type { Shared } from './shared'; type App = Shared;".into(),
        ),
        (
            "src/shared.test.ts".into(),
            "import type { Shared } from './shared'; type Test = Shared;".into(),
        ),
    ])
    .expect("scans");
    let candidates = apply_referrer_count_exclusions(candidates, &["**/*.test.ts".into()]);
    assert_eq!(
        candidates[0].referrer_paths,
        ["src/app.ts", "src/shared.test.ts"]
    );
    assert_eq!(candidates[0].counted_referrer_count, Some(1));
}

#[test]
fn excludes_standalone_types_ts_unless_requested() {
    let candidates = scan_declarations([
        (
            "src/feature/types.ts".into(),
            "export type Shared = {};".into(),
        ),
        (
            "src/feature/model.ts".into(),
            "export type Model = {};".into(),
        ),
    ])
    .expect("scans");
    let shared = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "Shared")
        .unwrap();
    let id = shared.id.clone();
    let default = filter_existing_types(candidates.clone(), false);
    assert_eq!(
        default
            .iter()
            .map(|candidate| candidate.symbol_name.as_str())
            .collect::<Vec<_>>(),
        ["Model"]
    );
    let included = filter_existing_types(candidates, true);
    assert_eq!(
        included
            .iter()
            .find(|candidate| candidate.symbol_name == "Shared")
            .unwrap()
            .id,
        id
    );
}

#[test]
fn explains_existing_type_locations_and_preserves_other_filters() {
    let candidates = scan_project_files(vec![
        ("src/feature/types.ts".into(), "export type Existing = {};".into()),
        ("src/feature/types/shared.ts".into(), "export type DirectoryType = {};".into()),
        ("src/feature/model.ts".into(), "export type Model = {}; export type WidgetProps = {};".into()),
        ("src/feature/types.tsx".into(), "export type TsxType = {};".into()),
        ("src/feature/sometypes.ts".into(), "export type NamedType = {};".into()),
        ("src/one.ts".into(), "import type { Existing } from './feature/types'; import type { DirectoryType } from './feature/types/shared'; import type { Model, WidgetProps } from './feature/model'; import type { TsxType } from './feature/types.tsx'; import type { NamedType } from './feature/sometypes'; type One = [Existing, DirectoryType, Model, WidgetProps, TsxType, NamedType];".into()),
        ("src/two.ts".into(), "import type { Existing } from './feature/types'; import type { DirectoryType } from './feature/types/shared'; import type { Model, WidgetProps } from './feature/model'; import type { TsxType } from './feature/types.tsx'; import type { NamedType } from './feature/sometypes'; type Two = [Existing, DirectoryType, Model, WidgetProps, TsxType, NamedType];".into()),
    ]).expect("scans");
    let types_id = candidates
        .iter()
        .find(|candidate| candidate.symbol_name == "Existing")
        .unwrap()
        .id
        .clone();
    let kinds = vec!["typeAlias".into(), "interface".into()];
    let aliases = BTreeMap::new();
    let options = |include_existing_types| ScanFilterOptions {
        minimum_referrers: 2,
        referrer_count_exclusions: &[],
        include_kinds: &kinds,
        ignore_entries: &[],
        include_existing_types,
        placement_style: crate::destination::PlacementStyle::Collocation,
        base_url: None,
        path_aliases: &aliases,
    };
    let (filtered, diagnostics) =
        filter_candidates_with_diagnostics(candidates.clone(), options(false));
    for name in ["Model", "WidgetProps", "TsxType", "NamedType"] {
        assert!(
            filtered
                .iter()
                .any(|candidate| candidate.symbol_name == name),
            "{name}"
        );
    }
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.symbol_name == "Existing")
        .unwrap();
    assert_eq!(diagnostic.reason, "existingTypesFile");
    assert_eq!(diagnostic.candidate_id, types_id);
    assert!(
        diagnostic.message.contains("types.ts")
            && diagnostic.message.contains("--include-existing-types")
    );
    assert_eq!(
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.symbol_name == "DirectoryType")
            .unwrap()
            .reason,
        "existingTypesDirectory"
    );
    let (included, diagnostics) =
        filter_candidates_with_diagnostics(candidates.clone(), options(true));
    assert_eq!(
        included
            .iter()
            .find(|candidate| candidate.symbol_name == "Existing")
            .unwrap()
            .id,
        types_id
    );
    assert_eq!(
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.symbol_name == "DirectoryType")
            .unwrap()
            .reason,
        "noopDestination"
    );
    let (again, _) = filter_candidates_with_diagnostics(candidates, options(true));
    assert_eq!(
        included
            .iter()
            .map(|candidate| &candidate.id)
            .collect::<Vec<_>>(),
        again
            .iter()
            .map(|candidate| &candidate.id)
            .collect::<Vec<_>>()
    );
    let ignored = vec![types_id];
    let mut restricted = options(true);
    restricted.ignore_entries = &ignored;
    restricted.minimum_referrers = 3;
    let (_, diagnostics) = filter_candidates_with_diagnostics(again, restricted);
    assert_eq!(
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.symbol_name == "Existing")
            .unwrap()
            .reason,
        "ignored"
    );
    assert_eq!(
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.symbol_name == "Model")
            .unwrap()
            .reason,
        "insufficientReferrers"
    );
}

#[test]
fn excludes_existing_types_unless_requested() {
    let candidates = scan_declarations([
        (
            "src/types/shared.ts".into(),
            "export type Shared = {};".into(),
        ),
        (
            "src/domain/model.ts".into(),
            "export type Model = {};".into(),
        ),
    ])
    .expect("scans");
    assert_eq!(filter_existing_types(candidates, false).len(), 1);
}
