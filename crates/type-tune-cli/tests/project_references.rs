use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "type-tune-refs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let f = Self(root);
        f.write("tsconfig.json", r#"{"include":["src"]}"#);
        f.write("src/model.ts", "export interface User { name: string }\n");
        for name in ["first", "second"] {
            f.write(&format!("src/{name}.ts"), &format!("import type {{ User }} from './model'; export const {name} = (u: User) => u.name;\n"));
        }
        f
    }
    fn write(&self, path: &str, content: &str) {
        fs::write(self.0.join(path), content).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_type-tune"))
            .current_dir(&self.0)
            .env(
                "PATH",
                std::env::join_paths(std::iter::once(self.0.join("bin")).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ))
                .unwrap(),
            )
            .args(args)
            .output()
            .unwrap()
    }
    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                let key = path.strip_prefix(root).unwrap().to_path_buf();
                let meta = fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    result.insert(key, b"directory".to_vec());
                    walk(root, &path, result);
                } else if meta.is_symlink() {
                    result.insert(
                        key,
                        fs::read_link(&path)
                            .unwrap()
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                    );
                } else {
                    result.insert(key, fs::read(&path).unwrap());
                }
            }
        }
        let mut result = BTreeMap::new();
        walk(&self.0, &self.0, &mut result);
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn rejected(out: Output, message: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn rejects_references_across_commands_before_writes_or_compiler_execution() {
    let f = Fixture::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir(f.0.join("bin")).unwrap();
        f.write("bin/tsc", "#!/bin/sh\ntouch compiler-invoked\nexit 0\n");
        fs::set_permissions(f.0.join("bin/tsc"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let scan = f.run(&["scan", "--format", "json", "--min-referrers", "1"]);
    assert!(scan.status.success());
    let json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(json["candidates"].as_array().unwrap().len(), 1);
    let plan = f.run(&[
        "plan",
        "--project",
        "tsconfig.json",
        "--out",
        "old.json",
        "--markdown",
        "old.md",
        "--default-status",
        "apply",
    ]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    f.write(
        "tsconfig.json",
        r#"{"include":["src"],"references":[{"path":"../outside"},{"path":"./missing"}]}"#,
    );
    f.write("existing.json", "sentinel json");
    f.write("existing.md", "sentinel markdown");
    let before = f.snapshot();
    for args in [
        vec!["scan"],
        vec!["scan", "--explain"],
        vec!["scan", "--format", "json"],
        vec!["plan", "--out", "new.json", "--markdown", "new.md"],
        vec![
            "plan",
            "--out",
            "existing.json",
            "--markdown",
            "existing.md",
        ],
        vec![
            "extract",
            "User",
            "--from",
            "src/model.ts",
            "--to",
            "src/new/user.ts",
        ],
        vec!["extract", "User", "--from", "src/model.ts", "--dry-run"],
        vec!["apply", "--plan", "old.json", "--accept-unreviewed"],
        vec![
            "apply",
            "--plan",
            "old.json",
            "--accept-unreviewed",
            "--dry-run",
        ],
        vec!["apply", "--plan", "old.json", "--markdown", "old.md"],
        vec![
            "apply",
            "--plan",
            "old.json",
            "--markdown",
            "old.md",
            "--dry-run",
        ],
    ] {
        rejected(f.run(&args), "project reference");
        assert_eq!(f.snapshot(), before, "mutation after {args:?}");
    }
}
#[test]
fn scan_rejects_solution_mixed_and_malformed_configs_but_accepts_empty() {
    let f = Fixture::new();
    for value in [
        r#"{"files":[],"references":[{"path":"./child"}]}"#,
        r#"{"include":["src"],"references":[{"path":"./a.json"},{"path":"./a.json"}]}"#,
    ] {
        f.write("tsconfig.json", value);
        let before = f.snapshot();
        rejected(f.run(&["scan", "--format", "json"]), "project reference");
        assert_eq!(before, f.snapshot());
    }
    for value in [r#"{"references":null}"#, r#"{"references":[{}]}"#] {
        f.write("tsconfig.json", value);
        rejected(
            f.run(&["scan", "--format", "json"]),
            "Invalid project references",
        );
    }
    f.write("tsconfig.json", r#"{"include":["src"],"references":[]}"#);
    assert!(f.run(&["scan", "--format", "json"]).status.success());
}
