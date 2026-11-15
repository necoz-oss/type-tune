# Draft: fix: prevent compiler-generated artifacts from escaping dry-run and rollback

**Priority: P1**

## Background and reproduction

At `09e4389a42cc51e460bb283d77edb1415ba9917f`, create a TypeScript 6.0.3 project with `{"compilerOptions":{"composite":true,"outDir":"dist"},"include":["src"]}`, an exported interface and two referrers. Generate a plan, approve the interface in Markdown, and run `type-tune apply --plan plan.json --markdown plan.md --dry-run`. The process exits 0 but leaves `dist/tsconfig.tsbuildinfo`. Real `tsc --noEmit --project tsconfig.json` creates the same artifact. See [compiler validation](../compiler-validation.md) and the project-reference investigation at commit `0f8b1da`.

## Expected and actual behavior

Dry-run should restore the project snapshot; failed apply should restore planned changes and not leak compiler-generated build-info files. Successful apply should retain only reviewed changes. Actual behavior left compiler-generated build info outside the tracked transaction; existing artifacts could be modified. This affects both TypeScript and Vue validation paths.

## Acceptance criteria

- [ ] Pinned real TypeScript and vue-tsc still validate rewritten sources; Vue validation follows TypeScript validation.
- [ ] New and existing build-info files remain unchanged on dry-run, failed apply and successful apply, including explicit and inherited paths.
- [ ] Temp cleanup failures fail validation; no arbitrary existing project files are removed.
- [ ] Full snapshots and actual compiler failure tests cover contents, directories and symlinks; CI executes real-compiler tests.
- [ ] Document other compiler/plugin side effects and forced-termination limitations without claiming a sandbox.

## Regression tests and related work

Test composite and incremental settings, custom and external destinations, repeated dry-runs, Vue failure after successful TypeScript validation, pre-existing build-info bytes, and reviewed partial selection. Related: [#30](https://github.com/necoz-oss/type-tune/issues/30), [PR #31](https://github.com/necoz-oss/type-tune/pull/31), project-reference design commit `0f8b1da`.
