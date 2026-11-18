#!/usr/bin/env node
/**
 * Take Mastra's enterprise-licensed content out of an installed `node_modules` tree, and check
 * that none is left.
 *
 * `@mastra/core/LICENSE.md` puts everything under a directory named `ee/` under the Mastra
 * Enterprise License. In the published package that content is:
 *   - the directories named `ee` under `dist/`;
 *   - one chunk the build names `dist/ee-<hash>.js` (ESM) and `dist/ee-<hash>.cjs` (CommonJS),
 *     each with a source map;
 *   - the type files `auth/ee.d.ts` and `agent-builder/ee.d.ts`, which only point into those
 *     directories.
 *
 * The directories, the CommonJS chunk, the maps and the type files are deleted. The ESM chunk
 * cannot only be deleted: other files of the package import names from it. It is replaced by a
 * stand-in written by this script, with the same file name (the importers name it) and the same
 * export letters, and with none of that chunk's code: every enterprise feature reads as off, and
 * only asking for fine-grained authorization with a provider configured is refused.
 * The chunk's file name and its letters change with each version of `@mastra/core`, so the
 * stand-in is generated from the real chunk's own `export { name as letter }` list. A name this
 * script has no stand-in for stops it: someone has to decide what that name does when off.
 *
 * Usage:
 *   node remove-enterprise.mjs <node_modules>           remove, then check
 *   node remove-enterprise.mjs --check <node_modules>   check only; changes nothing
 *
 * The check fails (exit 1) when the tree holds no `@mastra/core` at all (a wrong path), or holds:
 * a path component named `ee`; a file named
 * `ee.*` or `ee-*` in a `@mastra` package other than the stand-in, byte for byte as this script
 * writes it; a source map naming a source in an `ee` directory or an `ee-*` chunk; text of the
 * enterprise licence header or of its licence check; or an import from the stand-in of a letter
 * it does not export. What this says about a tree that passes is exactly that, and nothing
 * about anyone having approved it.
 *
 * `require("@mastra/core")` and the other CommonJS entries that load the deleted chunk no longer
 * load. The built-in agent and the SDK are ES modules and do not use them.
 */

import { existsSync, lstatSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

// ---- The stand-in ------------------------------------------------------------------------------

/** What the stand-in is made of. None of it comes from the chunk it replaces. */
const PARTS = {
  off: "const off = () => false;",
  none: "const none = () => undefined;",
  list: "const list = () => [];",
  notHere: "const notHere = () => { throw new Error('Enterprise features are not part of this build.'); };",
  // What asking for the sign-in capabilities gets where no sign-in is set up.
  noCapabilities: "const noCapabilities = async () => ({ enabled: false, login: null });",
  invalid: "const invalid = async () => ({ valid: false });",
  empty: "const empty = Object.freeze({});",
  sameName: "const sameName = name => name;",
  pairName: "const pairName = (agent, tool) => `${agent}:${tool}`;",
  listName: "const listName = (server, tool) => JSON.stringify([server, tool]);",
  // Only refuses when a fine-grained authorization provider was configured; Intcad configures none.
  refuse: "const refuse = async options => { if (options?.fgaProvider) notHere(); };",
  FGADeniedError: "class FGADeniedError extends Error {}",
  // A roles provider under which nobody has a role or a permission.
  StaticRBACProvider:
    "class StaticRBACProvider { async getRoles() { return []; } async hasRole() { return false; } async getPermissions() { return []; } async getPermissionsForRole() { return []; } async hasPermission() { return false; } async hasAllPermissions() { return false; } async hasAnyPermission() { return false; } }",
  // A permission's name from its key: AGENT_BUILDER_EXECUTE reads "agent-builder:execute".
  permissionNames:
    "const permissionNames = new Proxy({}, { get: (_, key) => typeof key === 'string' ? key.toLowerCase().replace(/_([^_]*)$/, ':$1').replaceAll('_', '-') : undefined });",
};

/** Each name the real chunk may export, and the part that stands in for it. */
const STAND_INS = {
  ACTIONS: "empty",
  DEFAULT_ROLES: "empty",
  FGADeniedError: "FGADeniedError",
  MastraFGAPermissions: "permissionNames",
  PERMISSIONS: "empty",
  PERMISSION_PATTERNS: "empty",
  RESOURCES: "empty",
  StaticRBACProvider: "StaticRBACProvider",
  buildCapabilities: "noCapabilities",
  checkFGA: "refuse",
  clearLicenseCache: "none",
  getAgentFGAResourceId: "sameName",
  getAgentToolFGAResourceId: "pairName",
  getDefaultRole: "none",
  getMCPToolFGAResourceId: "listName",
  getSafeLicenseSummary: "none",
  getStandaloneToolFGAResourceId: "sameName",
  getWorkflowFGAResourceId: "sameName",
  hasPermission: "off",
  isAuthenticated: "off",
  isDevEnvironment: "off",
  isEEEnabled: "off",
  isEELicenseValid: "off",
  isFeatureEnabled: "off",
  isLicenseValid: "off",
  isValidPermissionPattern: "off",
  matchesPermission: "off",
  requireFGA: "refuse",
  resolvePermissions: "list",
  resolvePermissionsFromMapping: "list",
  startLicenseValidation: "none",
  validateLicense: "invalid",
  validatePermissions: "off",
  warnIfDevEENeedsLicense: "none",
};

const IDENTIFIER = "[A-Za-z_$][A-Za-z0-9_$]*";

/** The text of the stand-in for these `[name, letter]` pairs. The same pairs give the same bytes. */
export function standIn(version, pairs) {
  const sorted = [...pairs].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  const lines = [
    `// Intcad's stand-in for the enterprise part of @mastra/core ${version}, which this build does not carry.`,
    "// Written by packages/mastra-runtime/scripts/remove-enterprise.mjs from the names the rest of the",
    "// package imports, with none of that part's code: every enterprise feature reads as off, and",
    "// asking for fine-grained authorization with a provider configured is refused.",
    ...Object.values(PARTS),
    "export {",
    ...sorted.map(([name, letter]) => `  ${STAND_INS[name]} as ${letter}, // ${name}`),
    "};",
    "",
  ];
  return lines.join("\n");
}

/** The `[name, letter]` pairs of a chunk's one `export { … }` statement. */
function exportPairs(text, what) {
  const statements = [...text.matchAll(/^export \{([^}]*)\};?$/gmu)];
  if (statements.length !== 1) throw new Error(`${what}: expected one export statement, found ${statements.length}`);
  return statements[0][1]
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => {
      const match = new RegExp(`^(${IDENTIFIER})(?: as (${IDENTIFIER}))?$`, "u").exec(item);
      if (!match) throw new Error(`${what}: cannot read the export "${item}"`);
      return [match[1], match[2] ?? match[1]];
    });
}

/** The `[name, letter]` pairs a stand-in says it was written for, read back from its own lines. */
function standInPairs(text) {
  return [...text.matchAll(new RegExp(`^  (${IDENTIFIER}) as (${IDENTIFIER}), // (${IDENTIFIER})$`, "gmu"))].map((match) => [match[3], match[2]]);
}

// ---- Walking the tree --------------------------------------------------------------------------

/** Every entry under `root`, as `[path relative to root with "/", Dirent]`. Links are not followed. */
function* entries(root, dir = root) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(dir, entry.name);
    yield [relative(root, path).split(sep).join("/"), entry];
    if (entry.isDirectory() && !entry.isSymbolicLink()) yield* entries(root, path);
  }
}

const isEeName = (name) => name.toLowerCase() === "ee";
const isEeFile = (name) => /^ee[.-]/iu.test(name);
/** Whether a path is inside a package of the `@mastra` scope (a nested copy counts). */
const inMastra = (path) => /(^|\/)@mastra\/[^/]+\//u.test(path);

/** The folders of every installed `@mastra/core`, nested copies too, relative to `root`. */
function cores(root) {
  const found = [];
  for (const [path, entry] of entries(root)) if (entry.isDirectory() && /(^|\/)@mastra\/core$/u.test(path)) found.push(path);
  return found;
}

// ---- Removing ----------------------------------------------------------------------------------

export function removeEnterprise(root) {
  const done = { cores: 0, directories: 0, files: 0, standIns: [] };
  const found = cores(root);
  if (found.length === 0) throw new Error(`no @mastra/core under ${root}`);
  for (const core of found) {
    const dist = join(root, core, "dist");
    const version = JSON.parse(readFileSync(join(root, core, "package.json"), "utf8")).version;
    const chunks = readdirSync(dist).filter(isEeFile);
    const esm = chunks.filter((name) => name.endsWith(".js"));
    if (esm.length !== 1) throw new Error(`${core}: expected one dist/ee-*.js chunk, found ${esm.length} (${chunks.join(", ") || "none"})`);
    const chunkPath = join(dist, esm[0]);
    const real = readFileSync(chunkPath, "utf8");
    // A second run finds the stand-in already in place and leaves it.
    const pairs = standInPairs(real).length > 0 && real === standIn(version, standInPairs(real)) ? standInPairs(real) : exportPairs(real, `${core}/dist/${esm[0]}`);
    const unknown = pairs.filter(([name]) => !Object.hasOwn(STAND_INS, name)).map(([name]) => name);
    if (unknown.length > 0) {
      throw new Error(`${core} ${version} exports names this script has no stand-in for: ${unknown.join(", ")}. Decide what each does when off and add it to STAND_INS.`);
    }
    for (const name of chunks) {
      if (name === esm[0]) continue;
      rmSync(join(dist, name));
      done.files += 1;
    }
    writeFileSync(chunkPath, standIn(version, pairs));
    done.standIns.push(`${core}/dist/${esm[0]}`);
    done.cores += 1;
  }
  // Everything named `ee`, wherever it is: directories first, then the type files that pointed at them.
  const doomed = [];
  for (const [path, entry] of entries(root)) {
    if (isEeName(entry.name)) doomed.push([path, entry.isDirectory() && !entry.isSymbolicLink()]);
    else if (!entry.isDirectory() && inMastra(path) && isEeFile(entry.name) && !done.standIns.includes(path)) doomed.push([path, false]);
  }
  for (const [path, directory] of doomed) {
    if (!existsSync(join(root, path))) continue; // inside a directory already removed
    rmSync(join(root, path), { recursive: directory, force: true });
    if (directory) done.directories += 1;
    else done.files += 1;
  }
  return done;
}

// ---- Checking ----------------------------------------------------------------------------------

/** Text that only the enterprise chunk, its maps and its type files carry. */
const MARKERS = ["@license Mastra Enterprise License", "see ee/LICENSE", "License validation for EE features", "eeTelemetryBridge"];
/** The licence's name may stand in prose (a README, a licence file); nowhere else. */
const NAME = "Mastra Enterprise License";
const isProse = (name) => /\.(md|markdown|txt)$/iu.test(name) || /^(licen[sc]e|notice|copying)/iu.test(name);
const MAX_SCANNED = 256 * 1024 * 1024;

export function checkEnterprise(root) {
  const problems = [];
  const report = { cores: 0, files: 0, maps: 0, standIns: [] };
  const standIns = new Map();
  for (const core of cores(root)) {
    report.cores += 1;
    const dist = join(root, core, "dist");
    const version = JSON.parse(readFileSync(join(root, core, "package.json"), "utf8")).version;
    for (const name of readdirSync(dist).filter(isEeFile)) {
      const path = `${core}/dist/${name}`;
      const text = readFileSync(join(dist, name), "utf8");
      const pairs = standInPairs(text);
      if (name.endsWith(".js") && pairs.length > 0 && text === standIn(version, pairs) && pairs.every(([key]) => Object.hasOwn(STAND_INS, key))) {
        standIns.set(path, new Set(pairs.map(([, letter]) => letter)));
        report.standIns.push(path);
      }
    }
    // Every letter another file of the package imports from the stand-in must be one it exports.
    for (const [path, entry] of entries(join(root, core, "dist"))) {
      if (!entry.isFile() || !/\.(js|mjs)$/u.test(entry.name)) continue;
      const text = readFileSync(join(dist, path), "utf8");
      for (const match of text.matchAll(/(?:import|export)\s*\{([^}]*)\}\s*from\s*["']([^"']*\/ee-[^"'/]+\.js)["']/gu)) {
        const target = join(dirname(join(core, "dist", path)), match[2]).split(sep).join("/");
        const letters = standIns.get(target);
        if (!letters) {
          problems.push(`${core}/dist/${path} imports ${match[2]}, which is not this script's stand-in`);
          continue;
        }
        for (const item of match[1].split(",").map((part) => part.trim()).filter(Boolean)) {
          const letter = item.split(/\s+as\s+/u)[0];
          if (!letters.has(letter)) problems.push(`${core}/dist/${path} imports "${letter}" from the stand-in, which does not export it`);
        }
      }
    }
  }
  for (const [path, entry] of entries(root)) {
    if (isEeName(entry.name)) {
      problems.push(`named ee: ${path}`);
      continue;
    }
    if (entry.isDirectory() || entry.isSymbolicLink()) continue;
    report.files += 1;
    if (inMastra(path) && isEeFile(entry.name) && !standIns.has(path)) {
      problems.push(`an ee file that is not the stand-in: ${path}`);
      continue;
    }
    const full = join(root, path);
    if (lstatSync(full).size > MAX_SCANNED) {
      problems.push(`too large to read: ${path}`);
      continue;
    }
    const bytes = readFileSync(full);
    for (const marker of MARKERS) if (bytes.includes(marker)) problems.push(`enterprise text ("${marker}") in ${path}`);
    if (!isProse(entry.name) && bytes.includes(NAME)) problems.push(`enterprise text ("${NAME}") in ${path}`);
    if (entry.name.endsWith(".map")) {
      report.maps += 1;
      let map;
      try {
        map = JSON.parse(bytes.toString("utf8"));
      } catch {
        continue; // not a source map this can read; the text markers above still applied
      }
      const sources = [...(map.sources ?? []), ...(map.sections ?? []).flatMap((section) => section.map?.sources ?? [])];
      const named = sources.filter((source) => typeof source === "string" && source.split(/[\\/]/u).some((part) => isEeName(part) || isEeFile(part)));
      if (named.length > 0) problems.push(`source map ${path} names enterprise sources: ${named.slice(0, 3).join(", ")}`);
    }
  }
  // A tree without the package is a wrong path, not a clean tree.
  if (report.cores === 0) problems.push("no @mastra/core in this tree: there is nothing here to vouch for");
  else if (report.standIns.length !== report.cores) problems.push(`${report.cores} @mastra/core found but ${report.standIns.length} stand-in(s)`);
  return { report, problems };
}

// ---- Command line ------------------------------------------------------------------------------

function main(args) {
  const checkOnly = args.includes("--check");
  const roots = args.filter((arg) => !arg.startsWith("--"));
  if (roots.length !== 1 || args.some((arg) => arg.startsWith("--") && arg !== "--check")) {
    console.error("usage: remove-enterprise.mjs [--check] <node_modules>");
    return 2;
  }
  const root = roots[0];
  if (!existsSync(root) || !lstatSync(root).isDirectory()) {
    console.error(`error: ${root} is not a directory`);
    return 2;
  }
  let removed = null;
  if (!checkOnly) {
    try {
      removed = removeEnterprise(root);
    } catch (error) {
      console.error(`error: ${error.message}`);
      return 1;
    }
  }
  const { report, problems } = checkEnterprise(root);
  console.log(JSON.stringify({ status: problems.length === 0 ? "none-found" : "failed", root: basename(root), removed, checked: report, problems: problems.slice(0, 50), problemCount: problems.length }));
  return problems.length === 0 ? 0 : 1;
}

// Run as a program (by whatever path, a linked one too), not when another script imports it.
if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) process.exit(main(process.argv.slice(2)));
