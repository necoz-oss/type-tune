# type-tune

<p align="center">
  <img src="docs/public/animation/motion-logo.gif" width="400" alt="Animated type-tune logo" />
</p>

<p align="center">
  <a href="https://github.com/necoz-oss/type-tune/actions/workflows/test.yml"><img src="https://github.com/necoz-oss/type-tune/actions/workflows/test.yml/badge.svg" alt="Test" /></a>
  <a href="https://github.com/necoz-oss/type-tune/actions/workflows/docs.yml"><img src="https://github.com/necoz-oss/type-tune/actions/workflows/docs.yml/badge.svg" alt="Docs" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License" /></a>
</p>

<h3 align="center">Give shared TypeScript types a better home. 😸</h3>
<p align="center">Scan. Review. Move. Validate.</p>

**type-tune** is a Rust-powered CLI that finds shared TypeScript `type` aliases and `interface` declarations and helps move them without manually chasing imports.

It creates a reviewable plan, updates supported type imports and re-exports, and validates the rewrite with TypeScript. **You decide what moves and where.**

Guides for installation, the first scan, and the review workflow live in the [official documentation](https://type-tune-nine.vercel.app/). Releases are listed on [GitHub Releases](https://github.com/necoz-oss/type-tune/releases), and the package is [`type-tune` on npm](https://www.npmjs.com/package/type-tune).

## Why type-tune?

Types often start next to a component or utility, then end up shared across a project. Moving them later means tracking imports, local dependencies, and barrels by hand.

- **Discover** shared types with deterministic candidate IDs.
- **Review** `plan.md` and choose the destination before making changes.
- **Rewrite** supported type-only imports, aliases, and simple barrels while keeping compatibility re-exports at the original module path.
- **Validate** with `tsc`, with rollback after normal validation failures and dry-run previews.
- **Optionally** add AI review notes without letting AI control the rewrite.

Unlike [Knip](https://knip.dev/), which finds unused code, type-tune focuses on **moving types that are still in use**. The tools are complementary.

<p align="center">
  <img src="docs/public/animation/conveyor-belt.gif" width="540" alt="Animated type-move workflow" />
</p>

## Installation

Install the published npm package in a project, or run a one-off command. Both need **Node.js 22+**. The launcher selects a platform binary for macOS arm64/x64, Linux x64 (glibc or musl), or Windows x64.

Add it as a development dependency:

```bash
pnpm add -D type-tune
# npm install -D type-tune
# yarn add -D type-tune
# bun add -d type-tune
```

Then run the installed CLI. `pnpm exec` and `npx` use the local install when the package is already in the project:

```bash
pnpm exec type-tune scan --project tsconfig.json
# npx type-tune scan --project tsconfig.json
```

Without a local install, `npx type-tune scan --project tsconfig.json` downloads the package for that invocation. The project should have TypeScript available: dry-run and apply prefer `node_modules/.bin/tsc`, then `tsc` on `PATH`.

## Quick Start

Start from a clean Git working tree so the rewrite diff stays easy to review. The commands below use `pnpm exec` after `pnpm add -D type-tune`. After `npm install -D type-tune`, use `npx type-tune` with the same arguments.

```bash
# 1. Find candidates
pnpm exec type-tune scan --project tsconfig.json

# 2. Generate a reviewable plan
pnpm exec type-tune plan --project tsconfig.json \
  --out .type-tune/plan.json \
  --markdown .type-tune/plan.md

# 3. Review the plan (or edit plan.md manually)
pnpm exec type-tune review --markdown .type-tune/plan.md

# 4. Preview, then apply
pnpm exec type-tune apply --plan .type-tune/plan.json \
  --markdown .type-tune/plan.md --dry-run
pnpm exec type-tune apply --plan .type-tune/plan.json \
  --markdown .type-tune/plan.md
```

Candidates begin as `[ignore]`. Mark only the ones you want as `[apply]`, and keep every candidate in the file. You can change the `to:` path in `plan.md` to a **new, project-relative destination**:

```md
- [apply] <candidate-id> `OrderSummary`
  - kind: `typeAlias`
  - from: `src/features/order/schema.ts`
  - to: `src/domains/orders/types/order-summary.ts`
```

`plan.md` is the file you edit. `plan.json` keeps candidate metadata, source fingerprints, and the options used to build the plan. Apply reads approval and destination changes from the Markdown, then checks them against `plan.json`. Do not edit that metadata in `plan.json`.

A `to:` path must differ from the source, use a normalized project-relative path, and end in `.ts`, `.tsx`, `.mts`, or `.cts`. It cannot be a declaration file or a file that already exists. Approved candidates from the same source file must share one destination. `review` needs an interactive terminal; editing `plan.md` directly works anywhere.

## A small example

Before, a type lives beside a feature:

```ts
// src/features/order/OrderPage.tsx
export type OrderSummary = {
  id: string;
  total: number;
};
```

After reviewing the move, the type can live in its own file:

```ts
// src/features/order/types/order.ts
export type OrderSummary = {
  id: string;
  total: number;
};
```

The original module keeps a type-only compatibility re-export, and supported TypeScript referrers are updated:

```ts
// src/features/order/OrderPage.tsx
export type { OrderSummary } from "./types/order";
```

That helps preserve imports through the original module, including unscanned consumers, but does not replace their own typechecks or builds.

## Scope & safety

- **TypeScript first:** scan and rewrite still edit only `.ts` and `.tsx` files from the selected `tsconfig.json`. Supported Vue single-file components can contribute to **read-only** referrer counts when their TypeScript lives in `<script lang="ts">`, `<script setup lang="ts">`, `<script lang="tsx">`, or `<script setup lang="tsx">`. Templates, styles, and other SFC content are not parsed as TypeScript and are never rewritten. `.vue` is not fully supported.
- **Vue moves are rejected:** if a move would have to update one of those Vue referrers, or a selected `.vue` file cannot be analyzed, `apply` and `apply --dry-run` stop before changing files. Vue import rewriting and framework-specific validation such as `vue-tsc` are future work. Svelte, Astro, JavaScript, and external consumers are still neither counted nor rewritten.
- **Validation:** apply and dry-run run `tsc --noEmit --project` only after a rewrite plan is accepted. That check is not Vue-aware and does not replace a framework build or consumer tests. After a successful apply, review the diff and run your formatter, tests, and framework build.
- **Safe review:** plans are revalidated against source changes and destination restrictions. A normal validation failure rolls the rewrite back. Dry-run writes the same files, validates them, and restores them when that process finishes normally. Forced termination and concurrent edits are outside that guarantee.
- **Limited moves:** exported type aliases and interfaces only; `enum`, `class`, default exports, namespace merges, and advanced value-derived types are outside the current scope.
- **Conservative formatting:** local conventions are preserved where safely possible, but formatter-clean output is not guaranteed. Review the diff and run your formatter.
- **Explicit scope:** existing `types/` directories and `types.ts` modules are excluded by default; monorepo traversal is not automatic.

For supported options and more detailed limitations, see [Requirements](docs/requirements.md).

## More commands

```bash
# Understand why a declaration is excluded
pnpm exec type-tune scan --project tsconfig.json --explain

# Include declarations already in types locations
pnpm exec type-tune scan --project tsconfig.json --include-existing-types

# Move one declaration directly (with preview)
pnpm exec type-tune extract OrderSummary \
  --from src/features/order/OrderPage.tsx \
  --to src/features/order/types/order.ts \
  --project tsconfig.json --dry-run

# Preview optional AI review data before sending it to a provider
pnpm exec type-tune plan --project tsconfig.json --ai-review --ai-dry-run
```

AI review is off unless you pass `--ai-review`. It only annotates the plan. `--ai-dry-run` writes the request and does not contact a provider. A real review can send source context to OpenAI, an OpenAI-compatible endpoint, Anthropic, or a configured Ollama server. The default `mock` provider reads a local `--ai-response-file` instead. See [AI review design](docs/ai-review-design.md) before enabling it.

## Development from source

Building in this repository is separate from installing the npm package:

```bash
pnpm install
pnpm build
./target/release/type-tune --help
```

Run the main checks with `pnpm check`, and run the documentation checks with `pnpm docs:check && pnpm docs:build`.

## Documentation

- [Official documentation](https://type-tune-nine.vercel.app/) · [npm package](https://www.npmjs.com/package/type-tune) · [GitHub Releases](https://github.com/necoz-oss/type-tune/releases)
- [Architecture](docs/design.md) · [Requirements](docs/requirements.md)
- [AI review](docs/ai-review-design.md) · [Release guide](RELEASING.md)
- [Distribution](docs/rust-migration.md)

<p align="center">
  <img src="docs/public/animation/eat-fish.gif" width="320" alt="Cat eating a fish animation" />
</p>

<p align="center"><strong>Small. Simple. Useful. Built Together! 🐈</strong></p>
