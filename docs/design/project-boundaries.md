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

### Declaration-chain investigation and architecture decision

**Phase 1-B is blocked; successful scans do not establish this contract.** The existing
`expand_imported_project_files` queue contains discovered implementation sources only. Its
preflight returns as soon as it sees a declaration, so `index.d.ts` and its outgoing edges are
never queued for analysis. The ordinary resolver then searches only the implementation universe.
An external implementation behind that declaration can therefore disappear without a diagnostic.
The regression `dependency_declaration_cannot_hide_external_source` remains ignored only while
this known failure is unresolved; it must pass in the default suite before Phase 1-B is complete.

The preserved fixture selects only `src/main.ts`, which imports the bare package `external-pkg`.
That package's symlinked `package.json` selects `index.d.ts`; that declaration re-exports
`./hidden-source`, an external `hidden-source.ts`. TypeScript 6.0.3 `--listFilesOnly` exits 0
and lists all three files. Type-tune `scan --format json` exits 0 with `{"candidates":[]}`.
Code inspection establishes that its graph queue starts with `src/main.ts`, encounters the
declaration and returns from preflight, and adds neither the declaration nor the implementation
edge. The ignored regression, run explicitly, exits 101 because that successful scan should fail.
Captured fixture and raw outputs for this local investigation are in
`/tmp/type-tune-import-boundary-evidence/bare-package-source/`; that temporary directory is not
a durable project artifact.

Three approaches were investigated:

* A compiler-assisted graph could use TypeScript's program and module-resolution APIs. A
  `--listFilesOnly` result is too late: the compiler has already read sources. A disposable
  `createCompilerHost` prototype guarded both `getSourceFile` and `readFile` before delegation.
  With TypeScript 5.5.4, 5.8.3 and 6.0.3, it read the package declaration and rejected the
  external implementation before those host callbacks read it. This is evidence for that fixture,
  not a complete filesystem security guarantee. TypeScript 4.9.5 did not follow the fixture's
  `bundler` resolution mode; ignoring config diagnostics would have produced a false success.
  A production host must reject unsupported compiler/configuration combinations and intercept
  every content-read path, including config and package handling as appropriate.
* Extending the Rust resolver to traverse declaration imports would still need authoritative
  handling of import types, triple-slash references, package imports, conditions, version ranges,
  `rootDirs`, extension substitutions, ambient modules and mode-dependent precedence. It is not
  established that unsupported cases can be detected reliably without duplicating a significant
  portion of TypeScript. A collection of narrow fallbacks would preserve silent omissions.
* A hybrid is the preferred next design: use the selected project's compatible TypeScript API to
  provide program membership and resolution evidence through a guarded host, then independently
  authorize logical and canonical implementation identities in type-tune. Do not let package
  names, `node_modules`, declarations or compiler resolution confer authorization. Reconcile
  every authorized implementation edge with the native import/referrer graph, including sources
  reached through declaration chains; reject unsupported or unrepresentable edges before scans
  and mutations. The host must handle declaration cycles and report provenance of rejected edges.

This hybrid is a larger architectural change, not a drop-in resolver fix. The native Rust core
currently scans without requiring Node or the TypeScript package; the published native package
contains a binary, while TypeScript is only a development dependency. A supported bridge needs
an explicit runtime and version policy, a versioned and bounded protocol, robust error handling,
and integration in the core and every workflow. Vue single-file components also use a separate
script parser; plain TypeScript program membership does not establish vue-tsc/Nuxt parity. Config
diagnostics, ambient declarations, type references, package modes, logical symlink identity,
`preserveSymlinks`, runtime cost and compiler-version support need tests before relying on this
evidence. Pnpm declaration symlinks require a declaration-specific policy, not blanket rejection.
The approach must avoid unauthorized implementation reads and preserve no-compiler failure
semantics deliberately rather than silently falling back to the current incomplete native graph.
Compiler plugins and other arbitrary compiler behavior, as well as concurrent filesystem
replacement, remain outside a simple path-check guarantee. Project references remain unsupported.

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
