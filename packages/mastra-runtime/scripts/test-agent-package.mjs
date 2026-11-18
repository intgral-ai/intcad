#!/usr/bin/env node
/**
 * Test for agent-package.mjs's `check` and `scan`, on made-up folders in a temporary place:
 * - a whole folder for the asked target passes;
 * - it fails when it is for another target, lacks its Node, still has enterprise content, holds a
 *   package the build leaves out, a package the lock file does not name or names at another
 *   version, a package under a refused licence or with none stated, or another session builder
 *   of the SDK than the one the agent's own follows; with `--read-only`, when a file can be written;
 * - the scan finds a key-shaped text (also one lying across two reads), a private key with its
 *   key material, a stored sign-in, a database and an agent's own folder, and names the file and
 *   the kind without printing what it found;
 * - the scan lets pass what only names a key format, and an editor's settings file.
 *
 * `build` is not run here: it installs from the npm registry. Nothing of Mastra's is needed.
 * The key-shaped texts are put together when the test runs, so this file holds none.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const failures = [];
const expect = (condition, message) => {
  if (!condition) failures.push(message);
};
const work = mkdtempSync(join(tmpdir(), "agent-package-test-"));

function run(script, args) {
  const result = spawnSync(process.execPath, [join(here, script), ...args], { encoding: "utf8" });
  let report = null;
  try {
    report = JSON.parse(result.stdout);
  } catch {
    // an error before the report is on stderr
  }
  return { code: result.status, report, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function write(root, path, text) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

const TARGET = "x86_64-unknown-linux-gnu";
let count = 0;

// A pin folder of the test's own: a lock for the made-up packages, and the checksum of the made-up
// file that stands for the SDK's session builder.
const BUILDER = "export const createAcpSession = () => {};\n";
const PIN = join(work, "pin");
const locked = { "@mastra/code-sdk": "1.0.0", "@agentclientprotocol/sdk": "1.0.0", "@mastra/core": "1.0.0", "left-pad": "1.0.0", "copyleft-thing": "1.0.0", "unstated-thing": "1.0.0", "@mastra/pg": "1.0.0" };
write(PIN, "package.json", JSON.stringify({ dependencies: { "@mastra/code-sdk": "1.0.0", "@agentclientprotocol/sdk": "1.0.0" } }));
write(PIN, "package-lock.json", JSON.stringify({ packages: { "": {}, ...Object.fromEntries(Object.entries(locked).map(([name, version]) => [`node_modules/${name}`, { version, resolved: `https://registry.npmjs.org/${name}/-/x-${version}.tgz`, integrity: "sha512-AAAA" }])) } }));
write(PIN, "adapted-from.json", JSON.stringify({ package: "@mastra/code-sdk", version: "1.0.0", file: "dist/acp/runtime.js", sha256: createHash("sha256").update(BUILDER).digest("hex"), adaptedIn: "agent/session.mjs" }));
const checkFolder = (root, target = TARGET, ...more) => run("agent-package.mjs", ["check", "--pin", PIN, "--target", target, ...more, root]);

/** A folder shaped like a built one, with made-up contents. */
function folder() {
  const root = join(work, `folder-${++count}`);
  const pkg = (name, licence, extra = {}) => write(root, `node_modules/${name}/package.json`, JSON.stringify({ name, version: "1.0.0", ...(licence ? { license: licence } : {}), ...extra }));
  write(root, "node", "not a real program\n");
  write(root, "agent/main.mjs", "// the entry\n");
  write(root, "agent.json", JSON.stringify({ target: TARGET }));
  write(root, "licenses/THIRD-PARTY-LICENSES.txt", "texts\n");
  write(root, "licenses/NODE-LICENSE.txt", "text\n");
  pkg("@mastra/code-sdk", "Apache-2.0");
  pkg("@agentclientprotocol/sdk", "Apache-2.0");
  pkg("@mastra/core", "Apache-2.0");
  pkg("left-pad", "MIT");
  write(root, "node_modules/@mastra/code-sdk/dist/acp/runtime.js", BUILDER);
  write(root, "node_modules/@mastra/core/dist/ee-AbC123-xY_z9.js", "export { requireFGA as y };\n");
  write(root, "node_modules/@mastra/core/dist/run.js", `import { y as requireFGA } from "./ee-AbC123-xY_z9.js";\nexport { requireFGA };\n`);
  const removed = run("remove-enterprise.mjs", [join(root, "node_modules")]);
  if (removed.code !== 0) throw new Error(`could not prepare the folder: ${removed.stderr}${removed.stdout}`);
  return { root, pkg };
}

// 1. A whole folder passes, and only for its own target.
{
  const { root } = folder();
  const checked = checkFolder(root);
  expect(checked.code === 0 && checked.report?.status === "ok", `a whole folder must pass: ${checked.stderr}${JSON.stringify(checked.report?.problems)}`);
  expect(checked.report?.licences?.packages === 4, "the report must count the packages");
  const other = checkFolder(root, "aarch64-apple-darwin");
  expect(other.code === 1 && other.report?.problems?.some((problem) => problem.includes("built for")), "a folder for another target must fail");
  // Against the repository's own pin the made-up folder is nothing the lock file names.
  const real = run("agent-package.mjs", ["check", "--target", TARGET, root]);
  expect(real.code === 1 && real.report?.problems?.some((problem) => problem.includes("lock file")), "without --pin the repository's lock file is what counts");
  if (process.platform !== "win32") {
    const writable = checkFolder(root, TARGET, "--read-only");
    expect(writable.code === 1 && writable.report?.problems?.some((problem) => problem.includes("can be written")), "--read-only must fail on a folder that can be written");
    const lock = (path) => {
      if (statSync(path).isDirectory()) for (const name of readdirSync(path)) lock(join(path, name));
      chmodSync(path, statSync(path).mode & ~0o222);
    };
    lock(root);
    const sealed = checkFolder(root, TARGET, "--read-only");
    expect(sealed.code === 0, `--read-only must pass on a folder nothing can write: ${JSON.stringify(sealed.report?.problems)}`);
    const unlock = (path) => {
      chmodSync(path, statSync(path).mode | 0o200);
      if (statSync(path).isDirectory()) for (const name of readdirSync(path)) unlock(join(path, name));
    };
    unlock(root);
  }
}

// 2. What makes a folder fail.
const breaks = [
  ["its Node missing", ({ root }) => rmSync(join(root, "node")), "missing: node"],
  ["a directory named ee", ({ root }) => write(root, "node_modules/@mastra/core/dist/auth/ee/index.js", "export {};\n"), "named ee"],
  ["a package the build leaves out", ({ pkg }) => pkg("@mastra/pg", "Apache-2.0"), "left-out package present: @mastra/pg"],
  ["a refused licence", ({ pkg }) => pkg("copyleft-thing", "(MIT OR GPL-3.0-or-later)"), "refuses"],
  ["a package with no licence stated", ({ pkg }) => pkg("unstated-thing", null), "no licence stated"],
  ["a stored sign-in", ({ root }) => write(root, "node_modules/left-pad/auth.json", "{}\n"), "keeps a sign-in"],
  ["a package the lock file does not name", ({ pkg }) => pkg("slipped-in", "MIT"), "not in the lock file: node_modules/slipped-in"],
  ["a package at another version than the lock file's", ({ pkg }) => pkg("left-pad", "MIT", { version: "1.0.1" }), "the lock file has 1.0.0"],
  ["another session builder in the SDK", ({ root }) => write(root, "node_modules/@mastra/code-sdk/dist/acp/runtime.js", `${BUILDER}// changed upstream\n`), "was adapted from"],
];
for (const [name, change, said] of breaks) {
  const made = folder();
  change(made);
  const checked = checkFolder(made.root);
  expect(checked.code === 1 && checked.report?.problems?.some((problem) => problem.includes(said)), `the check must fail on ${name} and say "${said}": ${JSON.stringify(checked.report?.problems)}`);
}

// 3. The scan. Each text is assembled here so that no key-shaped text is in this file.
const letters = "Qw3Er7Ty1Ui9Op5As2Df8Gh4Jk6Lz0Xc";
const key = ["sk", letters].join("-");
const pem = (body) => `${"-".repeat(5)}BEGIN PRIVATE KEY${"-".repeat(5)}${body}`;
const found = [
  ["a key in a text file", "notes/readme.txt", `the key is ${key} and more\n`, "an API key"],
  ["a key lying across two reads", "notes/large.bin", `${"x".repeat(4 * 1024 * 1024 - 10)} ${key}\n`, "an API key"],
  ["a private key with its key material", "keys/id.pem", `${pem("\n")}${letters.repeat(3)}\n`, "a private key"],
  ["a stored sign-in", "data/auth.json", "{}\n", "keeps a sign-in"],
  ["a database", "data/mastra.db", "\0\0\0", "a database"],
  ["an agent's own folder", "home/.mastracode/analytics.json", "{}\n", "own folder"],
];
for (const [name, path, text, said] of found) {
  const root = join(work, `scan-${++count}`);
  write(root, path, text);
  const scanned = run("agent-package.mjs", ["scan", root]);
  const finding = scanned.report?.results?.[0]?.findings?.[0];
  expect(scanned.code === 1 && finding?.path === path && finding.kind.includes(said), `the scan must find ${name}: ${scanned.stdout.slice(0, 200)}`);
  expect(!scanned.stdout.includes(letters) && !scanned.stderr.includes(letters), `the scan must not print what it found (${name})`);
}
{
  const root = join(work, `scan-${++count}`);
  write(root, "lib/pem.js", `const body = text.replace("${pem('", "")')};\n`);
  write(root, "docs/css.txt", "mask-border-source-and-other-long-property-names-0123456789\n");
  write(root, ".vscode/settings.json", "{}\n");
  write(root, "config/.env.example", "KEY=\n");
  const scanned = run("agent-package.mjs", ["scan", root]);
  expect(scanned.code === 0 && scanned.report?.status === "none-found", `the scan must let pass what only names a format: ${scanned.stdout.slice(0, 300)}`);
}

rmSync(work, { recursive: true, force: true });
if (failures.length > 0) {
  for (const failure of failures) console.error(`FAIL ${failure}`);
  process.exit(1);
}
console.log(`ok agent-package: ${5 + breaks.length + found.length + 1} cases`);
