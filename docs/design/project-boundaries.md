# Project boundary investigation

Status: selected source symlinks are rejected during discovery; explicit boundary selection and
external-import completeness are not implemented. Historical evidence below was collected
at `b9f784747dbc0b5b0cd8091fabae213c55dfec92` with the existing debug binary and TypeScript
6.0.3. This is not a claim of complete TypeScript module-resolution parity or a filesystem sandbox.

## Current responsibilities

The canonical selected tsconfig identifies the configuration. The loader retains the declaring
directory for inherited `files`, `include`, `exclude`, `baseUrl`, `paths`, and `rootDirs` before
converting them to root-relative values. The current `project_root` is the common ancestor of the
selected configuration directory and the directories declaring the effective `files` and `include`
settings. A relative base can therefore widen it; a package base can also do so if it supplies one
of those effective settings. Other inherited directories do not themselves determine this root.
Configured paths outside the inferred root are rejected, rather than expanding it based on their
contents. This default is compatibility behavior, not independent caller authorization.

That same root is also the source-discovery start, base for POSIX candidate and referrer paths and
IDs, limit for package scopes, destination and plan validation, transaction root, compiler lookup
starting point and compiler working directory. The selected tsconfig path is passed separately to
the compiler. A root change can consequently alter package resolution and compiler selection as
well as the meaning of persisted paths. `projectPath` in a v1 plan does not encode a workspace
boundary; apply reloads it and rescans, so a plan-supplied boundary must not become write authority.

## Evidence and blocker

In a disposable project containing `.generated/tsconfig.app.json` with
`{"include":["../src/**/*.ts"]}` and `src/model.ts`, `src/first.ts`, `src/second.ts`,
`tsc --noEmit -p .generated/tsconfig.app.json --listFilesOnly` exited 0 and listed all three
sources. `type-tune scan --project .generated/tsconfig.app.json --format json --min-referrers 1`
exited 1: `tsconfig path "../src/**/*.ts" resolves outside native project root .../.generated.`
The equivalent root config scanned successfully, returning `src/model.ts#Shared#interface` with
two referrers. Neither config contained references.

There are two independent completeness and authorization problems which a root flag alone cannot
fix:

1. Previously, source discovery followed directory symlinks without checking that their canonical targets remain
   inside the root. A fixture with `workspace/linked` pointing to an external directory and
   `include: ["linked/**/*.ts"]` produced a successful scan with the external declaration as
   `linked/model.ts`. This demonstrates an external **read and candidate**, not an external write;
   write-path checks and transactions have separate protections. Discovery now rejects relevant
   directory and source-file symlinks before enumeration or source reads. Internal aliases are
   rejected as ambiguous; broken relevant links are errors. Excluded and unrelated links are skipped.
   Paths remain logical, project-relative identities; canonical paths are used to diagnose and check
   containment. Dependency metadata and compiler reads have their existing separate behavior.
2. Before the incremental Phase 1-B checks, import expansion resolved only against the already discovered in-root universe. In a fixture
   where `src/main.ts` imports `../../external/model`, TypeScript listed the external source and
   exited 0; type-tune exited 0 with zero candidates and no boundary diagnostic. A correct boundary
   contract must distinguish such project sources from legitimate external dependency metadata and
   declarations rather than silently losing program members. The same omission occurred when
   `files` selects only `src/main.ts` and its import resolves through an otherwise unselected
   `src/linked` symlink to an external source: discovery does not traverse that link, but the scan
   still exits successfully with no candidates while TypeScript lists the linked source. This
   incremental checks below now reject these direct and transitive cases, but other omissions remain.

The native nuxt.com checkout at `71a7374d88f3747658eebb7e2384fc593aa706cc` also reproduced the
original generated-config rejection: scanning its unmodified `.nuxt/tsconfig.app.json` exited 1
on `../node_modules`. Scanning its root exited 1 with the existing four-reference diagnostic.
This does not establish generated-config compatibility. Historical preparation and typecheck
evidence is in [the compatibility report](https://github.com/necoz-oss/type-tune/blob/e6a2290/docs/vue-nuxt-compatibility.md).
The historical Vue RealWorld result used `tsconfig.json` at
`741c215ef0f674f90fcb03c5493a1b3a3a7f1b03`, which declares two references; its earlier success did
not establish complete single-config coverage and the current guard rejects that root.

## Proposed contract and remaining design work

### Incremental Phase 1-B contract

The existing inferred root remains the boundary. For statically recorded relative imports,
side-effect imports, re-exports, and configured path aliases, a source-file candidate resolved by
the supported extension/index rules must be
inside that root and must not use a symlink in its logical source path. A discovered in-root
source remains part of the existing import expansion; a resolved source outside that discovered
universe must fail rather than disappear. Candidate checks inspect filesystem metadata and
canonical paths before reading source contents. Ordinary unresolved specifiers are distinct from
a resolved but unsupported project source. Dependency declarations and metadata keep their
existing handling; their presence under `node_modules` is not proof that an arbitrary TypeScript
source is authorized.

This incremental contract does **not** establish Phase 1-B completeness: bare package resolution,
runtime dynamic imports, declarations and TypeScript's
full resolution precedence need separate investigation and enforcement. In particular, a
successful scan cannot yet be treated as a complete account of every TypeScript program member.
No general filesystem sandbox or protection against concurrent replacement is implied.

Before extending this check, a package fixture with a pnpm-style `node_modules` symlink and
`package.json` containing `"types":"index.ts"` showed that both the old scanner and this
incremental relative/alias check return success with no candidates, while TypeScript resolves
the external `.ts` source. Package metadata and `.d.ts` declarations are distinct from source
files; the presence of a `node_modules` path or a bare name is not an authorization decision.
The incremental check inspects the nearest package's explicit `types`/`typings` target, a single
unambiguous nested `types` target in exact or wildcard package exports, and matching
`typesVersions` alternatives that point at existing sources. It applies the same source identity
checks while treating declaration targets as dependency inputs. Multiple conditional targets,
other `typesVersions` shapes, `rootDirs` fallback targets and declaration imports remain outside
that narrow check and prevent a complete Phase 1-B verdict. An export `types` target can be
ambiguous when a TypeScript resolution mode ignores exports; the check may conservatively reject
such a source rather than establish which mode selected it.

This is a confirmed remaining defect, not merely a theoretical limitation: in a disposable
fixture, `external-pkg` exposed `index.d.ts`, which re-exported `./hidden-source` from an
external `hidden-source.ts`. TypeScript 6.0.3 included both files and exited successfully; the
incremental scanner also exited successfully while omitting `hidden-source.ts`. The scanner does
not follow declaration imports, so even a successful scan of a pnpm layout is not proof of
complete dependency analysis. No external write was demonstrated. Phase 1-C must not be built on
an assumption that this incomplete scan is a safe authorization boundary.

Prefer an optional `--workspace-root` and corresponding core API argument, canonicalized from the
caller-supplied path. Reject a missing or non-directory root and a selected configuration outside
it. The boundary is an authorization limit and path representation base, not an include glob;
each configuration option still resolves against its declaring config directory. Keep compiler
lookup and working directory tied to the prior config-derived context unless explicitly redesigned.
Do not infer wider authority from configured paths, discovered sources, Git roots or package roots.
The legacy inferred default needs an explicit compatibility/security decision, because relative or
package inheritance can currently expand it.

Before implementing explicit boundary support, discovery needs a fail-closed way to identify
out-of-bound project imports without prohibiting normal package/config dependency reads. This needs a bounded resolution
policy and tests against TypeScript, including symlinks, declaration inputs and package managers;
the current in-root-only resolver cannot establish completeness by itself. Reject unsupported cases
rather than treating missing resolution as proof that no external source exists. Preserve the
non-empty references guard.

Plan/apply support additionally needs a versioned or otherwise unambiguous boundary identity,
explicit matching caller authority on apply, early mismatch rejection, and consistent rescan,
source, destination, barrel and transaction checks. Old plans must retain their existing semantics
or fail with a migration diagnostic; no value read from a plan may expand authority. Scan-only
support remains a possible intermediate step once the discovery requirements above are met.
Canonical checks do not defend against arbitrary concurrent filesystem replacement, and compiler
artifact isolation does not sandbox arbitrary compiler or plugin behavior.
