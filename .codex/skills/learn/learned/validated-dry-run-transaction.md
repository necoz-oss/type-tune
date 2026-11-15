# Validated dry-run transaction

A dry-run must validate the temporary rewritten project, not just the original state, and then
attempt rollback whether validation succeeds or fails. Report rollback failures; do not print a
successful preview when validation or restoration fails. Normal apply uses the same compiler gate
and rolls back a validation failure.

For regression tests, use a project that compiles before the move but fails after the staged rewrite.
Check the diagnostic and non-zero exit, absence of a success summary, and restoration of affected
files and transaction-created empty directories. Preserve permissions and unrelated files.

Rollback is not a guarantee against forced termination or concurrent edits. Do not claim successful
restoration when cleanup reports an error.

Relevant code: `crates/type-tune-cli/src/preview.rs`, `crates/type-tune-cli/src/commit.rs`,
`crates/type-tune-core/src/transaction.rs`, and `crates/type-tune-cli/tests/review_destination.rs`.

## Compiler build information

Real TypeScript 6.0.3 and vue-tsc can write build information even under `--noEmit`, including
on validation failure. `compiler.rs` redirects each compiler to a fresh private build-info path and
private scratch directory, reports cleanup errors, and does not alter the project cache. This
prevents the observed build-info leak but is not a sandbox for arbitrary compiler or plugin writes.
The real-compiler tests in `crates/type-tune-cli/tests/vue_tsc_apply.rs` snapshot project contents,
directories and symlinks across successful and failed validation. See `docs/compiler-validation.md`.
