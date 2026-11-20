# Experimental compiler resolution bridge

Status: experimental. `--compiler-resolution-bridge` is opt-in on `scan`, `plan`, `extract`,
and `apply`. The default commands still use the native Rust resolver and do not require Node.js
or a TypeScript package. Do not treat a default scan as a complete TypeScript program.

## Responsibilities

Rust remains the analysis and transformation engine. It owns:

- the project-boundary policy;
- source-identity validation;
- reconciliation of compiler evidence into the source graph;
- candidate and referrer analysis;
- plan generation, transformation, apply safety, rollback, and validation.

The bridge is one Node process per project. It loads the project-local TypeScript package and
returns evidence:

- program source files and their roles;
- resolved module targets, including declaration re-exports, import types, and triple-slash paths;
- logical and canonical paths;
- resolution failures;
- configuration diagnostics;
- a completion flag.

The bridge does not enlarge the authorized project root. Package names, `node_modules`,
declaration files, and a successful module resolution do not authorize an implementation source.
Rust repeats the implementation-source check after the process exits. A graph is usable only when
the protocol status is `ok`, `complete` is true, and `coverage` is `complete`.

JavaScript does not keep a second evolving policy. The process receives a versioned policy
snapshot (`policy.version` 1) and applies those extensions, dependency-segment, symlink, and
reason strings. Rust is the source of that snapshot.

## Trust boundary

Finding `node_modules/typescript/lib/typescript.js` walks from the project root to parent
directories, matching the existing project-local compiler lookup. The bridge then executes that
package with Node. That dependency must be trusted. The guarded host is not a sandbox against
arbitrary compiler, plugin, or preload code, and it does not defend against concurrent filesystem
replacement.

Node is `TYPE_TUNE_NODE` when that value is an existing file, otherwise `node` on `PATH`.
`NODE_OPTIONS` is cleared for the child. The bridge does not install dependencies, run package
scripts, or invoke a bundler. It does not call `emit` and it refuses host writes. There is no
fallback to native resolution when Node, TypeScript, the compiler version, or the configuration
cannot support a complete graph.

## Compiler support

Tested range: TypeScript 4.9 through 6.x. The package version is read from the located
`package.json` before the process starts. Older and newer majors are refused.

`moduleResolution` must be one the selected compiler accepts. TypeScript 4.9 rejects `bundler`
as an invalid configuration; that failure is a diagnostics result, not evidence that a boundary
was enforced. Supported checks include relative and transitive imports, declaration re-exports,
import types, triple-slash references, declaration cycles, package `exports`, `typesVersions`,
`rootDirs`, and pnpm-style dependency symlinks. Declaration reads and `package.json` reads remain
allowed. Implementation sources outside the canonical project root, and implementation sources
under `node_modules`, are unauthorized. Source symlink identities are unsupported, consistent
with Phase 1-A.

Vue single-file components and Nuxt projects are refused before an incomplete program can be
reported as complete. Compiler plugins and project-reference traversal are also refused.

## Protocol

Request and response use `type-tune.compiler-resolution.v1`.

Response `status` is one of:

| Status | Meaning |
| --- | --- |
| `ok` | Evidence was produced. Usable only when `coverage` is `complete`. |
| `unauthorized` | An implementation source failed the policy before its contents were read. |
| `unsupported` | The compiler, framework, plugin, symlink, or graph shape cannot be represented. |
| `diagnostics` | Configuration is invalid or uses a resolution mode this compiler does not support. |
| `malformed` | The bridge output could not be interpreted. |

Process timeouts (120 seconds), output limits (8 MiB), and graph limits (20,000 sources,
100,000 edges) fail closed. Cycles are recorded as edges; the walk does not recurse without a
visited set.

## Reconciliation

Compiler implementation sources that pass Rust's identity checks and whose edges the native
referrer resolver can represent are added to the analysis graph. Declaration-mediated project
sources and other edges the native graph cannot represent produce an explicit diagnostic and no
candidates. Unauthorized sources produce no candidates and no plan.

The known fixture is `fixtures/compiler-resolution-bridge/declaration-mediated-external/`.
`src/main.ts` imports `external-pkg`, its declaration re-exports `./hidden-source`, and that
implementation lives outside the project through a `node_modules` symlink. The native regression
stays ignored. The experimental suite rejects the scan.

## Measured cost

Compared runs used the debug binary. Candidate sets matched, so the timing comparison is for the
same analysis result. `/usr/bin/time` max RSS is reported in megabytes. A cold page-cache run was
not isolated; the repeated runs below are warm.

Generated project, 201 implementation sources, 1 candidate, 200 referrers, TypeScript 6.0.3:

| Mode | Wall time | Max RSS |
| --- | --- | --- |
| Native | 0.05s, 0.05s, 0.06s | 16 MB |
| Hybrid | 0.56s, 0.56s, 0.55s | 213 MB |

The matching hybrid trace reported Node peak RSS of 223 MB, 283 program sources including
TypeScript libraries, and bridge-internal time of 126 ms configuration, 263 ms program
construction, and 23 ms graph serialization. Process time was 484 ms. A later phased run of the
same shape reported 35 ms Rust discovery, 524 ms in the Node process (125 ms configuration,
304 ms program construction, 24 ms graph serialization), about 27 ms of Rust reconciliation
around that process, and 24 ms of Rust candidate analysis over 201 project sources.

is-kit, same candidate and 4 referrers on both modes:

| Mode | Wall time | Max RSS |
| --- | --- | --- |
| Native | 0.27s | 16 MB |
| Hybrid | 0.61s | 215 MB |

The bridge adds roughly 400–500 ms and about 200 MB for these projects. On is-kit that is a
little more than 2× the native scan. On the generated project, where native scanning is already
about 50 ms, it is about 10×. The hybrid path does not keep the latency advantage of the Rust
scanner for small and medium projects. It is still a single compiler process per project, not a
process per import.

A local Nuxt tutorial project scanned natively and was refused by the bridge because Vue or Nuxt
sources are not represented. No Vitesse checkout was available.

## Adoption

Keep the bridge experimental. Plain TypeScript projects in the tested compiler range can use it
to reject declaration-mediated external implementation sources. It is not a replacement for the
default scanner until framework programs can be represented without an incomplete graph and the
startup and memory cost is acceptable.
