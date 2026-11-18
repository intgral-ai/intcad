#!/usr/bin/env node
/**
 * Test for remove-enterprise.mjs, on a small made-up `node_modules` tree in a temporary folder:
 * - the check fails on a tree that still has the enterprise content;
 * - removing leaves a tree the check passes, with the stand-in under the chunk's own name and
 *   everything else named `ee` gone, and a second run changes nothing;
 * - a package that only happens to be called `ee-first` is left alone;
 * - the check fails on a changed stand-in, a returned `ee` directory, a source map that names an
 *   enterprise source, enterprise licence text, and an import of a letter the stand-in lacks;
 * - a chunk that exports a name without a stand-in stops the removal;
 * - a tree with no `@mastra/core` in it (a wrong path) does not pass.
 *
 * Nothing of Mastra's is needed or copied: the tree is written here. Run with `node`.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const script = join(dirname(fileURLToPath(import.meta.url)), "remove-enterprise.mjs");
const failures = [];
const expect = (condition, message) => {
  if (!condition) failures.push(message);
};

function run(args) {
  const result = spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  let report = null;
  try {
    report = JSON.parse(result.stdout);
  } catch {
    // an error before the report is on stderr
  }
  return { code: result.status, report, stderr: result.stderr ?? "" };
}

function write(root, path, text) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

const CHUNK = "@mastra/core/dist/ee-AbC123-xY_z9.js";

/** A tree shaped like the published package, with made-up contents. */
function tree(exports = "requireFGA as y, MastraFGAPermissions as i, isEEEnabled as O") {
  const root = join(mkdtempSync(join(tmpdir(), "remove-enterprise-")), "node_modules");
  write(root, "@mastra/core/package.json", JSON.stringify({ name: "@mastra/core", version: "9.9.9" }));
  write(root, "@mastra/core/LICENSE.md", "Content under any directory named `ee/` is under the Mastra Enterprise License.\n");
  write(root, CHUNK, `const secret = 1;\n/** @license Mastra Enterprise License - see ee/LICENSE */\nexport { ${exports} };\n`);
  write(root, `${CHUNK}.map`, JSON.stringify({ version: 3, sources: ["../../_internals/auth/dist/ee-AbC123.js"], mappings: "" }));
  write(root, "@mastra/core/dist/ee-AbC123-Qq1.cjs", "exports.y = 1;\n");
  write(root, "@mastra/core/dist/ee-AbC123-Qq1.cjs.map", JSON.stringify({ version: 3, sources: ["../src/ee/index.ts"], mappings: "" }));
  write(root, "@mastra/core/dist/auth/ee/index.js", `export { y as requireFGA } from "../../ee-AbC123-xY_z9.js";\n`);
  write(root, "@mastra/core/dist/agent-builder/ee/index.js", "export const x = 1;\n");
  write(root, "@mastra/core/dist/_types/@internal_auth@0.0.1/dist/ee/index.d.ts", "export {};\n");
  write(root, "@mastra/core/auth/ee.d.ts", "export * from './../dist/auth/ee';\n");
  write(root, "@mastra/core/dist/run.js", `import { y as requireFGA, i as names } from "./ee-AbC123-xY_z9.js";\nexport const run = () => [requireFGA, names];\n`);
  write(root, "@mastra/core/dist/run.js.map", JSON.stringify({ version: 3, sources: ["../src/run.ts"], mappings: "" }));
  write(root, "ee-first/package.json", JSON.stringify({ name: "ee-first", version: "1.1.1", license: "MIT" }));
  write(root, "ee-first/index.js", "module.exports = function first() {};\n");
  return root;
}

const made = [];
const fresh = (...args) => {
  const root = tree(...args);
  made.push(dirname(root));
  return root;
};

// 1. An untouched tree does not pass the check, and the check changes nothing.
{
  const root = fresh();
  const before = readFileSync(join(root, CHUNK), "utf8");
  const checked = run(["--check", root]);
  expect(checked.code === 1 && checked.report?.status === "failed", "the check must fail on an untouched tree");
  expect(readFileSync(join(root, CHUNK), "utf8") === before && existsSync(join(root, "@mastra/core/dist/auth/ee")), "the check must change nothing");
}

// 2. Removing, and what is left.
const clean = fresh();
{
  const removed = run([clean]);
  expect(removed.code === 0 && removed.report?.status === "none-found", `removing must leave a tree that passes: ${removed.stderr}${JSON.stringify(removed.report?.problems)}`);
  const standIn = readFileSync(join(clean, CHUNK), "utf8");
  expect(!standIn.includes("secret") && !standIn.includes("Mastra Enterprise License"), "the stand-in must hold none of the chunk's text");
  expect(/ as y, \/\/ requireFGA$/mu.test(standIn) && / as i, \/\/ MastraFGAPermissions$/mu.test(standIn) && / as O, \/\/ isEEEnabled$/mu.test(standIn), "the stand-in must export the chunk's letters");
  for (const gone of [`${CHUNK}.map`, "@mastra/core/dist/ee-AbC123-Qq1.cjs", "@mastra/core/dist/ee-AbC123-Qq1.cjs.map", "@mastra/core/dist/auth/ee", "@mastra/core/dist/agent-builder/ee", "@mastra/core/dist/_types/@internal_auth@0.0.1/dist/ee", "@mastra/core/auth/ee.d.ts"]) {
    expect(!existsSync(join(clean, gone)), `${gone} must be removed`);
  }
  for (const kept of ["@mastra/core/LICENSE.md", "@mastra/core/dist/run.js", "ee-first/index.js"]) expect(existsSync(join(clean, kept)), `${kept} must be kept`);
  const again = run([clean]);
  expect(again.code === 0 && again.report?.removed?.files === 0 && again.report?.removed?.directories === 0, "a second run must change nothing");
  expect(readFileSync(join(clean, CHUNK), "utf8") === standIn, "a second run must leave the stand-in as it is");
  const loaded = spawnSync(process.execPath, ["--input-type=module", "-e", `const m = await import(${JSON.stringify(join(clean, "@mastra/core/dist/run.js"))}); const [requireFGA, names] = m.run(); await requireFGA({}); if (names.TOOLS_EXECUTE !== "tools:execute") process.exit(3);`], { encoding: "utf8" });
  expect(loaded.status === 0, `a file that imports from the stand-in must load and run: ${loaded.stderr}`);
}

// 3. What the check refuses in a tree that had passed.
const refusals = [
  ["a changed stand-in", (root) => writeFileSync(join(root, CHUNK), `${readFileSync(join(root, CHUNK), "utf8")}export const extra = 1;\n`)],
  ["a directory named ee", (root) => write(root, "@mastra/core/dist/auth/ee/index.js", "export {};\n")],
  ["a directory named EE in another package", (root) => write(root, "other/lib/EE/index.js", "export {};\n")],
  ["another ee file in a @mastra package", (root) => write(root, "@mastra/core/dist/ee-other.cjs", "exports.x = 1;\n")],
  ["a source map naming an enterprise source", (root) => write(root, "@mastra/core/dist/chunk.js.map", JSON.stringify({ version: 3, sources: ["../src/auth/ee/license.ts"], mappings: "" }))],
  ["enterprise licence text in code", (root) => write(root, "@mastra/core/dist/chunk.js", "/** @license Mastra Enterprise License - see ee/LICENSE */\n")],
  ["an import of a letter the stand-in lacks", (root) => write(root, "@mastra/core/dist/other.js", `import { Z as missing } from "./ee-AbC123-xY_z9.js";\n`)],
];
for (const [name, change] of refusals) {
  const root = fresh();
  expect(run([root]).code === 0, `${name}: the tree must pass before the change`);
  change(root);
  const checked = run(["--check", root]);
  expect(checked.code === 1 && (checked.report?.problemCount ?? 0) > 0, `the check must fail on ${name}`);
}

// 4. A name without a stand-in stops the removal, and nothing is written.
{
  const root = fresh("requireFGA as y, somethingNew as q");
  const before = readFileSync(join(root, CHUNK), "utf8");
  const removed = run([root]);
  expect(removed.code === 1 && removed.stderr.includes("somethingNew"), "an export without a stand-in must stop the removal and be named");
  expect(readFileSync(join(root, CHUNK), "utf8") === before, "a stopped removal must not replace the chunk");
}

// 5. A folder that is not the SDK's is refused, not passed as clean.
{
  const root = fresh();
  rmSync(join(root, "@mastra"), { recursive: true });
  const checked = run(["--check", root]);
  expect(checked.code === 1 && checked.report?.status === "failed", "the check must fail on a tree with no @mastra/core");
  expect(run([root]).code === 1, "removing must fail on a tree with no @mastra/core");
}

for (const folder of made) rmSync(folder, { recursive: true, force: true });
if (failures.length > 0) {
  for (const failure of failures) console.error(`FAIL ${failure}`);
  process.exit(1);
}
console.log(`ok remove-enterprise: ${4 + refusals.length} cases`);
