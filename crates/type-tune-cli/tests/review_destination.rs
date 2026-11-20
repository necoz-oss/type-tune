use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "type-tune-review-destination-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/feature")).unwrap();
        let fixture = Self(root);
        fixture.write("tsconfig.json", r#"{"compilerOptions":{"strict":true,"noEmit":true,"baseUrl":".","paths":{"@/*":["src/*"]},"ignoreDeprecations":"6.0"},"include":["src"]}"#);
        fixture.write(
            "src/feature/source.ts",
            "export type A = { a: string };\nexport type B = { b: number };\n",
        );
        fixture.write(
            "src/feature/index.ts",
            "export type { A, B } from './source';\n",
        );
        fixture.write(
            "src/one.ts",
            "import type { A, B } from '@/feature/source';\nexport type One = A & B;\n",
        );
        fixture.write(
            "src/two.ts",
            "import type { A, B } from '@/feature';\nexport type Two = A & B;\n",
        );
        fixture.write("src/other.ts", "export type C = { c: boolean };\n");
        fixture.write(
            "src/three.ts",
            "import type { C } from './other';\nexport type Three = C;\n",
        );
        fixture.write(
            "src/four.ts",
            "import type { C } from './other';\nexport type Four = C;\n",
        );
        fixture
    }
    fn write(&self, path: &str, content: &str) {
        let p = self.0.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/.bin");
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.0)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .args(args)
            .output()
            .unwrap()
    }
    fn plan(&self) {
        let out = self.run(&[
            "plan",
            "--project",
            "tsconfig.json",
            "--out",
            "plan.json",
            "--markdown",
            "plan.md",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn select(&self, assignments: &[(&str, &str)]) {
        let plan: serde_json::Value =
            serde_json::from_slice(&fs::read(self.0.join("plan.json")).unwrap()).unwrap();
        let mut markdown = fs::read_to_string(self.0.join("plan.md")).unwrap();
        for (name, destination) in assignments {
            let candidate = plan["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["candidate"]["symbolName"] == *name)
                .unwrap();
            let id = candidate["candidate"]["id"].as_str().unwrap();
            let from = format!("- [ignore] {id} `{name}`");
            let start = markdown.find(&from).unwrap();
            let end = markdown[start..]
                .find("\n\n")
                .map_or(markdown.len(), |i| start + i);
            let entry = &markdown[start..end];
            let old = format!(
                "  - to: `{}`",
                candidate["candidate"]["destinationPath"].as_str().unwrap()
            );
            let new = format!("  - to: `{destination}`");
            let edited = entry
                .replace(&from, &format!("- [apply] {id} `{name}`"))
                .replace(&old, &new);
            markdown.replace_range(start..end, &edited);
        }
        self.write("plan.md", &markdown);
    }
    fn apply(&self, dry: bool) -> Output {
        let mut args = vec!["apply", "--plan", "plan.json", "--markdown", "plan.md"];
        if dry {
            args.push("--dry-run");
        }
        self.run(&args)
    }
}
fn error(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}
fn assert_rejected(out: Output, expected: &str) {
    assert!(!out.status.success(), "unexpected success");
    assert!(error(&out).contains(expected), "{}", error(&out));
}

#[test]
fn rejects_invalid_assignments_and_untrusted_plans_before_writing() {
    for (assignments, expected) in [
        (vec![("A", "../outside.ts")], "Invalid reviewed destination"),
        (
            vec![("A", "/tmp/outside.ts")],
            "Invalid reviewed destination",
        ),
        (vec![("A", "src/feature/source.ts")], "must differ"),
        (vec![("A", "src/other.ts")], "Destination already exists"),
        (
            vec![("A", "src/new/a.ts"), ("B", "src/new/b.ts")],
            "share one destination",
        ),
    ] {
        let f = Fixture::new();
        f.plan();
        f.select(&assignments);
        assert_rejected(f.apply(true), expected);
        assert!(!f.0.join("src/new").exists());
    }
    let f = Fixture::new();
    f.plan();
    f.select(&[("A", "src/new/a.ts")]);
    let mut plan: serde_json::Value =
        serde_json::from_slice(&fs::read(f.0.join("plan.json")).unwrap()).unwrap();
    let entry = plan["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["candidate"]["symbolName"] == "A")
        .unwrap();
    entry["candidate"]["destinationPath"] = "src/new/tampered.ts".into();
    f.write("plan.json", &serde_json::to_string(&plan).unwrap());
    assert_rejected(f.apply(true), "Invalid candidate metadata");
    let f = Fixture::new();
    f.plan();
    f.select(&[("A", "src/new/a.ts")]);
    f.write(
        "src/feature/source.ts",
        "export type A = { a: number };\nexport type B = { b: number };\n",
    );
    assert_rejected(f.apply(true), "Stale plan");
}

#[cfg(unix)]
#[test]
fn rejects_destination_through_outside_symlink() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    f.write("tsconfig.json", r#"{"compilerOptions":{"strict":true,"noEmit":true,"baseUrl":".","paths":{"@/*":["src/*"]},"ignoreDeprecations":"6.0"},"include":["src"],"exclude":["src/escape"]}"#);
    fs::create_dir(outside.0.join("empty")).unwrap();
    symlink(outside.0.join("empty"), f.0.join("src/escape")).unwrap();
    f.plan();
    f.select(&[("A", "src/escape/new.ts")]);
    assert_rejected(f.apply(true), "escapes project root");
}

#[test]
fn applies_custom_destinations_and_rolls_back_dry_run() {
    let tsc = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/.bin")
        .join(if cfg!(windows) { "tsc.cmd" } else { "tsc" });
    if !tsc.exists() {
        eprintln!("Skipping compiler integration: project TypeScript is not installed");
        return;
    }
    let f = Fixture::new();
    f.plan();
    f.select(&[
        ("A", "src/domain/types/shared.ts"),
        ("B", "src/domain/types/shared.ts"),
        ("C", "src/separate/types/other.ts"),
    ]);
    let before = fs::read(f.0.join("src/feature/source.ts")).unwrap();
    let out = f.apply(true);
    assert!(out.status.success(), "{}", error(&out));
    assert_eq!(before, fs::read(f.0.join("src/feature/source.ts")).unwrap());
    assert!(!f.0.join("src/domain").exists());
    assert!(!f.0.join("src/separate").exists());
    let out = f.apply(false);
    assert!(out.status.success(), "{}", error(&out));
    let dest = fs::read_to_string(f.0.join("src/domain/types/shared.ts")).unwrap();
    assert!(dest.contains("type A") && dest.contains("type B"));
    assert!(f.0.join("src/separate/types/other.ts").exists());
    let referrer = fs::read_to_string(f.0.join("src/one.ts")).unwrap();
    assert!(referrer.contains("@/domain/types/shared"), "{referrer}");
    let barrel = fs::read_to_string(f.0.join("src/feature/index.ts")).unwrap();
    assert!(barrel.contains("domain/types/shared"), "{barrel}");
}

#[test]
fn compiler_failure_rolls_back_custom_destination() {
    let tsc = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/.bin")
        .join(if cfg!(windows) { "tsc.cmd" } else { "tsc" });
    if !tsc.exists() {
        return;
    }
    let f = Fixture::new();
    f.write("src/broken.ts", "export const broken: string = 42;\n");
    f.plan();
    f.select(&[("A", "src/new/nested/a.ts")]);
    let before = fs::read(f.0.join("src/feature/source.ts")).unwrap();
    assert_rejected(f.apply(true), "rolled back");
    assert_eq!(before, fs::read(f.0.join("src/feature/source.ts")).unwrap());
    assert!(!f.0.join("src/new").exists());
    assert_eq!(
        fs::read_to_string(f.0.join("src/broken.ts")).unwrap(),
        "export const broken: string = 42;\n"
    );
}
