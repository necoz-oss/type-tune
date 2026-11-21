import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const binary = join(repo, "target", "debug", "type-tune.exe");
const root = await mkdtemp(join(tmpdir(), "type tune é "));
const source = join(root, "src/features/catalog/schema.ts");
const destination = join(root, "src/features/catalog/types/catalog.ts");
const log = join(root, "compiler-args.jsonl");
const compiler = resolve(repo, "tests/compiler-versions/node_modules/typescript58/bin/tsc");

function run(args, success = true) {
  const result = spawnSync(binary, args, { cwd: root, encoding: "utf8" });
  console.log(`${args.join(" ")}: exit ${result.status}\n${result.stdout}${result.stderr}`);
  assert.equal(result.status === 0, success);
  return result;
}

try {
  await cp(join(repo, "fixtures/cli-plan-apply-project"), root, { recursive: true });
  await writeFile(join(root, "tsconfig.json"), JSON.stringify({ compilerOptions: { strict: true, incremental: true, tsBuildInfoFile: "custom.tsbuildinfo" }, include: ["src/**/*.ts"] }));
  await mkdir(join(root, "node_modules/.bin"), { recursive: true });
  const wrapper = join(root, "compiler-wrapper.cjs");
  await writeFile(wrapper, `require('fs').appendFileSync(${JSON.stringify(log)}, JSON.stringify({cwd: process.cwd(), args: process.argv.slice(2)}) + '\\n');\nrequire(${JSON.stringify(compiler)});\n`);
  await writeFile(join(root, "node_modules/.bin/tsc.cmd"), `@echo off\r\nnode "${wrapper}" %*\r\n`);
  const original = await readFile(source, "utf8");
  const plan = ["plan", "--project", "tsconfig.json", "--out", ".type-tune/plan.json", "--markdown", ".type-tune/plan.md", "--min-referrers", "1", "--default-status", "apply"];
  const apply = ["apply", "--plan", ".type-tune/plan.json", "--markdown", ".type-tune/plan.md"];
  run(["scan", "--project", "tsconfig.json", "--min-referrers", "1"]);
  run(plan);
  run([...apply, "--dry-run"]);
  assert.equal(await readFile(source, "utf8"), original);
  await assert.rejects(readFile(destination));
  // A compiler error in a separate file must reject and roll back the proposed rewrite.
  const invalid = join(root, "src/invalid.ts");
  await writeFile(invalid, "const invalid: string = 42;\n");
  const failure = run(apply, false);
  assert.match(failure.stderr, /rolled back file changes/);
  assert.match(failure.stderr, /TS2322/);
  assert.equal(await readFile(source, "utf8"), original);
  await assert.rejects(readFile(destination));
  await rm(invalid);
  run(apply);
  assert.notEqual(await readFile(source, "utf8"), original);
  const calls = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  assert.ok(calls.length >= 6);
  const realRoot = (await realpath(root)).toLowerCase();
  for (const call of calls) {
    assert.equal(call.args[call.args.indexOf("--project") + 1], "tsconfig.json");
    assert.equal((await realpath(call.cwd)).toLowerCase(), realRoot);
  }
  await assert.rejects(readFile(join(root, "tsconfig.tsbuildinfo")));
  await assert.rejects(readFile(join(root, "custom.tsbuildinfo")));
} finally {
  await rm(root, { recursive: true, force: true });
}
