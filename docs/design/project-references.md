# TypeScript project references: current boundary and roadmap

Phase 0-B is implemented: type-tune rejects a selected tsconfig with a non-empty `references`
array. Its analysis uses one effective configuration and cannot establish source or referrer coverage
across referenced projects. Returning a partial scan as complete could produce unsafe plans. The
configuration loader checks the selected JSONC without opening reference paths; malformed reference
arrays or entries are errors. References in an extended base are not inherited, as in TypeScript.
Absent and empty arrays retain single-config behavior.

`scan` (including explain and JSON), `plan`, `extract`, and `apply` (including dry-run) exit 1 and
report the error on stderr. No plan output or rewrite is produced for this error. Apply reloads the
selected configuration even for existing plans, so adding references invalidates their use. JSON scan
stdout remains empty on error. Selecting a reference-free child configuration can be useful only if
it is independently analyzable; it does not establish workspace-wide referrer coverage.

Native Nuxt solution configs commonly contain references and are therefore rejected. Generated
configs may also include paths outside type-tune's inferred project root; this separate boundary
limitation remains unresolved. Do not treat a generated child config as a guaranteed workaround.

The Phase 1 design must define an explicitly authorized workspace boundary, configuration-aware
source ownership and resolution, deterministic graph discovery, diagnostics, and graph freshness.
Graph-wide apply additionally needs cross-project referrer coverage and validation of every affected
context. No graph traversal or graph-wide scan or apply is implemented. Historical investigation and
reproduction are available in [design commit 0f8b1da](https://github.com/necoz-oss/type-tune/blob/0f8b1da/docs/design/project-references.md)
and [compatibility report e6a2290](https://github.com/necoz-oss/type-tune/blob/e6a2290/docs/vue-nuxt-compatibility.md).
