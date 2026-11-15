# type-tune architecture

This document describes the active Rust implementation. For installation and an end-to-end example,
start with the [README](../README.md). For the npm delivery boundary, see
[Rust CLI and npm distribution](rust-migration.md).

## Product boundary

type-tune is a refactoring assistant for TypeScript projects. It discovers exported `type` aliases
and `interface` declarations that are shared outside their declaring file, proposes destinations,
and applies only reviewed moves. It is intentionally narrower than a general-purpose codemod:
predictable plans and safe import updates take precedence over broad syntax coverage.

The supported workflow is:

```text
scan → plan → human review → apply
                         ↘ dry-run
```

`extract` uses the same planning, rewrite, validation, and rollback path for one explicitly named
symbol.

## Implementation boundaries

```text
crates/type-tune-cli       Command parsing and user-facing output
             │
             ▼
crates/type-tune-core      Scan, plans, review state, rewriting, validation, AI annotations
             │
             ├─ TypeScript project files and tsconfig.json
             └─ project-local TypeScript compiler for final validation

npm launcher               Resolve platform package → verify checksum → execute Rust binary
```

The implementation is Rust-only. The small JavaScript launcher in `bin/type-tune.mjs` is an npm
distribution adapter, not an alternate CLI. The Astro documentation site retains its own TypeScript
tooling and is outside this architecture.

## Commands and data flow

### `scan`

`scan --project <tsconfig>` loads the project configuration, discovers TypeScript sources, parses
them with tree-sitter, and resolves type-only relationships using the project configuration. It
emits candidates as text or JSON. `--explain` also reports exported declarations intentionally
omitted from the result. Included `.vue` files are not parsed as TypeScript files. A dedicated
extractor reads supported script blocks, preserves their original byte offsets, and feeds only
that TypeScript into the existing import and usage analysis. Those referrers stay out of the
writable TypeScript edit pipeline. A later step may replace module specifiers inside the extracted
script ranges. Template and style bytes are not edited.

Candidates are limited to exported type aliases and interfaces. Generated files and declarations
already in a `types` directory are excluded by default; callers can opt in with
`--include-generated` and `--include-existing-types`.

### `plan` and `review`

`plan` applies the requested eligibility and placement options, then writes a machine-readable JSON
plan and a human-editable Markdown checklist. The default review status is `ignore`, so generating
a plan never authorizes changes. `review` updates that Markdown checklist interactively.

The planner records all material decisions with every entry: its source and destination, local type
dependencies, referrer paths, selected import strategies, and a source fingerprint used to reject a
stale plan later.

### `apply` and `extract`

`apply` merges the plan with its reviewed Markdown state, rejects stale or conflicting entries,
and executes a transaction. It moves the declaration and its safe local type dependencies, updates
relative, alias, or simple barrel imports, and preserves compatibility re-exports where needed.
It then runs the project-local TypeScript compiler. When the rewrite changes a `.vue` file, it also
runs `vue-tsc` because `tsc` does not typecheck single-file components. A missing `vue-tsc`, a
compiler failure, or an operational error rolls the transaction back. `--dry-run` follows that same
path and always rolls the changes back.

`extract <symbol> --from <path>` builds a one-symbol operation with the same safeguards. It is
useful when a declaration does not meet the scan's referrer threshold but a developer has already
identified the move they want.

## Determinism and safety

Source paths are normalized to project-relative POSIX form before storage, comparison, hashing, or
output. Candidates are sorted by a stable key made from source path, symbol name, and declaration
kind. The candidate ID is the first 12 hexadecimal characters of that key's SHA-256 digest.
Concrete declarations receive a full SHA-256 declaration key that includes their start position;
that key drives rewrite deduplication and conflict detection.

The safety contract is deliberately conservative:

- no source changes occur during `scan` or `plan`;
- plan entries must be explicitly reviewed before normal `apply`;
- unknown, stale, duplicate, and destination-conflicting entries are rejected;
- file paths must remain inside the chosen project;
- the final TypeScript compiler validation runs before a transaction is retained;
- dry runs leave the working tree unchanged;
- a Vue referrer is updated only by replacing a module specifier inside a supported script
  block, and that change is kept only when both `tsc` and `vue-tsc` succeed;
- a move that depends on a Vue file the scan could not analyze, or on a Vue import that is not a
  direct specifier update, is rejected inside rewrite planning before any edit is applied. An
  unresolved Vue file blocks every candidate from that scan. The scan JSON field is
  `analysisLimitations`. Direct namespace and default Vue imports are blockers, not supported
  script strategies. Ignored plan entries stay in freshness and metadata checks, and their Vue
  files are omitted from the rewrite source map so they cannot block a separate approved move.

This is a source-aware refactoring tool, not a promise that every TypeScript construct can be moved
automatically. Unsupported declaration shapes are excluded rather than guessed at.

## Import and destination strategy

The default placement style is `collocation`, which writes a `types/` file near the source. The
`repo-types` style places the equivalent structure beneath repository-level `types/`. Referrer
imports retain the existing style when possible:

- an existing relative import becomes a relative import to the new type file;
- an existing configured path alias remains an alias;
- a simple barrel import remains a barrel import and its export is updated.

Same-file type dependencies move with the selected declaration when they are safe to move. Runtime
values and unsupported declaration forms are not silently copied into the destination.

## AI review annotations

AI review is optional and begins only with `plan --ai-review`. It can advise `apply`, `ignore`, or
`needs-review`; it never edits source code or replaces the deterministic scan and rewrite logic.
Provider responses are validated against known candidate IDs and the expected response schema before
they are attached to plan entries. See [AI review](ai-review-design.md) for provider, context, and
error-handling details.

## Verification

The repository's primary implementation check is:

```bash
pnpm check
```

It checks Rust formatting, runs Clippy with warnings denied, executes the Rust tests, and produces a
release binary. `pnpm package:check` validates npm packaging, while `pnpm publish:check` adds native
package version validation. The documentation site has independent `pnpm docs:check` and
`pnpm docs:build` checks.
