# Vue script edits stay outside the TypeScript rewrite set

## Problem

Referrer discovery and rewrite planning share `Candidate`. `referrer_paths` and `referrer_strategies` are the files the TypeScript rewrite is allowed to edit. Putting a `.vue` path in that set makes `plan_rewrite` parse and patch the component as TypeScript, which rewrites template and style bytes.

## Invariant

Supported Vue script blocks are counted on `readonly_referrers` and given `vue_script_strategies`. `referrer_count` includes them. `referrer_strategies` does not. `source_role` is the classification rewrite planning consults: Vue SFCs are `ReadOnlyVue`. `reject_unsupported_rewrite` runs at the start of `plan_rewrite`, before edits are built. It rejects unresolved Vue files, Vue declaration sources, Vue paths in the writable strategy list, and Vue imports that are not direct relative or alias specifiers.

An SFC that cannot be parsed reliably becomes an `analysisLimitations` entry on every candidate and on the scan result. It is not recorded as zero referrers. One unresolved file therefore rejects every move from that scan; narrowing the gate is future work. The message names the parse failure and tells the caller to correct the script or exclude the component and regenerate the plan. Scan JSON omits the field when empty and still accepts the older `analysis_limitations` name. `scan` and `scan --explain` report the same limitations. Template and style text never enter the TypeScript parser. Script bytes stay at their original offsets by embedding them in a same-length buffer.

`apply` validates fingerprints and metadata for every plan entry, then builds a second source map from the approved entries only. A Vue file that belongs solely to an ignored entry is not a rewrite input. `plan_rewrite` still rejects a Vue path that is present but is not a script-strategy target of the candidates it was given. A direct `import * as` or default Vue import stays a referrer and is recorded in `vueRewriteBlockers` instead of `vue_script_strategies`. A named import in the same file does not override that blocker. TypeScript namespace imports stay on `referrer_strategies`.

Specifier replacement runs only after the TypeScript plan succeeds. Each replacement range must sit inside an extracted script body. The original file receives those replacements, so bytes outside the quoted specifiers stay intact. `tsc` does not typecheck `.vue` files. A change to a `.vue` path is kept only when `vue-tsc --noEmit` succeeds; a missing or failing `vue-tsc` rolls the transaction back. A TypeScript-only change does not run `vue-tsc`.

## When it applies

Referrer discovery and specifier updates for `<script lang="ts">`, `<script setup lang="ts">`, and the `tsx` variants. It does not authorize template or style edits, barrel or namespace Vue imports, or moving a type declared inside an SFC.

## Evidence

`crates/type-tune-core/src/source_role.rs`, `crates/type-tune-core/src/vue.rs`, `crates/type-tune-core/src/rewrite/vue_scripts.rs`, `crates/type-tune-core/src/scan/referrers.rs`, and `crates/type-tune-cli/src/commands/apply.rs` implement the split. `scan/vue_tests.rs`, `crates/type-tune-cli/tests/vue_referrers.rs`, and `crates/type-tune-cli/tests/vue_reviewed_selection.rs` cover counting, namespace blockers, partial plan approval, malformed files, mixed imports, and `vue-tsc` rollback. Confirmed by those tests.
