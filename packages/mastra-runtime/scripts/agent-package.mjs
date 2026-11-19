#!/usr/bin/env node
/**
 * Build and check the folder a package carries the built-in agent in (`mastra-agent`).
 *
 *   node agent-package.mjs build --target <triple> --out <dir> [--node <node program>]
 *   node agent-package.mjs check --target <triple> [--read-only] <dir>
 *   node agent-package.mjs scan <dir>...
 *
 * (`--pin <dir>` names another folder in place of `packages/mastra-runtime/agent-package`; the
 * tests use it with made-up folders, and no package script passes it.)
 *
 * `build` makes the folder from nothing but the repository and the npm registry:
 *   1. `npm ci --ignore-scripts` of `packages/mastra-runtime/agent-package/package-lock.json` for
 *      the target's system and CPU, into a new folder. No package's install script runs.
 *   2. Mastra's enterprise-licensed content is taken out (remove-enterprise.mjs).
 *   3. What Node never loads is dropped: source maps, type declaration files, native binaries of
 *      other systems.
 *   4. Whole packages are left out that only a feature Intcad's agent does not offer reaches, each
 *      through an `import()` on demand (LEFT_OUT below says which, and where it is imported).
 *   5. The agent's entry files and a Node program are put beside `node_modules`, with the licence
 *      texts of every package that is left.
 *   6. The folder is checked as `check` checks it.
 *
 * `check` fails (exit 1) when the folder is not whole, is for another target, still holds
 * enterprise content, holds a package the lock file does not name at that version, a package
 * under a licence the repository refuses or with none stated, or something that looks like a
 * secret or a stored sign-in; when Mastra Code's session builder is not the file the agent's own
 * was adapted from; and, with `--read-only`, when anything in the folder can be written. A
 * package script takes the folder only after this passes. `scan` runs the secret check alone on any folders, such as a
 * staged package; it prints paths and kinds, never what it found.
 *
 * What a passing folder shows is what these checks look at. Nobody has reviewed the licences the
 * packages state, and nothing here says Mastra approved anything.
 *
 * The folder is for test packages. It does not go through `cargo xtask runtime-artifact`, which
 * admits the other artifact (the workflow runtime in `mastra/`), and it adds no approval.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, closeSync, cpSync, existsSync, lstatSync, mkdirSync, openSync, readFileSync, readSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { checkEnterprise, removeEnterprise } from "./remove-enterprise.mjs";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "../..");
/** The folder that pins what the agent is built from: the two packages, their lock, and the SDK file the agent's session builder follows. */
const PIN_DIR = join(packageRoot, "agent-package");
const readPin = (dir) => ({
  dir,
  pinned: JSON.parse(readFileSync(join(dir, "package.json"), "utf8")).dependencies,
  lock: JSON.parse(readFileSync(join(dir, "package-lock.json"), "utf8")).packages,
  adapted: JSON.parse(readFileSync(join(dir, "adapted-from.json"), "utf8")),
});

/** The targets a test package is built for, as npm and Node name them. */
const TARGETS = {
  "aarch64-apple-darwin": { os: "darwin", cpu: "arm64", node: "node" },
  "x86_64-unknown-linux-gnu": { os: "linux", cpu: "x64", libc: "glibc", node: "node" },
  "x86_64-pc-windows-msvc": { os: "win32", cpu: "x64", node: "node.exe" },
};

/**
 * Packages left out with everything only they need. Each is reached from the agent only by an
 * `import()` that runs when its feature is used; `where` names the importing file.
 */
const LEFT_OUT = [
  { feature: "browser automation tools", packages: ["@mastra/agent-browser", "@mastra/stagehand"], where: "@mastra/code-sdk dist/onboarding/settings.js" },
  { feature: "DuckDB trace store (/observability local on)", packages: ["@mastra/duckdb"], where: "@mastra/code-sdk dist/utils/local-observability.js" },
  { feature: "voice transcription", packages: ["@mastra/voice-deepgram", "@mastra/voice-mistral", "@mastra/voice-openai"], where: "@mastra/code-sdk dist/voice/transcribe.js" },
  { feature: "Postgres storage", packages: ["@mastra/pg"], where: "@mastra/code-sdk dist/utils/storage-factory.js" },
  { feature: "the type-generation command line of @mastra/mcp", packages: ["tsx", "json-schema-to-typescript"], where: "@mastra/mcp dist/cli/ (not its library entry)" },
  // The agent's sessions have no vector store (agent/session.mjs), so nothing asks for an embedding.
  { feature: "embedding text with a model on this computer", packages: ["@mastra/fastembed"], where: "@mastra/code-sdk dist/agents/fastembed-subprocess.js, a child process of a session with a vector store" },
];

/** Licences the repository refuses whatever a policy says (docs/mastra-distribution.md). */
const REFUSED_LICENCE = /GPL|SSPL|CPAL|EUPL|OSL|MPL|EPL|CDDL|CeCILL|CC-BY-SA/iu;
const PLAIN_LICENCE = /^(MIT|ISC|BSD-2-Clause|BSD-3-Clause|Apache-2\.0)$/u;

const fail = (message) => {
  throw new Error(message);
};
const slashed = (path) => path.split(sep).join("/");

/** Every entry under `root`, as `[path relative to root with "/", Dirent]`. Links are not followed. */
function* entries(root, dir = root) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(dir, entry.name);
    yield [slashed(relative(root, path)), entry];
    if (entry.isDirectory() && !entry.isSymbolicLink()) yield* entries(root, path);
  }
}

function size(root) {
  let bytes = 0;
  let files = 0;
  for (const [path, entry] of entries(root)) {
    if (entry.isFile()) {
      bytes += lstatSync(join(root, path)).size;
      files += 1;
    }
  }
  return { bytes, files };
}

// ---- The lock file -----------------------------------------------------------------------------

/** Every dependency is pinned to one version, from the npm registry, with its checksum. */
function lockProblems({ pinned, lock }) {
  const problems = [];
  for (const [name, version] of Object.entries(pinned)) {
    if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/u.test(version)) problems.push(`${name} is not pinned to one version: ${version}`);
    else if (lock[`node_modules/${name}`]?.version !== version) problems.push(`the lock file has ${name} at ${lock[`node_modules/${name}`]?.version}, the pin is ${version}`);
  }
  for (const [key, entry] of Object.entries(lock)) {
    if (!key) continue;
    if (!entry.version || !entry.resolved?.startsWith("https://registry.npmjs.org/") || !/^sha512-[A-Za-z0-9+/]+=*$/u.test(entry.integrity ?? "")) {
      problems.push(`the lock file entry ${key} lacks a version, an npm registry address or a sha512 checksum`);
    }
  }
  return problems;
}

/** Every package in the folder is one the lock names, at the lock's version. npm checked each archive against the lock's checksum when it installed it. */
function lockedProblems(list, lock) {
  const problems = [];
  for (const item of list) {
    const entry = lock[`node_modules/${item.path}`];
    if (!entry) problems.push(`not in the lock file: node_modules/${item.path}`);
    else if (entry.version !== item.version) problems.push(`node_modules/${item.path} is ${item.version}, the lock file has ${entry.version}`);
  }
  return problems;
}

// ---- Installing --------------------------------------------------------------------------------

function install(target, into, pin) {
  const unlocked = lockProblems(pin);
  if (unlocked.length > 0) fail(unlocked.slice(0, 5).join("; "));
  mkdirSync(into, { recursive: true });
  for (const name of ["package.json", "package-lock.json"]) cpSync(join(pin.dir, name), join(into, name));
  const args = ["ci", "--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund", `--os=${target.os}`, `--cpu=${target.cpu}`];
  if (target.libc) args.push(`--libc=${target.libc}`);
  // npm is a .cmd file on Windows, which Node starts only through a shell.
  const done = spawnSync("npm", args, { cwd: into, stdio: ["ignore", "inherit", "inherit"], shell: process.platform === "win32" });
  if (done.status !== 0) fail(`npm ci failed (exit ${done.status ?? done.signal})`);
  for (const [name, version] of Object.entries(pin.pinned)) {
    const installed = JSON.parse(readFileSync(join(into, "node_modules", ...name.split("/"), "package.json"), "utf8")).version;
    if (installed !== version) fail(`${name} was installed as ${installed}, the pin is ${version}`);
  }
  return join(into, "node_modules");
}

// ---- Dropping what is not loaded ---------------------------------------------------------------

function drop(path, tally) {
  const stat = lstatSync(path);
  if (stat.isDirectory()) {
    const { bytes, files } = size(path);
    rmSync(path, { recursive: true, force: true });
    tally.bytes += bytes;
    tally.files += files;
  } else {
    rmSync(path, { force: true });
    tally.bytes += stat.size;
    tally.files += 1;
  }
}

/** Source maps, type declaration files, and native binaries for another system, CPU or C library. */
function dropUnloaded(modules, target) {
  const tally = { maps: { bytes: 0, files: 0 }, declarations: { bytes: 0, files: 0 }, otherSystems: { bytes: 0, files: 0 } };
  const doomed = [];
  for (const [path, entry] of entries(modules)) {
    const name = entry.name;
    const parent = path.slice(0, path.length - name.length - 1);
    if (entry.isDirectory()) {
      // onnxruntime-node keeps every system's library in bin/napi-vN/<system>/<cpu>.
      if (/(^|\/)onnxruntime-node\/bin\/napi-v\d+$/u.test(parent) && name !== target.os) doomed.push([path, "otherSystems"]);
      else if (/(^|\/)onnxruntime-node\/bin\/napi-v\d+\/[^/]+$/u.test(parent) && name !== target.cpu) doomed.push([path, "otherSystems"]);
      // Prebuilt addons are in prebuilds/<system>-<cpu>.
      else if (/(^|\/)prebuilds$/u.test(parent) && !(name.startsWith(`${target.os}-`) && name.includes(target.cpu))) doomed.push([path, "otherSystems"]);
      // npm installs a native package for every C library of a Linux target.
      else if (target.libc === "glibc" && /(^|\/)node_modules\/(@[^/]+\/)?[^/]+$/u.test(`node_modules/${path}`) && /-musl(eabihf)?$/u.test(name)) doomed.push([path, "otherSystems"]);
    } else if (name.endsWith(".map")) doomed.push([path, "maps"]);
    else if (/\.d\.[cm]?ts$/u.test(name)) doomed.push([path, "declarations"]);
  }
  for (const [path, kind] of doomed) if (existsSync(join(modules, path))) drop(join(modules, path), tally[kind]);
  return tally;
}

/** Remove the LEFT_OUT packages and every package only they lead to, by the lock file's graph. */
function leaveOut(modules, lock) {
  const resolveFrom = (from, name) => {
    for (let base = from; ; ) {
      const candidate = `${base ? `${base}/` : ""}node_modules/${name}`;
      if (lock[candidate]) return candidate;
      if (!base) return null;
      const cut = base.lastIndexOf("/node_modules/");
      base = cut >= 0 ? base.slice(0, cut) : "";
    }
  };
  const reach = (skip) => {
    const seen = new Set();
    const todo = [""];
    while (todo.length > 0) {
      const key = todo.pop();
      const entry = lock[key];
      for (const name of Object.keys({ ...entry.dependencies, ...entry.optionalDependencies, ...entry.peerDependencies })) {
        const found = resolveFrom(key, name);
        if (!found || seen.has(found) || skip.has(found)) continue;
        seen.add(found);
        todo.push(found);
      }
    }
    return seen;
  };
  const roots = new Set(LEFT_OUT.flatMap((cut) => cut.packages.map((name) => `node_modules/${name}`)));
  for (const root of roots) if (!lock[root]) fail(`${root.slice(13)} is named in LEFT_OUT but is not in the lock file`);
  const kept = reach(roots);
  const tally = { bytes: 0, files: 0, packages: 0 };
  // Shortest first: a package goes with the copies nested inside it.
  for (const key of [...reach(new Set())].filter((key) => !kept.has(key)).sort((a, b) => a.length - b.length)) {
    const path = join(modules, key.slice("node_modules/".length));
    if (!existsSync(path)) continue;
    drop(path, tally);
    tally.packages += 1;
  }
  // The command links npm made, and its own copy of the lock: neither is read at run time, and
  // both would name packages that are gone.
  for (const leftover of [".bin", ".package-lock.json"]) rmSync(join(modules, leftover), { recursive: true, force: true });
  for (const scope of readdirSync(modules).filter((name) => name.startsWith("@"))) {
    if (readdirSync(join(modules, scope)).length === 0) rmSync(join(modules, scope), { recursive: true });
  }
  return tally;
}

// ---- Licences ----------------------------------------------------------------------------------

/** Every installed package (nested copies too): name, version, the licence it states, its licence files. */
function packages(modules) {
  const found = [];
  const visit = (dir, prefix) => {
    for (const name of readdirSync(dir).sort()) {
      if (name.startsWith(".")) continue;
      const path = join(dir, name);
      if (!lstatSync(path).isDirectory()) continue;
      for (const sub of name.startsWith("@") ? readdirSync(path).sort().map((inner) => join(name, inner)) : [name]) {
        const folder = join(dir, sub);
        if (!existsSync(join(folder, "package.json"))) continue;
        let manifest;
        try {
          manifest = JSON.parse(readFileSync(join(folder, "package.json"), "utf8"));
        } catch {
          continue;
        }
        let licence = manifest.license ?? manifest.licenses;
        if (Array.isArray(licence)) licence = licence.map((item) => item.type ?? item).join(" OR ");
        if (licence && typeof licence === "object") licence = licence.type;
        const files = readdirSync(folder).filter((file) => /^(licen[sc]e|copying|notice|unlicense)/iu.test(file) && lstatSync(join(folder, file)).isFile());
        found.push({ path: slashed(join(prefix, sub)), name: manifest.name ?? slashed(sub), version: manifest.version ?? "", licence: typeof licence === "string" && licence.trim() ? licence : null, folder, files });
        if (existsSync(join(folder, "node_modules"))) visit(join(folder, "node_modules"), join(prefix, sub, "node_modules"));
      }
    }
  };
  visit(modules, "");
  return found;
}

function licenceProblems(list) {
  const problems = [];
  for (const item of list) {
    if (!item.licence) problems.push(`no licence stated: ${item.name}@${item.version}`);
    else if (REFUSED_LICENCE.test(item.licence)) problems.push(`a licence the repository refuses (${item.licence}): ${item.name}@${item.version}`);
  }
  return problems;
}

function licenceSummary(list) {
  const count = {};
  for (const item of list) count[item.licence ?? "(none stated)"] = (count[item.licence ?? "(none stated)"] ?? 0) + 1;
  return {
    packages: list.length,
    byLicence: Object.fromEntries(Object.entries(count).sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))),
    otherThanMitIscBsdApache: list.filter((item) => item.licence && !PLAIN_LICENCE.test(item.licence)).map((item) => `${item.name}@${item.version}: ${item.licence}`),
    withoutLicenceFile: list.filter((item) => item.files.length === 0).map((item) => `${item.name}@${item.version}`),
  };
}

function writeLicences(list, out) {
  mkdirSync(out, { recursive: true });
  const parts = [
    "Third-party packages in Intcad's built-in agent",
    "",
    "Each package below is carried as its publisher released it, under the licence it states, except",
    "that @mastra/core is carried without its enterprise-licensed content: the directories named `ee`",
    "and the `ee-*` chunk are removed, and one file written by Intcad stands in that chunk's place.",
    "Where a package ships no licence file, only the licence it states is given.",
    "",
  ];
  for (const item of list) {
    parts.push("=".repeat(80), `${item.name} ${item.version}`, `Licence stated: ${item.licence ?? "(none)"}`, `In: node_modules/${item.path}`, "");
    for (const file of item.files) parts.push(`--- ${file}`, readFileSync(join(item.folder, file), "utf8").trimEnd(), "");
  }
  writeFileSync(join(out, "THIRD-PARTY-LICENSES.txt"), `${parts.join("\n")}\n`);
  cpSync(join(repoRoot, "packaging/mastra/NODE-LICENSE.txt"), join(out, "NODE-LICENSE.txt"));
}

// ---- Secrets and stored sign-ins ---------------------------------------------------------------

/** Shapes of the keys and tokens of the services the agent talks to. A letter or digit before one means it is part of a longer word. */
const SECRET_SHAPES = [
  ["an API key (sk-…)", /(?<![A-Za-z0-9_-])sk-(?:ant-|proj-|or-v1-)?[A-Za-z0-9]{32,}/u],
  ["an Anthropic key or token", /(?<![A-Za-z0-9_-])sk-ant-[A-Za-z0-9_-]{24,}/u],
  ["a Google API key", /(?<![A-Za-z0-9_-])AIza[0-9A-Za-z_-]{35}(?![0-9A-Za-z_-])/u],
  ["a GitHub token", /(?<![A-Za-z0-9_])(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})/u],
  ["an xAI key", /(?<![A-Za-z0-9_-])xai-[A-Za-z0-9]{40,}/u],
  ["an AWS access key id", /(?<![A-Za-z0-9])AKIA[0-9A-Z]{16}(?![0-9A-Z])/u],
  // The header with key material after it. The header alone is how code and manuals name the format.
  ["a private key", /-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----(?:\\[nr]|\s)*[A-Za-z0-9+/=]{40,}/u],
];

/** Files that hold a sign-in or what an agent kept while it ran, by name. None belongs in a package. */
function storedSignIn(path) {
  const parts = path.split("/");
  const name = parts[parts.length - 1].toLowerCase();
  if (["auth.json", ".credentials.json", ".netrc", ".npmrc", ".env"].includes(name)) return "a file that keeps a sign-in";
  if (/^\.env\.(?!example$|sample$|template$)/u.test(name)) return "a file that keeps a sign-in";
  if (/\.(db|db-wal|db-shm|sqlite|sqlite3)$/u.test(name)) return "a database";
  if (name === "settings.json" && !parts.includes(".vscode")) return "a settings file";
  if (parts.some((part) => [".mastracode", ".codex", ".claude", ".aws", ".ssh"].includes(part.toLowerCase()))) return "an agent's or account's own folder";
  return null;
}

/**
 * Places in the agent folder where a finding is known to be something else: the exact path, the
 * kind, and why. A new place is added here; the shapes and names are not loosened for it.
 */
const KNOWN_NOT_SECRET = [];
const known = (path, kind) => KNOWN_NOT_SECRET.some((entry) => entry.path === path && entry.kind === kind);

const CHUNK = 4 * 1024 * 1024;
const OVERLAP = 512;

/** What looks like a secret or a stored sign-in under `root`: `[path, kind]`, never the text. */
export function scanSecrets(root) {
  const findings = [];
  let files = 0;
  const buffer = Buffer.alloc(CHUNK + OVERLAP);
  for (const [path, entry] of entries(root)) {
    if (!entry.isFile()) continue;
    files += 1;
    const kind = storedSignIn(path);
    if (kind && !known(path, kind)) findings.push([path, kind]);
    const seen = new Set();
    const handle = openSync(join(root, path), "r");
    try {
      // Read in pieces that overlap, so a key across a boundary is still seen whole.
      for (let kept = 0; ; ) {
        const read = readSync(handle, buffer, kept, CHUNK, null);
        if (read === 0) break;
        const total = kept + read;
        const text = buffer.latin1Slice(0, total);
        for (const [shape, pattern] of SECRET_SHAPES) if (!seen.has(shape) && pattern.test(text)) seen.add(shape);
        kept = Math.min(OVERLAP, total);
        buffer.copyWithin(0, total - kept, total);
      }
    } finally {
      closeSync(handle);
    }
    for (const shape of seen) if (!known(path, shape)) findings.push([path, shape]);
  }
  return { files, findings };
}

// ---- Checking ----------------------------------------------------------------------------------

function check(dir, targetName, { readOnly = false, pin = readPin(PIN_DIR) } = {}) {
  const target = TARGETS[targetName] ?? fail(`unknown target ${targetName}; known: ${Object.keys(TARGETS).join(", ")}`);
  const problems = [];
  const modules = join(dir, "node_modules");
  for (const needed of [target.node, "agent/main.mjs", "agent.json", "licenses/THIRD-PARTY-LICENSES.txt", "licenses/NODE-LICENSE.txt", "node_modules/@mastra/code-sdk/package.json", "node_modules/@agentclientprotocol/sdk/package.json"]) {
    if (!existsSync(join(dir, needed))) problems.push(`missing: ${needed}`);
  }
  if (problems.length > 0) return { problems };
  const manifest = JSON.parse(readFileSync(join(dir, "agent.json"), "utf8"));
  if (manifest.target !== targetName) problems.push(`the folder was built for ${manifest.target}, not ${targetName}`);
  const enterprise = checkEnterprise(modules);
  problems.push(...enterprise.problems);
  const list = packages(modules);
  problems.push(...licenceProblems(list), ...lockedProblems(list, pin.lock));
  // The agent's session builder follows one file of the SDK; another file means it has to be looked at again.
  const adapted = join(modules, ...pin.adapted.package.split("/"), ...pin.adapted.file.split("/"));
  if (!existsSync(adapted) || createHash("sha256").update(readFileSync(adapted)).digest("hex") !== pin.adapted.sha256) {
    problems.push(`${pin.adapted.package}/${pin.adapted.file} is not the file ${pin.adapted.adaptedIn} was adapted from (${pin.adapted.version}): compare the two and update adapted-from.json`);
  }
  if (readOnly) {
    // Nothing in the folder may be written by the person running the app. (Modes say nothing on Windows.)
    const writable = [];
    if (process.platform !== "win32") for (const [path] of entries(dir)) if (lstatSync(join(dir, path)).mode & 0o222) writable.push(path);
    if (process.platform !== "win32" && lstatSync(dir).mode & 0o222) writable.push(".");
    if (writable.length > 0) problems.push(`${writable.length} entries can be written, such as ${writable[0]}`);
  }
  for (const cut of LEFT_OUT) for (const name of cut.packages) if (existsSync(join(modules, ...name.split("/")))) problems.push(`left-out package present: ${name}`);
  const secrets = scanSecrets(dir);
  problems.push(...secrets.findings.map(([path, kind]) => `looks like ${kind}: ${path}`));
  return { problems, report: { target: targetName, enterprise: enterprise.report, licences: licenceSummary(list), scanned: secrets.files, ...size(dir) } };
}

// ---- Building ----------------------------------------------------------------------------------

function build(targetName, out, nodeProgram, pin = readPin(PIN_DIR)) {
  const target = TARGETS[targetName] ?? fail(`unknown target ${targetName}; known: ${Object.keys(TARGETS).join(", ")}`);
  const onHost = process.platform === target.os && process.arch === target.cpu;
  if (!onHost && !nodeProgram) fail(`--node is needed to build for ${targetName} on ${process.platform} ${process.arch}: this Node is not that system's`);
  const node = nodeProgram ?? process.execPath;
  const pinnedNode = readFileSync(join(repoRoot, ".nvmrc"), "utf8").trim();
  if (onHost) {
    const version = spawnSync(node, ["--version"], { encoding: "utf8" }).stdout?.trim();
    if (version !== `v${pinnedNode}`) fail(`${node} is Node ${version || "(did not run)"}; the repository pins ${pinnedNode} (.nvmrc)`);
  }
  if (existsSync(out)) {
    if (readdirSync(out).length > 0 && !existsSync(join(out, "agent.json"))) fail(`${out} exists and is not a folder this script built`);
    rmSync(out, { recursive: true, force: true });
  }

  const installed = install(target, out, pin);
  for (const name of ["package.json", "package-lock.json"]) rmSync(join(out, name));
  const asInstalled = size(installed);
  const enterprise = removeEnterprise(installed);
  const unloaded = dropUnloaded(installed, target);
  const leftOut = leaveOut(installed, pin.lock);

  mkdirSync(join(out, "agent"), { recursive: true });
  for (const name of readdirSync(join(packageRoot, "agent")).filter((name) => name.endsWith(".mjs"))) cpSync(join(packageRoot, "agent", name), join(out, "agent", name));
  cpSync(node, join(out, target.node));
  if (target.os !== "win32") chmodSync(join(out, target.node), 0o755);

  const list = packages(join(out, "node_modules"));
  writeLicences(list, join(out, "licenses"));
  const version = (name) => JSON.parse(readFileSync(join(out, "node_modules", ...name.split("/"), "package.json"), "utf8")).version;
  const megabytes = (tally) => ({ megabytes: Math.round(tally.bytes / 1e5) / 10, files: tally.files });
  writeFileSync(
    join(out, "agent.json"),
    `${JSON.stringify(
      {
        target: targetName,
        node: onHost ? pinnedNode : `${pinnedNode} (not run on the building system)`,
        sdk: Object.fromEntries(["@mastra/code-sdk", "@mastra/core", "@agentclientprotocol/sdk"].map((name) => [name, version(name)])),
        enterpriseContent: "removed by packages/mastra-runtime/scripts/remove-enterprise.mjs; one stand-in file is in its place",
        leftOut: LEFT_OUT,
        removed: {
          asInstalled: megabytes(asInstalled),
          enterprise: { files: enterprise.files, directories: enterprise.directories },
          sourceMaps: megabytes(unloaded.maps),
          typeDeclarations: megabytes(unloaded.declarations),
          otherSystemsBinaries: megabytes(unloaded.otherSystems),
          leftOutPackages: { ...megabytes(leftOut), packages: leftOut.packages },
        },
        licences: licenceSummary(list),
      },
      null,
      2,
    )}\n`,
  );
  return check(out, targetName, { pin });
}

// ---- Command line ------------------------------------------------------------------------------

function main(args) {
  const [command, ...rest] = args;
  const flag = (name) => {
    const index = rest.indexOf(name);
    return index < 0 ? undefined : rest.splice(index, 2)[1];
  };
  const usage = () => {
    console.error("usage: agent-package.mjs build --target <triple> --out <dir> [--node <program>]\n       agent-package.mjs check --target <triple> [--read-only] <dir>\n       agent-package.mjs scan <dir>...");
    return 2;
  };
  try {
    if (command === "scan") {
      if (rest.length === 0) return usage();
      let total = 0;
      const lines = [];
      for (const dir of rest) {
        const { files, findings } = scanSecrets(resolve(dir));
        total += findings.length;
        lines.push({ scanned: dir, files, findings: findings.map(([path, kind]) => ({ path, kind })) });
      }
      console.log(JSON.stringify({ status: total === 0 ? "none-found" : "failed", count: total, results: lines }));
      return total === 0 ? 0 : 1;
    }
    const target = flag("--target");
    const out = flag("--out");
    const node = flag("--node");
    const readOnly = rest.includes("--read-only") && rest.splice(rest.indexOf("--read-only"), 1).length === 1;
    const pinFlag = flag("--pin");
    const pin = readPin(pinFlag ? resolve(pinFlag) : PIN_DIR);
    if (!target || (command !== "build" && command !== "check")) return usage();
    if (command === "build" ? !out || rest.length > 0 : rest.length !== 1) return usage();
    const result = command === "build" ? build(target, resolve(out), node ? resolve(node) : undefined, pin) : check(resolve(rest[0]), target, { readOnly, pin });
    console.log(JSON.stringify({ status: result.problems.length === 0 ? "ok" : "failed", problemCount: result.problems.length, problems: result.problems.slice(0, 50), ...result.report }));
    return result.problems.length === 0 ? 0 : 1;
  } catch (error) {
    console.error(`error: ${error.message}`);
    return 1;
  }
}

// Run as a program (by whatever path, a linked one too), not when another script imports it.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) process.exit(main(process.argv.slice(2)));
