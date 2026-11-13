# Vue script edits stay outside the TypeScript rewrite set

## Problem

Referrer discovery and rewrite planning share `Candidate`. `referrer_paths` and `referrer_strategies` are the files the TypeScript rewrite is allowed to edit. Putting a `.vue` path in that set makes `plan_rewrite` parse and patch the component as TypeScript, which rewrites template and style bytes.

## Invariant

Supported Vue script blocks are counted on `readonly_referrers` and given `vue_script_strategies`. `referrer_count` includes them. `referrer_strategies` does not. `source_role` is the classification rewrite planning consults: Vue SFCs are `ReadOnlyVue`. `reject_unsupported_rewrite` runs at the start of `plan_rewrite`, before edits are built. It rejects unresolved Vue files, Vue declaration sources, Vue paths in the writable strategy list, and Vue imports that are not direct relative or alias specifiers.

Specifier replacement runs only after the TypeScript plan succeeds. Each replacement range must sit inside an extracted script body. The original file receives those replacements, so bytes outside the quoted specifiers stay intact. `tsc` does not typecheck `.vue` files. A change to a `.vue` path is kept only when `vue-tsc --noEmit` succeeds; a missing or failing `vue-tsc` rolls the transaction back.

## When it applies

Referrer discovery and specifier updates for `<script lang="ts">`, `<script setup lang="ts">`, and the `tsx` variants. It does not authorize template or style edits, barrel or namespace Vue imports, or moving a type declared inside an SFC.

## Evidence

`crates/type-tune-core/src/source_role.rs`, `crates/type-tune-core/src/vue.rs`, `crates/type-tune-core/src/rewrite/vue_scripts.rs`, and `crates/type-tune-core/src/compiler.rs` implement the split. `scan/vue_tests.rs` and `crates/type-tune-cli/tests/vue_referrers.rs` cover counting, specifier updates, malformed files, mixed imports, and `vue-tsc` rollback. Confirmed by those tests.
