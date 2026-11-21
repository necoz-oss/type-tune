import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const binary = join(repo, "target", "debug", "type-tune.exe");
const compiler = resolve(repo, "tests/compiler-versions/node_modules/typescript58/bin/tsc");
const root = await mkdtemp(join(tmpdir(), "type tune é "));
const source = join(root, "src/features/catalog/schema.ts");
const destination = join(root, "src/features/catalog/types/catalog.ts");
const compilerLog = join(root, "compiler-args.jsonl");

const planArgs = [
  "plan",
  "--project", "tsconfig.json",
  "--out", ".type-tune/plan.json",
  "--markdown", ".type-tune/plan.md",
  "--min-referrers", "1",
  "--default-status", "apply"
];
const applyArgs = [
  "apply",
  "--plan", ".type-tune/plan.json",
  "--markdown", ".type-tune/plan.md"
];

function run(args, success = true) {
  const result = spawnSync(binary, args, { cwd: root, encoding: "utf8" });
  console.log(`${args.join(" ")}: exit ${result.status}\n${result.stdout}${result.stderr}`);
  assert.equal(result.status === 0, success);
  return result;
}

async function prepareProject() {
  await cp(join(repo, "fixtures/cli-plan-apply-project"), root, { recursive: true });
  const config = {
    compilerOptions: {
      strict: true,
      incremental: true,
      tsBuildInfoFile: "custom.tsbuildinfo"
    },
    include: ["src/**/*.ts"]
  };
  await writeFile(join(root, "tsconfig.json"), JSON.stringify(config));

  // Record the arguments and working directory before invoking the real compiler.
  const wrapper = join(root, "compiler-wrapper.cjs");
  const wrapperSource = [
    `const fs = require('fs');`,
    `const call = { cwd: process.cwd(), args: process.argv.slice(2) };`,
    `fs.appendFileSync(${JSON.stringify(compilerLog)}, JSON.stringify(call) + '\\n');`,
    `require(${JSON.stringify(compiler)});`,
    ""
  ].join("\n");
  await writeFile(wrapper, wrapperSource);
  await mkdir(join(root, "node_modules/.bin"), { recursive: true });
  await writeFile(join(root, "node_modules/.bin/tsc.cmd"), `@echo off\r\nnode "${wrapper}" %*\r\n`);
}

async function assertRewriteRolledBack(originalSource) {
  assert.equal(await readFile(source, "utf8"), originalSource);
  await assert.rejects(readFile(destination));
}

async function assertCompilerCalls() {
  const calls = (await readFile(compilerLog, "utf8")).trim().split("\n").map(JSON.parse);
  assert.ok(calls.length >= 6);
  // Windows can report the working directory using either its short or long name.
  const realRoot = (await realpath(root)).toLowerCase();
  for (const call of calls) {
    assert.equal(call.args[call.args.indexOf("--project") + 1], "tsconfig.json");
    assert.equal((await realpath(call.cwd)).toLowerCase(), realRoot);
  }
}

try {
  await prepareProject();
  const originalSource = await readFile(source, "utf8");

  run(["scan", "--project", "tsconfig.json", "--min-referrers", "1"]);
  run(planArgs);
  run([...applyArgs, "--dry-run"]);
  await assertRewriteRolledBack(originalSource);

  // A compiler error in a separate file must reject and roll back the proposed rewrite.
  const invalid = join(root, "src/invalid.ts");
  await writeFile(invalid, "const invalid: string = 42;\n");
  const failure = run(applyArgs, false);
  assert.match(failure.stderr, /rolled back file changes/);
  assert.match(failure.stderr, /TS2322/);
  await assertRewriteRolledBack(originalSource);

  await rm(invalid);
  run(applyArgs);
  assert.notEqual(await readFile(source, "utf8"), originalSource);
  await assertCompilerCalls();
  await assert.rejects(readFile(join(root, "tsconfig.tsbuildinfo")));
  await assert.rejects(readFile(join(root, "custom.tsbuildinfo")));
} finally {
  await rm(root, { recursive: true, force: true });
}
