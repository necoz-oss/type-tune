# Compiler validation and build-info artifacts

`apply`, `extract`, and their dry runs validate rewritten TypeScript with the project-local `tsc` where available, falling back to `PATH`. If Vue scripts change, they also run `vue-tsc` after `tsc`. Before each validation, the selected compiler runs `--showConfig --project` to resolve effective options, including package-based and ordinary `extends`. When `incremental` or `composite` is true, validation receives `--tsBuildInfoFile` pointing to a fresh private path. A composite configuration is isolated even when it explicitly disables incremental compilation: TypeScript can write build info before reporting that invalid combination. When both options are absent or false, the override is omitted so older non-incremental compilers retain their native behavior. Type-tune does not enable incremental compilation or modify the user's config.

Compiler `TMPDIR`, `TMP`, and `TEMP` point to a private directory for both inspection and validation. The directory is removed after each compiler exits, including on failure; cleanup errors are reported and cause the rewrite to fail or roll back. If the compiler cannot provide valid `--showConfig` output, type-tune fails closed with a diagnostic rather than guessing or retrying without isolation. Each TypeScript or Vue check starts an additional compiler process for inspection; the cost depends on project and compiler startup time.

For incremental and composite validation, existing project build-info files are neither read nor updated. Successful apply keeps approved source changes, not validation build-info files; users can run their normal build to refresh caches. Dry-run rolls back the planned edits, and failed apply rolls them back as before. TypeScript 4.9.5, 5.5.4, 5.8.3 and 6.0.3 were tested; current pinned vue-tsc 3.3.14 was tested with TypeScript 6.0.3. In a disposable Vue 3.3.4 fixture, vue-tsc 1.8.27 with TypeScript 4.9.5 also passed native validation and reviewed dry-runs with incremental both enabled and disabled; snapshots matched. Legacy Vue apply and other legacy combinations were not tested. This is not a claim of universal compiler compatibility: wrappers must faithfully implement `--showConfig` and compiler options, and compilers that cannot be inspected fail closed. Compiler and configuration must not change concurrently between inspection and validation.

The build-info destination override can change diagnostics specifically about its original location or writability; it does not validate that destination. Other compiler errors are retained. Private scratch handling is not a general filesystem sandbox: compiler executables, Vue plugins, explicitly configured caches, and other options may have other side effects. Forced termination and concurrent filesystem changes can prevent cleanup. Filesystem snapshots cover contents, paths, directory presence and symlink targets, but not timestamps, ACLs or other platform metadata. Cross-platform behavior beyond Linux has not been established.

## Compiler behavior observed

For each row, direct native `--noEmit -p` and the same command with `--tsBuildInfoFile` were run against each listed compiler. Inherited cases use `extends`; all builds were in disposable directories. “Artifact” means a build-info file, either at the configured/default path or at the override. The separate end-to-end tests cover a subset of these combinations; this table is not an apply-compatibility claim for every row.

| Effective configuration | TypeScript 4.9.5 and 5.5.4 | TypeScript 5.8.3 and 6.0.3 |
| --- | --- | --- |
| Incremental absent or false; inherited true overridden false | Native exit 0, no artifact; adding override fails TS5069. | Both exit 0, no artifact. |
| Incremental true, composite true, or either inherited | Both exit 0 and create build info at their respective output paths. | Same. |
| Incremental true with custom build-info path | Both exit 0; override redirects the artifact. | Same. |
| Composite true and incremental false | Both exit 2 with TS6379 **and create build info**; override redirects it. | Same. |
| Custom build-info path without incremental/composite | Both exit 2 with TS5069 and no artifact. | Both exit 0 with no artifact. |

The non-incremental regression in the original PR is reproduced by TypeScript 5.5.4: native validation and pre-PR type-tune passed, but the unconditional override failed with TS5069. The [TypeScript 5.5.4 compiler](https://github.com/microsoft/TypeScript/blob/v5.5.4/src/compiler/program.ts#L4244-L4253) checks whether the build-info option is allowed. Using `--showConfig` delegates effective-option inheritance to the compiler rather than approximating it in type-tune's partial config loader. Raw JSON inspection can miss inherited settings; a diagnostic-based retry without the override is unsafe because compiler errors alone do not prove that no artifact can be written. Separate version-specific capability logic is unnecessary for the tested configurations.

## Reproduction and regression

The prior implementation could leak build info even with `--noEmit`. For example, in a disposable project with `{"compilerOptions":{"composite":true,"outDir":"dist"},"include":["src"]}`, an exported interface and two TypeScript referrers, generate and review a plan and run:

```sh
type-tune plan --project tsconfig.json --out plan.json --markdown plan.md
# Approve the candidate in plan.md.
type-tune apply --plan plan.json --markdown plan.md --dry-run
```

On the pre-fix `09e4389a42cc51e460bb283d77edb1415ba9917f` binary this exited 0 and left `dist/tsconfig.tsbuildinfo`; the same reviewed dry-run with the fix exits 0 without that project change. In a separate incremental fixture with a TypeScript error, direct `tsc --noEmit --project tsconfig.json` exited 2 with TS2322 and still created its configured `cache/result.tsbuildinfo`. `pnpm test:vue-tsc` runs the real pinned Vue compiler integration tests; `npm ci --ignore-scripts --prefix tests/compiler-versions` followed by `pnpm test:compiler-versions` runs the pinned legacy compiler tests. These include snapshots for new and existing build info, explicit and inherited destinations, failure rollback, Vue validation and repeated previews. Project `node_modules` is a fixture dependency symlink and is excluded from snapshot traversal; other symlink targets are recorded.

Disabling incremental mode is not an equivalent fix: the pinned compilers reject `--incremental false` with `composite` (TS6379). Tracking presumed artifact paths risks missing inherited or external destinations and deleting user-owned files; validation in a full copied project or OS sandbox would be a larger separate design with different path and dependency semantics. Redirection addresses the confirmed build-info output without claiming arbitrary compiler isolation. The project-reference design investigated in commit `0f8b1da` remains future work; this change does not implement graph traversal or graph-wide validation.
