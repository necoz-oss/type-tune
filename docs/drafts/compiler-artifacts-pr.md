# Draft PR: fix: isolate compiler build-info artifacts during validation

## Problem and behavior

A reviewed dry-run with a composite project could exit successfully while leaving `dist/tsconfig.tsbuildinfo`: `tsc --noEmit` still writes incremental build information. The same leak could occur on failed apply and Vue validation. Type-tune now directs each compiler invocation to a fresh private temporary build-info path, sets compiler scratch-directory variables to that private directory, and explicitly removes that directory after the process exits. Existing project build-info is left untouched, including on successful apply. Validation and the order of `tsc` then `vue-tsc` are retained.

The pinned compilers reject disabling incremental compilation for composite projects. Redirecting the artifact avoids predicting and deleting user paths and forces fresh validation. Full project-copy or OS sandboxing is deferred because it changes path/dependency behavior and is not required to address this confirmed artifact. This is not a sandbox for arbitrary compiler executables or plugins; abnormal process termination, concurrent changes and non-build-info compiler side effects remain limitations. Older compilers that reject the override fail validation rather than being skipped.

## Verification

- Before: real TypeScript 6.0.3 regression test failed because the successful reviewed dry-run added `dist/tsconfig.tsbuildinfo`.
- After: real pinned `tsc` and `vue-tsc` tests compare filesystem snapshots for new/existing/default/custom/inherited/external/symlinked build-info paths, repeated previews, failed TypeScript and Vue validation, and successful apply.
- CI's existing `pnpm test:vue-tsc` equivalent runs the real-compiler tests; `pnpm check` covers Rust and compatibility fixtures.
- Public Vue 3 RealWorld: native baseline typecheck/build, reviewed dry-run and apply, full clone snapshot comparison after dry-run, and native post-apply typecheck/build passed. Dependencies were a symlink to an existing disposable copy and were not traversed in the snapshot.


## Legacy compiler correction

The original unconditional override caused TS5069 on valid non-incremental TypeScript 4.9.5 and 5.5.4 projects. The selected compiler now resolves effective options with `--showConfig` before validation; only incremental or composite configurations receive the isolated build-info path. Failed or malformed inspection fails closed. Real pinned TypeScript 4.9.5, 5.5.4, 5.8.3 and 6.0.3 regression tests run through `pnpm test:compiler-versions` after `npm ci --ignore-scripts --prefix tests/compiler-versions`; CI installs and runs them explicitly. Existing Vue compiler tests remain in place.
