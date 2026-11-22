# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### Fixed

- Avoid passing canonical Windows extended-length project paths to TypeScript during apply validation, addressing the TS5058 regression reported in [#36](https://github.com/necoz-oss/type-tune/issues/36).

## [v1.0.0] - 2026-11-08

### Added

- introduce type-tune 🦀 by @nyaomaru in [#1](https://github.com/necoz-oss/type-tune/pull/1)

### Fixed

- repair native package builds and checks by @nyaomaru in [#2](https://github.com/necoz-oss/type-tune/pull/2)

### Chore

- prepare first npm release checks by @nyaomaru in [#3](https://github.com/necoz-oss/type-tune/pull/3)

### 🐈 Why type-tune?

Have you ever created a type inside a component, only to find that half your project depends on it a few months later?

Maybe it started like this:

```ts
// src/features/order/OrderPage.tsx
export type OrderSummary = {
  id: string;
  total: number;
};
```

But now it's imported by API modules, utilities, other features, and tests.

You want to move it somewhere more appropriate.

Sounds simple, right?

Well... not always! 😹

You need to find the references, update imports, consider local type dependencies, preserve exports, and make sure nothing breaks.

And doing all of that manually isn't particularly fun.

**That's why I built type-tune!**

It takes care of the mechanical work while keeping the important decisions in your hands.

![type-tune workflow](https://raw.githubusercontent.com/necoz-oss/type-tune/main/docs/public/animation/conveyor-belt.gif)

### ✨ What's included in v1.0.0?

#### 🔍 Discover shared types

Find exported TypeScript types and interfaces that are referenced by multiple files.

```bash
npx type-tune scan --project tsconfig.json
```

type-tune analyzes your TypeScript project and identifies potential candidates for relocation.

You can also inspect why a declaration was excluded:

```bash
npx type-tune scan --project tsconfig.json --explain
```

No files are modified during scanning!

#### 📋 Create reviewable plans

Instead of immediately changing your code, type-tune creates a plan you can inspect.

```bash
npx type-tune plan \
  --project tsconfig.json \
  --out .type-tune/plan.json \
  --markdown .type-tune/plan.md
```

The plan includes candidate information, proposed destinations, references, and dependencies.

Every candidate starts as `ignore`.

**Nothing gets moved without your approval.**

You can edit the Markdown plan directly or use the interactive review command:

```bash
npx type-tune review --markdown .type-tune/plan.md
```

#### 🦀 Move types and update imports

Once you've approved the changes, type-tune can relocate supported declarations and update their imports.

For example, our `OrderSummary` type can move to:

```ts
// src/features/order/types/order.ts
export type OrderSummary = {
  id: string;
  total: number;
};
```

When necessary, type-tune also preserves a type-only compatibility re-export at the original location:

```ts
// src/features/order/OrderPage.tsx
export type { OrderSummary } from "./types/order";
```

It supports safely resolvable relative imports, configured TypeScript path aliases, and simple barrel exports.

And for a single declaration you already know you want to move, there's also `extract`:

```bash
npx type-tune extract OrderSummary \
  --from src/features/order/OrderPage.tsx \
  --to src/features/order/types/order.ts \
  --project tsconfig.json \
  --dry-run
```

#### 🛡️ Validate before keeping changes

Refactoring tools should be predictable.

That's why type-tune includes several safeguards:

- Deterministic candidate IDs and ordering
- Explicit human review before normal apply operations
- Detection of stale plans and conflicting destinations
- Project-boundary validation
- TypeScript compiler validation
- Transactional rollback on normal validation and execution failures
- Dry-run support for previewing changes

You can preview a planned refactor with:

```bash
npx type-tune apply \
  --plan .type-tune/plan.json \
  --markdown .type-tune/plan.md \
  --dry-run
```

Dry-run executes the actual rewrite and validation path, then restores the touched files on normal completion.

When everything looks good, remove `--dry-run` to apply the approved changes.

**Review first. Rewrite second.** 😸

#### 🤖 Optional AI review

Want another perspective on your refactoring plan?

type-tune also supports optional AI-assisted review.

```bash
npx type-tune plan \
  --project tsconfig.json \
  --ai-review \
  --ai-dry-run
```

AI review can provide suggestions, confidence scores, and potential concerns.

Supported providers include OpenAI, Anthropic, Ollama, and OpenAI-compatible endpoints, alongside a mock provider for testing.

But there's an important distinction:

**AI can review the plan. It does not control the rewrite.**

The scan, planning identities, source transformations, and validation remain deterministic.

AI review is disabled by default. You decide whether to enable it and what source context to share.

### 📦 Installation

You can install type-tune as a development dependency:

```bash
pnpm add -D type-tune
```

Or try it directly:

```bash
npx type-tune scan --project tsconfig.json
```

Other package managers such as npm, Yarn, and Bun are supported through the npm package.

#### Supported platforms

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (arm64) |
| macOS | Intel (x64) |
| Linux | x64 GNU |
| Linux | x64 musl |
| Windows | x64 MSVC |

**Requirements:** Node.js 22 or later, plus a TypeScript project. Apply-time validation uses the project's local TypeScript compiler.

The npm package uses a small launcher to execute the matching native Rust binary. It does not contain an alternative JavaScript CLI implementation.

Native packages include SHA-256 checksums that are verified by the launcher when present.

### ⚠️ Current limitations

type-tune intentionally focuses on a relatively small and predictable scope.

In v1.0.0:

- Only exported `type` aliases and `interface` declarations are eligible.
- Only `.ts` and `.tsx` source files from the selected TypeScript project are analyzed and rewritten.
- Vue, Svelte, Astro, JavaScript, and external consumers are not automatically rewritten.
- Monorepo-wide discovery across multiple TypeScript projects is not automatic.
- Classes, enums, runtime values, default exports, and several advanced declaration forms are outside the supported scope.
- TypeScript validation does not replace framework builds, linting, or other project checks.
- Dry-run rollback does not guarantee recovery from forced process termination or concurrent file edits.

The goal isn't to support every possible refactor immediately.

It's to make the supported ones **predictable, reviewable, and useful**.

### 🛠️ Built with Rust

type-tune's CLI and core implementation are written in Rust.

The project uses tree-sitter to analyze TypeScript source code, while the project's TypeScript compiler provides final validation.

The npm layer is only responsible for finding and launching the appropriate native binary.

**One Rust implementation. Multiple platforms.**

### 💙 What's next?

This is the first public release, and there's still plenty of room to improve!

I'd love to keep refining type-tune with real-world feedback, especially around TypeScript project compatibility, import handling, and the developer experience.

If you encounter a bug, find an unsupported case, or have an idea for improvement, feel free to open an issue or contribute!

- 🦀 [GitHub Repository](https://github.com/necoz-oss/type-tune)
- 📖 [Documentation](https://type-tune-nine.vercel.app/)
- 🐛 [Issues & Feature Requests](https://github.com/necoz-oss/type-tune/issues)
- 🐈 [Necoz Organization](https://github.com/necoz-oss)

### 🎉 Thank you!

type-tune started as a small idea: making it easier to organize TypeScript types without turning a simple refactor into a much bigger task.

And now it's finally becoming an open-source project!

YAY!!! 🎉

Thank you for checking it out, and I hope this little tool makes your development experience a bit easier.

Happy coding! 😸

**Small. Simple. Useful. Built Together! 🐈**

### New Contributors

* @nyaomaru made their first contribution in https://github.com/necoz-oss/type-tune/pull/1

**Full Changelog**: https://github.com/necoz-oss/type-tune/commits/v1.0.0

[v1.0.0]: https://github.com/necoz-oss/type-tune/compare/0be6eb6cf6da8aad8839b8ce5e933f6ce5e6dc7d...v1.0.0

## 1.0.0 - 2026-06-13

### Added

- Initial public release preparation.
- `scan`, `plan`, `review`, `apply`, and `extract` CLI workflows.
- Deterministic candidate IDs, reviewable plans, compile validation, and rollback.
- Optional AI review annotations for plan generation.
