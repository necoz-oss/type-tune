# Vue referrers stay outside the writable rewrite set

## Problem

Referrer discovery and rewrite planning share `Candidate`. `referrer_paths` and `referrer_strategies` are the files the TypeScript rewrite is allowed to edit. Putting a `.vue` path in that set makes `plan_rewrite` parse and patch the component as TypeScript.

## Invariant

Supported Vue script blocks are counted, then stored on `readonly_referrers`. `referrer_count` includes them. `referrer_strategies` does not. `source_role` is the only classification rewrite planning consults: Vue SFCs are `ReadOnlyVue`, and `reject_unsupported_rewrite` runs at the start of `plan_rewrite`, before edits are built. The same gate rejects a strategy or barrel path whose role is not writable, so a forged `.vue` strategy cannot skip the read-only list.

An SFC that cannot be parsed reliably becomes an `analysis_limitations` entry on every candidate from that scan. It is not recorded as zero referrers. Template and style text never enter the TypeScript parser. Script bytes stay at their original offsets by embedding them in a same-length buffer.

## When it applies

Phase 1 referrer discovery for `<script lang="ts">`, `<script setup lang="ts">`, and the `tsx` variants. It does not authorize Vue import edits or `vue-tsc` validation.

## Evidence

`crates/type-tune-core/src/source_role.rs`, `crates/type-tune-core/src/vue.rs`, and `crates/type-tune-core/src/rewrite/planning.rs` implement the split. `scan/vue_tests.rs` and `crates/type-tune-cli/tests/vue_referrers.rs` cover counting, malformed files, and apply/dry-run rejection without file changes. Confirmed by those tests. Vue rewriting remains unsupported.
