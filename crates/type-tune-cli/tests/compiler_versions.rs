//! Real compiler compatibility tests. Run with the pinned packages in tests/compiler-versions.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(version: &str, options: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "type-tune-compiler-version-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("scratch")).unwrap();
        fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        let fixture = Self { root };
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let package = if version == "60" {
            repo.join("node_modules/typescript")
        } else {
            repo.join(format!(
                "tests/compiler-versions/node_modules/typescript{version}"
            ))
        };
        assert!(
            package.join("bin/tsc").is_file(),
            "Run npm ci --prefix tests/compiler-versions"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&package, fixture.root.join("node_modules/typescript"))
                .unwrap();
            std::os::unix::fs::symlink(
                "../typescript/bin/tsc",
                fixture.root.join("node_modules/.bin/tsc"),
            )
            .unwrap();
        }
        #[cfg(not(unix))]
        panic!("real compiler compatibility tests require Linux CI");
        fixture.write("tsconfig.json", &format!(r#"{{"compilerOptions":{{"strict":true,"target":"ES2020",{options}}},"include":["src"]}}"#));
        fixture.write("src/model.ts", "export interface User { name: string }\n");
        fixture.write(
            "src/a.ts",
            "import type { User } from './model'; export type A = User;\n",
        );
        fixture.write(
            "src/b.ts",
            "import type { User } from './model'; export type B = User;\n",
        );
        fixture
    }
    fn write(&self, p: &str, s: &str) {
        let path = self.root.join(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, s).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.root)
            .env("TMPDIR", self.root.join("scratch"))
            .args(args)
            .output()
            .unwrap()
    }
    fn native(&self) -> Output {
        Command::new(self.root.join("node_modules/.bin/tsc"))
            .current_dir(&self.root)
            .args(["--noEmit", "--project", "tsconfig.json"])
            .output()
            .unwrap()
    }
    fn plan(&self) {
        let p = self.run(&[
            "plan",
            "--project",
            "tsconfig.json",
            "--out",
            "plan.json",
            "--markdown",
            "plan.md",
        ]);
        assert!(p.status.success(), "{}", text(&p));
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(self.root.join("plan.json")).unwrap()).unwrap();
        let id = json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["candidate"]["symbolName"] == "User")
            .unwrap()["candidate"]["id"]
            .as_str()
            .unwrap();
        let markdown = fs::read_to_string(self.root.join("plan.md")).unwrap();
        let reviewed = markdown.replace(
            &format!("- [ignore] {id} `User`"),
            &format!("- [apply] {id} `User`"),
        );
        assert_ne!(markdown, reviewed);
        self.write("plan.md", &reviewed);
    }
}
fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            let m = fs::symlink_metadata(&p).unwrap();
            if m.file_type().is_symlink() {
                out.insert(rel, format!("link:{:?}", fs::read_link(&p).unwrap()));
            } else if m.is_dir() {
                out.insert(rel, "dir".into());
                visit(root, &p, out);
            } else {
                out.insert(rel, format!("file:{:?}", fs::read(p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
#[ignore = "requires npm ci --prefix tests/compiler-versions"]
fn legacy_non_incremental_reviewed_dry_run_and_apply() {
    for version in ["49", "55"] {
        let f = Fixture::new(version, "\"noEmit\":true");
        let native = f.native();
        assert!(native.status.success(), "{}", text(&native));
        f.plan();
        let before = snapshot(&f.root);
        let dry = f.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(dry.status.success(), "TypeScript {version}: {}", text(&dry));
        assert!(snapshot(&f.root) == before, "dry-run changed the snapshot");
        let applied = f.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
        assert!(applied.status.success(), "{}", text(&applied));
        let after = snapshot(&f.root);
        let changed = before
            .keys()
            .chain(after.keys())
            .filter(|key| before.get(*key) != after.get(*key))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let expected = [
            "src/model.ts",
            "src/a.ts",
            "src/b.ts",
            "src/types",
            "src/types/src.ts",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(changed, expected);
        assert!(f.native().status.success());
    }
}

#[test]
#[ignore = "requires npm ci --prefix tests/compiler-versions and root pnpm install"]
fn real_compilers_preserve_incremental_and_inherited_artifacts() {
    for (version, options) in [
        (
            "55",
            r#""incremental":true,"tsBuildInfoFile":"cache/custom.tsbuildinfo""#,
        ),
        (
            "55",
            r#""composite":true,"tsBuildInfoFile":"cache/custom.tsbuildinfo""#,
        ),
        (
            "58",
            r#""incremental":true,"tsBuildInfoFile":"cache/custom.tsbuildinfo""#,
        ),
        (
            "60",
            r#""incremental":true,"tsBuildInfoFile":"cache/custom.tsbuildinfo""#,
        ),
    ] {
        let f = Fixture::new(version, options);
        f.plan();
        let before = snapshot(&f.root);
        let dry = f.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(dry.status.success(), "{version}: {}", text(&dry));
        assert!(
            snapshot(&f.root) == before,
            "new artifact leaked for {version}"
        );
        let native = f.native();
        assert!(native.status.success(), "{}", text(&native));
        let existing = snapshot(&f.root);
        assert!(f.root.join("cache/custom.tsbuildinfo").is_file());
        let dry = f.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(dry.status.success(), "{}", text(&dry));
        assert!(
            snapshot(&f.root) == existing,
            "existing artifact changed for {version}"
        );
        let original_artifact = fs::read(f.root.join("cache/custom.tsbuildinfo")).unwrap();
        let apply = f.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
        assert!(apply.status.success(), "{}", text(&apply));
        assert_eq!(
            fs::read(f.root.join("cache/custom.tsbuildinfo")).unwrap(),
            original_artifact
        );
        assert!(f.native().status.success());
    }
    for (base, child) in [
        (
            r#""incremental":true,"tsBuildInfoFile":"cache/inherited.tsbuildinfo""#,
            "",
        ),
        (
            r#""composite":true,"tsBuildInfoFile":"cache/inherited.tsbuildinfo""#,
            "",
        ),
        (r#""incremental":true"#, r#""incremental":false"#),
    ] {
        let f = Fixture::new("55", "\"noEmit\":true");
        f.write("base.json", &format!(r#"{{"compilerOptions":{{{base}}}}}"#));
        f.write(
            "tsconfig.json",
            &format!(
                r#"{{"extends":"./base.json","compilerOptions":{{{child}}},"include":["src"]}}"#
            ),
        );
        f.plan();
        let before = snapshot(&f.root);
        let dry = f.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(
            dry.status.success(),
            "base={base}, child={child}: {}",
            text(&dry)
        );
        assert!(
            snapshot(&f.root) == before,
            "inherited configuration leaked build info"
        );
    }
    let f = Fixture::new("55", "\"noEmit\":true");
    f.write(
        "node_modules/example-config/package.json",
        r#"{"name":"example-config","tsconfig":"base.json"}"#,
    );
    f.write("node_modules/example-config/base.json", r#"{"compilerOptions":{"incremental":true,"tsBuildInfoFile":"../../cache/package.tsbuildinfo"}}"#);
    f.write(
        "tsconfig.json",
        r#"{"extends":"example-config","include":["src"]}"#,
    );
    f.plan();
    let before = snapshot(&f.root);
    let dry = f.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(dry.status.success(), "package extends: {}", text(&dry));
    assert!(
        snapshot(&f.root) == before,
        "package config leaked build info"
    );
}

#[test]
#[ignore = "requires npm ci --prefix tests/compiler-versions and root pnpm install"]
fn compiler_errors_and_invalid_options_are_not_suppressed() {
    for version in ["55", "60"] {
        let f = Fixture::new(version, r#""composite":true,"incremental":false"#);
        f.plan();
        let before = snapshot(&f.root);
        for dry in [true, false] {
            let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
            if dry {
                args.push("--dry-run");
            }
            let output = f.run(&args);
            assert!(!output.status.success());
            assert!(text(&output).contains("TS6379"), "{}", text(&output));
            assert!(snapshot(&f.root) == before, "invalid config leaked files");
        }
        let f = Fixture::new(version, r#""incremental":true"#);
        f.write("src/broken.ts", "export const broken: string = 42;\n");
        f.plan();
        let before = snapshot(&f.root);
        for dry in [true, false] {
            let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
            if dry {
                args.push("--dry-run");
            }
            let output = f.run(&args);
            assert!(!output.status.success());
            assert!(text(&output).contains("TS2322"), "{}", text(&output));
            assert!(
                snapshot(&f.root) == before,
                "failed compiler validation leaked files"
            );
        }
    }
    let f = Fixture::new("55", r#""tsBuildInfoFile":"cache/custom.tsbuildinfo""#);
    f.plan();
    let before = snapshot(&f.root);
    let output = f.run(&[
        "apply",
        "--plan",
        "plan.json",
        "--markdown",
        "plan.md",
        "--dry-run",
    ]);
    assert!(!output.status.success());
    assert!(text(&output).contains("TS5069"), "{}", text(&output));
    assert!(snapshot(&f.root) == before);
}

#[test]
#[ignore = "requires npm ci --prefix tests/compiler-versions and root pnpm install"]
fn modern_non_incremental_compilers_accept_reviewed_moves() {
    for version in ["58", "60"] {
        let f = Fixture::new(version, r#""incremental":false"#);
        assert!(f.native().status.success());
        f.plan();
        let before = snapshot(&f.root);
        let dry = f.run(&[
            "apply",
            "--plan",
            "plan.json",
            "--markdown",
            "plan.md",
            "--dry-run",
        ]);
        assert!(dry.status.success(), "{}", text(&dry));
        assert!(snapshot(&f.root) == before);
        let apply = f.run(&["apply", "--plan", "plan.json", "--markdown", "plan.md"]);
        assert!(apply.status.success(), "{}", text(&apply));
        assert!(f.native().status.success());
    }
}
