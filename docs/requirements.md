# type-tune behavior and requirements

This is the current product contract for type-tune. It describes user-visible behavior; the
implementation design is in [Architecture](design.md).

## Goal

type-tune helps a developer relocate a TypeScript type that has become shared across a project. It
must make the mechanical work safer—finding referrers, updating type imports, validating the
project—without deciding on the developer's behalf that every shared type should move.

## Supported project inputs

- One TypeScript project selected with `--project <tsconfig>`.
- TypeScript and TSX source files included by that project configuration.
- Vue single-file components included by that same configuration. Supported blocks are
  `<script lang="ts">`, `<script setup lang="ts">`, `<script lang="tsx">`, and
  `<script setup lang="tsx">`. Referrers in those blocks can receive a specifier-only import
  update. Templates, styles, and other SFC content are not analyzed as TypeScript and are not
  rewritten.
- Exported `type` aliases and `interface` declarations.
- Existing relative module specifiers, configured path aliases, and simple barrel exports when their
  relationships can be resolved safely.

The project-local TypeScript compiler is used for final validation. A project should therefore have
TypeScript available in its dependencies before an apply operation. Moves that change a `.vue`
file also require `vue-tsc`, because `tsc` does not typecheck single-file components.

## Scan requirements

`scan` must be read-only. It reports candidates in deterministic order and may render a concise text
table or JSON (`--format text|json`). Its default eligibility threshold is two referrer files outside
the declaring file; `--min-referrers` changes that threshold.

By default the scanner excludes generated paths (`generated`, `__generated__`, and
`*.generated.ts[x]`) and declarations under a `types` directory or in a file named exactly
`types.ts`. These exclusions avoid churning generated output and already organized type locations.
Use `--include-generated` or `--include-existing-types` when the caller wants those sources considered.

`--include type,interface`, `--ignore-file`, and `--referrer-count-exclude` let a caller narrow the
candidate set. `scan --explain` must make omitted exported declarations understandable rather than
silently disappearing from the report.

A used type import inside a supported Vue script block counts as one referrer file, including
`defineProps<T>()` and `defineEmits<T>()` type arguments. Unused imports do not count. Relative
specifiers and configured TypeScript aliases resolve with the same rules as `.ts` files. The count
is deterministic and unique per file. These Vue paths are reported separately from writable
TypeScript strategies. A direct relative or alias type import can receive a specifier-only update.
Barrel imports, namespace imports, and statements that also bind a symbol that is not moving are
reported as blockers and are not edited.

If a selected `.vue` file cannot be parsed reliably — for example an unclosed script, a script
without `lang="ts"` or `lang="tsx"`, an external `src` script, or a script with syntax errors —
scan reports that limitation. The file is not treated as proof that nothing references the type.
The limitation is project-wide: it is attached to every candidate and also reported once at the
top level. JSON uses `analysisLimitations` and omits that field when the list is empty. Saved scan
JSON that still uses `analysis_limitations` can be read. `scan` and `scan --explain` report the
same limitations in text and JSON. One unresolved Vue file rejects every move from that scan.
Limiting the gate to candidates that file could affect is future work.

## Plan and review requirements

`plan` writes both a JSON artifact for the program and a Markdown artifact for human review. Each
entry identifies the candidate, source, destination, referrers, local dependencies, and import
strategy. It also stores a fingerprint of the source state that `apply` uses to detect stale work.

The default entry status is `ignore`. A user selects a move by marking it `apply` in the reviewed
Markdown or using `review --markdown <path>`. This plan-first requirement makes batch refactors
auditable and prevents a scan from turning into an unreviewed rewrite.

The supported placement styles are:

| Style | Destination |
| --- | --- |
| `collocation` | A `types/` file beside the source's responsibility area. |
| `repo-types` | The equivalent path under repository-level `types/`. |

## Apply requirements

Normal `apply` requires a JSON plan and its reviewed Markdown counterpart. `--accept-unreviewed` is
an explicit escape hatch; it should be reserved for controlled automation. Before changing files,
type-tune must reject malformed, stale, duplicate, or destination-conflicting entries and paths that
fall outside the selected project.

For accepted entries, the rewrite must:

- move the chosen declaration and safe same-file type dependencies;
- update type-only imports using the resolved relative, alias, or barrel strategy;
- retain type-only compatibility re-exports when required to preserve unresolved star-barrel,
  external-package, or unscanned non-TypeScript imports;
- update a supported Vue script import by replacing only its module specifier, leaving template,
  style, and every other byte unchanged;
- reject the move before writing files when an approved candidate's Vue import is not a direct
  specifier update, or when any selected Vue file could not be analyzed. Ignored plan entries are
  still validated for freshness, metadata, and project boundaries, but their Vue files are not
  rewrite inputs. An ignored candidate must not block an independent approved move;
- after `tsc` succeeds, run `vue-tsc --noEmit --project` when a `.vue` file changed. `tsc` does not
  typecheck single-file components. If `vue-tsc` is missing or reports an error, roll the
  transaction back and report that failure. A move that does not change a `.vue` file does not
  require `vue-tsc`. Dry-run uses the same validation and then restores the files. A failed
  rollback is reported rather than hidden.
- validate the resulting project with TypeScript;
- restore every touched file when validation or the transaction fails.

`apply --dry-run` has the same analysis and validation behavior but always restores the files. It is
the recommended final preview before a real apply.

## Direct extraction

`extract <symbol> --from <path>` provides the same safety path for a single declaration. It is
allowed to target a declaration that a threshold-based scan did not list. `--to`,
`--placement-style`, `--dry-run`, and `--include-generated` control its destination and scope.

## Deterministic output

All stored and printed source paths are project-relative and POSIX-normalized. Results sort by a
stable key built from source path, symbol name, and declaration kind. Candidate IDs are the first 12
hex characters of the SHA-256 hash of that key. A full declaration key additionally includes the
declaration start position and is used for rewrite deduplication and conflict detection.

Given the same project sources and command options, scan and plan output must be stable. AI review
data is an optional annotation and must not alter this deterministic identity or rewrite path.

## AI review requirements

AI review is off unless `plan --ai-review` is passed. It may recommend `apply`, `ignore`, or
`needs-review`, but it cannot write source files and does not run during `apply`. Provider output is
validated before merging into a plan. Request context is selectable and bounded; see
[AI review](ai-review-design.md) for the full privacy and failure contract.

## Distribution requirements

`npx type-tune` remains a supported entrypoint. The root npm package contains only a launcher that
chooses an optional platform package and executes its Rust binary. It must not contain an alternate
JavaScript or TypeScript CLI implementation. When a staged checksum is present, it must be verified
before execution. Distribution details and release checks are documented in
[Rust CLI and npm distribution](rust-migration.md).

## Explicit non-goals

- Automatically deciding that every shared type should move.
- Broad syntax-preserving transformation of every TypeScript declaration shape.
- Automatic cross-monorepo discovery or multi-project planning.
- Moving runtime values, classes, enums, default exports, or arbitrary declaration merges.
- Replacing a project's formatter, linter, compiler, or source-control review.
- Making AI output authoritative for source changes.
