#!/usr/bin/env node
/**
 * Gate test for the runtime inventory split:
 * - evidence mode passes with recorded needs-human content and reports unapproved;
 * - require-approved fails closed;
 * - inventory drift fails;
 * - an injected, unrecorded ee path fails instead of being pruned.
 *
 * It uses the real installed tree and rebuilds the artifact, so run it after
 * `npm ci` and `npm run build`. It does not write tracked repository files.
 */

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "../..");
const script = join(packageRoot, "scripts/runtime-artifact.mjs");
const inventory = join(repoRoot, "packaging/mastra/inventory.json");
const tempInventory = join(repoRoot, "target/mastra-runtime-inventory-drift.json");
const injected = join(repoRoot, "node_modules/@mastra/core/dist/injected/ee/x.js");

const env = { ...process.env };
if (!env["SOLIDCRAFT_NODE_BIN"]) env["SOLIDCRAFT_NODE_BIN"] = process.execPath;

function run(args) {
  const result = spawnSync(process.execPath, [script, ...args], { cwd: repoRoot, env, encoding: "utf8" });
  return { code: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function expect(name, condition, detail) {
  if (!condition) {
    console.error(`FAIL ${name}`);
    if (detail) console.error(detail);
    process.exitCode = 1;
    return false;
  }
  console.log(`ok ${name}`);
  return true;
}

let ok = true;
try {
  const evidence = run(["--check"]);
  ok = expect(
    "evidence mode exits 0 and reports unapproved",
    evidence.code === 0 && evidence.stdout.includes('"status":"unapproved"'),
    `exit=${evidence.code}\n${evidence.stdout}\n${evidence.stderr}`,
  ) && ok;

  const approved = run(["--check", "--require-approved"]);
  ok = expect(
    "require-approved exits 1 and reports unapproved",
    approved.code === 1 && approved.stdout.includes('"status":"unapproved"'),
    `exit=${approved.code}\n${approved.stdout}\n${approved.stderr}`,
  ) && ok;

  mkdirSync(dirname(tempInventory), { recursive: true });
  copyFileSync(inventory, tempInventory);
  const value = JSON.parse(readFileSync(tempInventory, "utf8"));
  value.outputs[0].digest = `sha256:${"0".repeat(64)}`;
  writeFileSync(tempInventory, `${JSON.stringify(value, null, 2)}\n`);
  const drift = run(["--check", "--inventory", tempInventory]);
  ok = expect(
    "inventory drift exits 1",
    drift.code === 1 && /inventory drift/u.test(drift.stderr),
    `exit=${drift.code}\n${drift.stdout}\n${drift.stderr}`,
  ) && ok;

  mkdirSync(dirname(injected), { recursive: true });
  writeFileSync(injected, "// @license Mastra Enterprise License\n");
  const injectedResult = run(["--check"]);
  ok = expect(
    "unrecorded ee path exits 1",
    injectedResult.code === 1 && /unexpected ee path in shipped package/u.test(injectedResult.stderr),
    `exit=${injectedResult.code}\n${injectedResult.stdout}\n${injectedResult.stderr}`,
  ) && ok;
  if (!ok) process.exitCode = 1;
} finally {
  rmSync(tempInventory, { force: true });
  rmSync(dirname(dirname(injected)), { recursive: true, force: true });
}
