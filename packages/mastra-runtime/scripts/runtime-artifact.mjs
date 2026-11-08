#!/usr/bin/env node
/**
 * Build and verify the reviewed desktop Mastra runtime artifact.
 *
 * The artifact is an unpacked Linux x86_64 tree:
 *   node (official 22.23.4 linux-x64 binary), dist/src, the production
 *   node_modules closure with every `ee/` path pruned, the protocol schema
 *   bundle, package metadata, consolidated licence/notice files, and the
 *   inventory.json/runtime.json controls.
 *
 * `--generate` writes packaging/mastra/{inventory.json,LICENSES.txt,NOTICE.txt}
 * and updates the single reviewed policy approval. `--check` rebuilds into a
 * temporary tree and compares it with the committed controls; it does not write
 * to the repository. Both modes require Node 22.23.4 and an installed `npm ci`
 * tree. No package is downloaded here.
 */

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync, chmodSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "../..");
const packageJsonPath = join(packageRoot, "package.json");
const lockPath = join(repoRoot, "package-lock.json");
const policyPath = join(repoRoot, "packaging/mastra/policy.json");
const sourcesPath = join(repoRoot, "packaging/mastra/source-records.json");
const inventoryPath = join(repoRoot, "packaging/mastra/inventory.json");
const licensesPath = join(repoRoot, "packaging/mastra/LICENSES.txt");
const noticePath = join(repoRoot, "packaging/mastra/NOTICE.txt");
const contractManifestPath = join(repoRoot, "schema/mastra-runtime/v1/manifest.json");

const NODE_SOURCE_URL = "https://nodejs.org/dist/v22.23.4/node-v22.23.4-linux-x64.tar.xz";
const NODE_ARCHIVE_DIGEST = "sha256:1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce";
const NODE_EXECUTABLE_DIGEST = "sha256:94efd062ed066c1a0b332e18249859c905e920eeada5df55d24c99b919629a90";
const NPM_SOURCE_URL = "https://registry.npmjs.org/npm/-/npm-11.19.0.tgz";
const NPM_ARCHIVE_DIGEST = "sha256:31e9770f7dc71119a58509353b27917557aaf0ac9b5ef1a0465ee7d8ec67ae75";
const TARGET = "x86_64-unknown-linux-gnu";
const APPROVAL_ID = "mastra-runtime-linux-x64-v1";
const RUNTIME_VERSION = "0.1.0";
const RUNTIME_LICENSE = "MIT OR Apache-2.0";
const RUNTIME_LICENSE_ID = "lic-runtime";
const NODE_LICENSE_ID = "lic-node";
const TYPESCRIPT_LICENSE_ID = "lic-typescript";

const args = process.argv.slice(2);
const flag = (name) => {
  const index = args.indexOf(name);
  return index === -1 ? undefined : args[index + 1];
};
const has = (name) => args.includes(name);
const check = has("--check");
const artifactDir = resolve(flag("--artifact") ?? join(repoRoot, "target/mastra-runtime-artifact"));
const inputsDir = resolve(flag("--inputs") ?? join(repoRoot, "target/mastra-runtime-inputs"));
const nodeBinary = resolve(flag("--node-bin") ?? process.env["SOLIDCRAFT_NODE_BIN"] ?? process.execPath);
const revision = flag("--revision") ?? execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim();
const archiveDigestsPath = flag("--archive-digests");
const committedInventoryPath = resolve(flag("--inventory") ?? inventoryPath);
const requireApproved = has("--require-approved");

const failures = [];
const fail = (message) => {
  failures.push(message);
};
const require = (condition, message) => {
  if (!condition) throw new Error(message);
};
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const digest = (bytes) => `sha256:${sha256(bytes)}`;
const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
const shortId = (prefix, text) => `${prefix}-${createHash("sha256").update(text).digest("hex").slice(0, 20)}`;
const byString = (left, right) => (left < right ? -1 : left > right ? 1 : 0);
const sanitize = (text) => text.toLowerCase().replace(/[^a-z0-9._-]+/gu, "-").replace(/-+/gu, "-").replace(/^-|-$/gu, "").slice(0, 100);
const isAsciiPath = (path) => /^[\x20-\x7e]+$/u.test(path) && !path.includes("\\") && !path.includes(":") && !/[<>"|?*]/u.test(path);
const eeComponent = (path) => path.split("/").some((part) => part.toLowerCase() === "ee");

function listFiles(root) {
  const out = [];
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolute = join(directory, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) out.push(absolute);
    }
  };
  if (existsSync(root)) walk(root);
  return out;
}

function listPackageFiles(root) {
  const out = [];
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.name === "node_modules") continue;
      const absolute = join(directory, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) walk(absolute);
      else if (entry.isFile()) out.push(absolute);
    }
  };
  if (existsSync(root)) walk(root);
  return out;
}

function copyFile(source, destination) {
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
}

function checkPins() {
  require(process.versions.node === "22.23.4", `Node ${process.versions.node}; expected 22.23.4`);
  require(existsSync(nodeBinary), `node binary not found: ${nodeBinary}`);
  const observed = digest(readFileSync(nodeBinary));
  require(observed === NODE_EXECUTABLE_DIGEST, `node binary ${observed}; expected ${NODE_EXECUTABLE_DIGEST} (set --node-bin to the official 22.23.4 linux-x64 binary)`);
  const nvmrc = readFileSync(join(repoRoot, ".nvmrc"), "utf8").trim();
  require(nvmrc === "22.23.4", `.nvmrc is ${nvmrc}`);
  const pkg = readJson(packageJsonPath);
  require(pkg.engines?.node === ">=22.23.4 <23", `engines.node is ${pkg.engines?.node}`);
  require(pkg.version === RUNTIME_VERSION, `runtime version ${pkg.version}`);
  const npmVersion = execFileSync("npm", ["--version"], { cwd: repoRoot, encoding: "utf8" }).trim();
  require(npmVersion === "11.19.0", `npm ${npmVersion}; expected 11.19.0`);
}

function lockEntry(name, version) {
  const lock = readJson(lockPath).packages;
  const matches = [];
  for (const [key, entry] of Object.entries(lock)) {
    if (!key.startsWith("node_modules/")) continue;
    if (entry.version !== version) continue;
    const base = key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    const bare = base.includes("/node_modules/") ? base.slice(base.lastIndexOf("node_modules/") + "node_modules/".length) : base;
    if (bare === name) matches.push({ key, entry });
  }
  if (matches.length === 0) return undefined;
  return matches.sort((left, right) => left.key.length - right.key.length)[0].entry;
}

function resolvePhysical(parentPath, name, version) {
  const candidates = [];
  let current = parentPath;
  for (;;) {
    candidates.push(current.length === 0 ? join("node_modules", name) : join(current, "node_modules", name));
    const marker = current.lastIndexOf(`${sep}node_modules${sep}`);
    if (marker === -1) break;
    current = current.slice(0, marker);
  }
  candidates.push(join("node_modules", name));
  for (const candidate of candidates) {
    const manifest = join(repoRoot, candidate, "package.json");
    if (!existsSync(manifest)) continue;
    try {
      const parsed = readJson(manifest);
      // npm aliases keep the installed directory name but the manifest's name is
      // the target package; the exact version and resolved path are the identity.
      if (parsed.version === version) return candidate;
    } catch {
      // Keep looking; a malformed candidate is not the installed package.
    }
  }
  return undefined;
}

function productionTree() {
  const raw = execFileSync("npm", ["ls", "--omit=dev", "--all", "--json"], { cwd: repoRoot, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  const tree = JSON.parse(raw);
  const workspace = tree.dependencies?.["@intgral-ai/solidcraft-mastra-runtime"];
  require(workspace, "workspace package missing from npm ls");
  const packages = new Map();
  const rootNames = Object.keys(workspace.dependencies ?? {}).sort();

  const walk = (deps, parentPath) => {
    for (const [name, node] of Object.entries(deps ?? {})) {
      if (typeof node.version !== "string") continue;
      const physical = resolvePhysical(parentPath, name, node.version);
      require(physical, `cannot locate ${name}@${node.version} under ${parentPath || "."}`);
      const id = `npm-${sanitize(`${name}-${node.version}`)}`;
      const existing = packages.get(id) ?? { id, name, version: node.version, paths: new Set(), childIds: new Set() };
      existing.paths.add(physical);
      packages.set(id, existing);
      const childNames = Object.keys(node.dependencies ?? {}).sort();
      for (const childName of childNames) {
        const child = node.dependencies[childName];
        if (typeof child.version !== "string") continue;
        const childId = `npm-${sanitize(`${childName}-${child.version}`)}`;
        existing.childIds.add(childId);
      }
      walk(node.dependencies ?? {}, physical);
    }
  };
  walk(workspace.dependencies ?? {}, "");
  for (const pkg of packages.values()) {
    pkg.childIds = [...pkg.childIds].sort();
    pkg.paths = [...pkg.paths].sort();
  }
  const rootDependencyIds = rootNames
    .map((name) => {
      const node = workspace.dependencies[name];
      return typeof node?.version === "string" ? `npm-${sanitize(`${name}-${node.version}`)}` : undefined;
    })
    .filter(Boolean)
    .sort();
  return { packages, rootDependencyIds };
}

function licenseExpression(raw, name) {
  let expression = typeof raw === "string" ? raw.trim() : raw?.type;
  if (!expression) throw new Error(`${name}: no license in package.json`);
  if (expression === "(AFL-2.1 OR BSD-3-Clause)") return "BSD-3-Clause";
  if (!["Apache-2.0", "MIT", "MIT OR Apache-2.0", "ISC", "BSD-2-Clause", "BSD-3-Clause", "BlueOak-1.0.0"].includes(expression)) {
    throw new Error(`${name}: unsupported license expression ${expression}`);
  }
  return expression;
}

function rightsHolder(manifest, name) {
  const clean = (value) => String(value).replace(/<[^>]*>/gu, "").trim();
  if (typeof manifest.author === "string" && clean(manifest.author)) return clean(manifest.author);
  if (manifest.author && typeof manifest.author === "object" && clean(manifest.author.name ?? "")) return clean(manifest.author.name);
  if (Array.isArray(manifest.maintainers) && manifest.maintainers.length > 0) {
    const first = manifest.maintainers[0];
    const value = typeof first === "string" ? first : first?.name;
    if (value && clean(value)) return clean(value);
  }
  const overrides = {
    "@mastra/core": "Mastra AI and contributors",
    "@mastra/schema-compat": "Mastra AI and contributors",
    "@intgral-ai/solidcraft-mastra-runtime": "SolidCraft contributors",
    zod: "Colin McDonnell and contributors",
    ajv: "Evgeny Poberezkin and contributors",
  };
  return overrides[name] ?? "unstated in package.json";
}

function licenseFileText(packagePath, manifest) {
  const directory = join(repoRoot, packagePath);
  const candidates = readdirSync(directory)
    .filter((name) => /^(license|licence|copying|notice)/iu.test(name))
    .sort()
    .map((name) => join(directory, name))
    .filter((path) => statSync(path).isFile());
  if (candidates.length === 0) return undefined;
  return candidates.map((path) => readFileSync(path)).join(Buffer.from("\n\n"));
}

function buildDependencyData() {
  const lock = readJson(lockPath);
  const { packages, rootDependencyIds } = productionTree();
  const archiveDigests = archiveDigestsPath && existsSync(archiveDigestsPath) ? readJson(archiveDigestsPath) : {};
  const archiveByResolved = new Map(Object.values(archiveDigests).map((entry) => [`${entry.resolved}|${entry.integrity}`, entry.sha256]));
  const committed = existsSync(committedInventoryPath) ? readJson(committedInventoryPath) : undefined;
  const committedDigests = new Map((committed?.dependencies ?? []).map((dep) => [`${dep.name}@${dep.version}`, dep.archiveDigest]));
  const dependencies = [];
  const licenses = [];
  for (const pkg of [...packages.values()].sort((left, right) => byString(left.id, right.id))) {
    const entry = packages.get(pkg.id);
    const lockPackage = lockEntry(pkg.name, pkg.version);
    if (!lockPackage) throw new Error(`${pkg.name}@${pkg.version}: missing package-lock entry`);
    const manifest = readJson(join(repoRoot, pkg.paths[0], "package.json"));
    const expression = licenseExpression(lockPackage.license ?? manifest.license, `${pkg.name}@${pkg.version}`);
    const archiveEntry = archiveDigests[`${pkg.name}@${pkg.version}`]
      ?? archiveByResolved.get(`${lockPackage.resolved}|${lockPackage.integrity}`)
      ?? committedDigests.get(`${pkg.name}@${pkg.version}`);
    const archive = typeof archiveEntry === "string" ? archiveEntry : archiveEntry?.sha256;
    if (!archive) throw new Error(`${pkg.name}@${pkg.version}: no archive digest; pass --archive-digests`);
    const sourceUrl = lockPackage.resolved;
    require(typeof sourceUrl === "string" && sourceUrl.startsWith("https://"), `${pkg.name}@${pkg.version}: missing https resolved URL`);
    dependencies.push({
      id: pkg.id,
      name: pkg.name,
      version: pkg.version,
      sourceUrl,
      archiveDigest: `sha256:${String(archive).replace(/^sha256:/u, "")}`,
      lockIntegrity: lockPackage.integrity,
      dependencyIds: pkg.childIds,
    });
    const licenceText = licenseFileText(pkg.paths[0], manifest);
    licenses.push({
      id: `lic-${sanitize(`${pkg.name}-${pkg.version}`)}`,
      expression,
      rightsHolder: rightsHolder(manifest, pkg.name),
      verdict: licenceText ? "verified" : "needs-human",
      reason: licenceText
        ? "SPDX expression declared by the package; its licence file is retained in LICENSES.txt."
        : "The published package has no separate licence file; its permissive SPDX declaration needs legal review.",
      licenseFiles: [],
      noticeFiles: [],
    });
  }
  licenses.push({
    id: "lic-mastra-enterprise",
    expression: "LicenseRef-Mastra-Enterprise",
    rightsHolder: "Mastra AI and contributors",
    verdict: "needs-human",
    reason: "Mastra Enterprise License header and src/ee/index.ts source-map provenance; the contract excludes enterprise modules and no distribution approval exists.",
    licenseFiles: [],
    noticeFiles: [],
  });
  licenses.sort((left, right) => byString(left.id, right.id));
  return { dependencies, licenses, rootDependencyIds, packages };
}

function expectedPrunedMastraEe(output) {
  return (
    /^node_modules\/@mastra\/core\/dist\/(?:auth|agent-builder)\/ee\//u.test(output) ||
    /^node_modules\/@mastra\/core\/dist\/_types\/[^/]+\/dist\/ee\//u.test(output)
  );
}

function artifactPlan(dependencyData) {
  const plan = [];
  // Third-party production packages, pruned of every `ee` path component.
  for (const pkg of dependencyData.packages.values()) {
    for (const path of pkg.paths) {
      for (const absolute of listPackageFiles(join(repoRoot, path))) {
        const output = relative(repoRoot, absolute).split(sep).join("/");
        if (eeComponent(output)) {
          if (pkg.name === "@mastra/core" && expectedPrunedMastraEe(output)) continue;
          throw new Error(`unexpected ee path in shipped package: ${output}`);
        }
        const enterprise = pkg.name === "@mastra/core" && /\/ee-[^/]+$/u.test(output);
        plan.push({
          source: absolute,
          output,
          kind: "dependency",
          originId: pkg.id,
          licenseId: enterprise ? "lic-mastra-enterprise" : `lic-${sanitize(`${pkg.name}-${pkg.version}`)}`,
        });
      }
    }
  }
  // Protocol bundle (repository input).
  for (const absolute of listFiles(join(repoRoot, "schema/mastra-runtime/v1"))) {
    const output = relative(repoRoot, absolute).split(sep).join("/");
    plan.push({ source: absolute, output, kind: "repository", originalPath: output, licenseId: RUNTIME_LICENSE_ID });
  }
  // Built runtime entry points (build evidence + TypeScript tool).
  for (const absolute of listFiles(join(packageRoot, "dist/src"))) {
    const output = relative(packageRoot, absolute).split(sep).join("/");
    plan.push({ source: absolute, output, kind: "built", licenseId: RUNTIME_LICENSE_ID });
  }
  // Package metadata shipped with the runtime.
  for (const name of ["package.json", "README.md", "DEPENDENCIES.md"]) {
    const absolute = join(packageRoot, name);
    if (existsSync(absolute)) plan.push({ source: absolute, output: name, kind: "repository", originalPath: `packages/mastra-runtime/${name}`, licenseId: RUNTIME_LICENSE_ID });
  }
  // Pinned Node executable.
  plan.push({ source: nodeBinary, output: "node", kind: "node", licenseId: NODE_LICENSE_ID, mode: 0o755 });
  plan.sort((left, right) => byString(left.output, right.output));
  return plan;
}

function enterpriseFilePaths(dependencyData) {
  const mastra = [...dependencyData.packages.values()].find((pkg) => pkg.name === "@mastra/core");
  if (!mastra) return [];
  const paths = [];
  for (const path of mastra.paths) {
    for (const absolute of listPackageFiles(join(repoRoot, path))) {
      const output = relative(repoRoot, absolute).split(sep).join("/");
      if (/\/ee-[^/]+$/u.test(output)) paths.push(output);
    }
  }
  return paths.sort(byString);
}

function renderNotices(dependencyData) {
  const licenseTexts = new Map();
  for (const pkg of dependencyData.packages.values()) {
    const manifest = readJson(join(repoRoot, pkg.paths[0], "package.json"));
    const text = licenseFileText(pkg.paths[0], manifest);
    if (text) licenseTexts.set(pkg.id, text);
  }
  const licenseOf = (pkg) => dependencyData.licenses.find((item) => item.id === `lic-${sanitize(`${pkg.name}-${pkg.version}`)}`);
  const expressions = [...new Set(dependencyData.licenses.map((license) => license.expression))].sort();
  const chunks = [];
  chunks.push("SolidCraft Mastra runtime — third-party licence texts");
  chunks.push("Pinned build; see packaging/mastra/inventory.json for exact files and digests.");
  chunks.push("This file is a reviewed distribution record, not a legal sign-off.");
  chunks.push("");
  for (const expression of expressions) {
    chunks.push(`# ${expression}`);
    chunks.push("");
    if (expression === "LicenseRef-Mastra-Enterprise") {
      chunks.push("Mastra Enterprise License content is NOT approved for this distribution. These files are");
      chunks.push("recorded as needs-human legal review; the current public @mastra/core/agent entry imports");
      chunks.push("them, so the artifact is blocked until legal review or a different build resolves them.");
      chunks.push("");
      for (const path of enterpriseFilePaths(dependencyData)) {
        chunks.push(`- ${path}`);
      }
      chunks.push("");
      continue;
    }
    const sample = [...dependencyData.packages.values()].find((pkg) => licenseOf(pkg)?.expression === expression);
    if (sample) {
      const text = licenseTexts.get(sample.id);
      chunks.push(text ? text.toString("utf8") : "The published package does not include a separate licence file; the SPDX identifier above applies.");
    }
    chunks.push("");
    const packages = [...dependencyData.packages.values()]
      .filter((pkg) => licenseOf(pkg)?.expression === expression)
      .sort((left, right) => byString(left.name, right.name) || byString(left.version, right.version));
    for (const pkg of packages) {
      const license = licenseOf(pkg);
      const text = licenseTexts.get(pkg.id);
      chunks.push(`## ${pkg.name}@${pkg.version}`);
      chunks.push(`Rights holder: ${license.rightsHolder}`);
      chunks.push(`Source: ${dependencyData.dependencies.find((dep) => dep.id === pkg.id)?.sourceUrl ?? "unstated"}`);
      chunks.push(`Review: ${license.verdict} — ${license.reason}`);
      chunks.push(text ? text.toString("utf8") : "No separate licence file in the published package; the SPDX identifier above applies.");
      chunks.push("");
    }
  }
  const typescriptDir = join(repoRoot, "node_modules/typescript");
  chunks.push("# TypeScript 5.9.3 (build tool, not shipped)");
  const typescriptText = licenseFileText("node_modules/typescript", readJson(join(typescriptDir, "package.json")));
  chunks.push(typescriptText ? typescriptText.toString("utf8") : "The published TypeScript package does not include a separate licence file.");
  chunks.push("");
  chunks.push("# Node.js 22.23.4");
  chunks.push(readFileSync(join(repoRoot, "packaging/mastra/NODE-LICENSE.txt"), "utf8"));
  chunks.push("");
  chunks.push("# SolidCraft runtime (MIT OR Apache-2.0)");
  chunks.push(readFileSync(join(repoRoot, "LICENSE-MIT"), "utf8"));
  chunks.push(readFileSync(join(repoRoot, "LICENSE-APACHE"), "utf8"));
  const licenses = Buffer.from(`${chunks.join("\n")}\n`);

  const notice = [];
  notice.push("SolidCraft Mastra runtime — third-party notices");
  notice.push(`Target: ${TARGET}; Node 22.23.4; npm 11.19.0; runtime ${RUNTIME_VERSION}.`);
  notice.push("The artifact contains no path component named `ee`. However, the shipped `@mastra/core/dist/ee-*.js`/`.cjs`/`.map` files carry Mastra Enterprise License headers and `src/ee/index.ts` source-map provenance. They are NOT approved for distribution, are recorded as needs-human legal review, and the current public `@mastra/core/agent` entry imports them. The artifact is blocked; #14 must not bundle it until legal review or a different build resolves this.");
  notice.push("");
  notice.push("| Package | Version | Licence | Rights holder | Source |");
  notice.push("|---|---|---|---|---|");
  for (const dep of [...dependencyData.dependencies].sort((left, right) => byString(left.name, right.name) || byString(left.version, right.version))) {
    const license = dependencyData.licenses.find((item) => item.id === `lic-${sanitize(`${dep.name}-${dep.version}`)}`);
    notice.push(`| \`${dep.name}\` | ${dep.version} | ${license.expression} | ${license.rightsHolder} | ${dep.sourceUrl} |`);
  }
  notice.push(`| \`node\` | 22.23.4 | MIT | OpenJS Foundation and Node.js contributors | ${NODE_SOURCE_URL} |`);
  notice.push(`| \`@intgral-ai/solidcraft-mastra-runtime\` | ${RUNTIME_VERSION} | MIT OR Apache-2.0 | SolidCraft contributors | https://github.com/intgral-ai/open-skyfall-desktop |`);
  notice.push(`| \`typescript\` | 5.9.3 | Apache-2.0 | Microsoft and contributors | build tool only, not shipped |`);
  notice.push("");
  notice.push("Enterprise-license files requiring legal review:");
  for (const path of enterpriseFilePaths(dependencyData)) notice.push(`- \`${path}\``);
  return { licenses, notice: Buffer.from(`${notice.join("\n")}\n`) };
}

function npmVersionText() {
  return "11.19.0";
}

function build() {
  checkPins();
  const dependencyData = buildDependencyData();
  const noticeTexts = renderNotices(dependencyData);
  if (check) {
    require(existsSync(licensesPath) && readFileSync(licensesPath).equals(noticeTexts.licenses), "packaging/mastra/LICENSES.txt is stale; run --generate");
    require(existsSync(noticePath) && readFileSync(noticePath).equals(noticeTexts.notice), "packaging/mastra/NOTICE.txt is stale; run --generate");
  } else {
    writeFileSync(licensesPath, noticeTexts.licenses);
    writeFileSync(noticePath, noticeTexts.notice);
  }
  rmSync(artifactDir, { recursive: true, force: true });
  rmSync(inputsDir, { recursive: true, force: true });
  mkdirSync(artifactDir, { recursive: true });
  mkdirSync(inputsDir, { recursive: true });

  const plan = artifactPlan(dependencyData);
  plan.push({ source: licensesPath, output: "licenses/LICENSES.txt", kind: "repository", originalPath: "packaging/mastra/LICENSES.txt", licenseId: "ALL" });
  plan.push({ source: noticePath, output: "notices/NOTICE.txt", kind: "repository", originalPath: "packaging/mastra/NOTICE.txt", licenseId: "ALL" });
  plan.sort((left, right) => byString(left.output, right.output));
  const inputs = [];
  const inputByMaterialized = new Map();
  const outputs = [];
  const derivations = [];
  const licenseFileDigest = digest(noticeTexts.licenses);
  const noticeFileDigest = digest(noticeTexts.notice);
  const allLicenseIds = [...dependencyData.licenses.map((license) => license.id), RUNTIME_LICENSE_ID, NODE_LICENSE_ID, TYPESCRIPT_LICENSE_ID].sort();

  const addInput = (spec) => {
    if (inputByMaterialized.has(spec.materializedPath)) return inputByMaterialized.get(spec.materializedPath);
    const bytes = readFileSync(spec.source);
    const input = {
      id: spec.id ?? shortId("in", spec.materializedPath),
      materializedPath: spec.materializedPath,
      originKind: spec.originKind,
      originId: spec.originId,
      originalPath: spec.originalPath,
      sourceRevision: spec.sourceRevision ?? null,
      digest: digest(bytes),
      sizeBytes: bytes.length,
      licenseIds: spec.licenseIds ?? [spec.licenseId],
      sourceRecordId: null,
    };
    inputByMaterialized.set(spec.materializedPath, input);
    inputs.push(input);
    copyFile(spec.source, join(inputsDir, spec.materializedPath));
    return input;
  };

  for (const item of plan) {
    const bytes = readFileSync(item.source);
    copyFile(item.source, join(artifactDir, item.output));
    if (item.mode !== undefined) chmodSync(join(artifactDir, item.output), item.mode);
    let outputLicenseIds = item.licenseId === "ALL" ? allLicenseIds : [item.licenseId];
    const output = { path: item.output, digest: digest(bytes), sizeBytes: bytes.length, role: item.output === "node" ? "node" : item.output === "licenses/LICENSES.txt" ? "license" : item.output === "notices/NOTICE.txt" ? "notice" : "code", licenseIds: outputLicenseIds };
    outputs.push(output);
    if (item.kind === "built") {
      const sourceRelative = item.output.replace(/^dist\//u, "").replace(/\.d\.ts$/u, ".ts").replace(/\.js$/u, ".ts");
      const sourcePath = join(packageRoot, sourceRelative);
      require(existsSync(sourcePath), `${item.output}: missing TypeScript source ${sourceRelative}`);
      const source = addInput({ source: sourcePath, materializedPath: `sources/${sourceRelative}`, originKind: "repository", originId: shortId("in", `sources/${sourceRelative}`), originalPath: `packages/mastra-runtime/${sourceRelative}`, licenseId: RUNTIME_LICENSE_ID });
      const manifest = addInput({ source: packageJsonPath, materializedPath: "sources/package.json", originKind: "repository", originId: shortId("in", "sources/package.json"), originalPath: "packages/mastra-runtime/package.json", licenseId: RUNTIME_LICENSE_ID });
      const tsconfig = addInput({ source: join(packageRoot, "tsconfig.json"), materializedPath: "sources/tsconfig.json", originKind: "repository", originId: shortId("in", "sources/tsconfig.json"), originalPath: "packages/mastra-runtime/tsconfig.json", licenseId: RUNTIME_LICENSE_ID });
      const evidencePath = `evidence/${item.output}.json`;
      const evidenceBytes = Buffer.from(`${JSON.stringify({ schemaVersion: "1.0", outputPath: item.output, inputIds: [source.id, manifest.id, tsconfig.id].sort(), toolInputIds: ["tool-typescript"] })}\n`);
      const evidenceSource = join(inputsDir, evidencePath);
      mkdirSync(dirname(evidenceSource), { recursive: true });
      writeFileSync(evidenceSource, evidenceBytes);
      const evidence = addInput({ source: evidenceSource, materializedPath: evidencePath, originKind: "build-evidence", originId: shortId("in", evidencePath), originalPath: evidencePath, licenseId: RUNTIME_LICENSE_ID });
      derivations.push({ outputPath: item.output, inputIds: [source.id, manifest.id, tsconfig.id].sort(), toolInputIds: ["tool-typescript"], evidenceInputId: evidence.id });
    } else if (item.kind === "node") {
      const input = addInput({ source: item.source, materializedPath: "node", originKind: "node", originId: "node", originalPath: "node", licenseId: NODE_LICENSE_ID });
      derivations.push({ outputPath: item.output, inputIds: [input.id], toolInputIds: [], evidenceInputId: null });
    } else {
      const input = addInput({
        source: item.source,
        materializedPath: item.output,
        originKind: item.kind === "dependency" ? "dependency" : "repository",
        originId: item.kind === "dependency" ? item.originId : shortId("in", item.output),
        originalPath: item.kind === "repository" ? item.originalPath : item.output,
        licenseId: item.licenseId === "ALL" ? RUNTIME_LICENSE_ID : item.licenseId,
        licenseIds: item.licenseId === "ALL" ? allLicenseIds : undefined,
      });
      derivations.push({ outputPath: item.output, inputIds: [input.id], toolInputIds: [], evidenceInputId: null });
    }
  }

  // The consolidated licence/notice files are referenced by every licence id.
  const fileRef = (path, digestValue) => ({ path, digest: digestValue });
  const licenses = [
    ...dependencyData.licenses.map((license) => ({ ...license, licenseFiles: [fileRef("licenses/LICENSES.txt", licenseFileDigest)], noticeFiles: [fileRef("notices/NOTICE.txt", noticeFileDigest)] })),
    { id: RUNTIME_LICENSE_ID, expression: RUNTIME_LICENSE, rightsHolder: "SolidCraft contributors", verdict: "verified", reason: "Original SolidCraft runtime code under the project dual licence.", licenseFiles: [fileRef("licenses/LICENSES.txt", licenseFileDigest)], noticeFiles: [fileRef("notices/NOTICE.txt", noticeFileDigest)] },
    { id: NODE_LICENSE_ID, expression: "MIT", rightsHolder: "OpenJS Foundation and Node.js contributors", verdict: "verified", reason: "Official Node.js linux-x64 distribution under its MIT licence.", licenseFiles: [fileRef("licenses/LICENSES.txt", licenseFileDigest)], noticeFiles: [fileRef("notices/NOTICE.txt", noticeFileDigest)] },
    { id: TYPESCRIPT_LICENSE_ID, expression: "Apache-2.0", rightsHolder: "Microsoft and contributors", verdict: "verified", reason: "TypeScript build tool, not shipped, under Apache-2.0.", licenseFiles: [fileRef("licenses/LICENSES.txt", licenseFileDigest)], noticeFiles: [fileRef("notices/NOTICE.txt", noticeFileDigest)] },
  ].sort((left, right) => byString(left.id, right.id));

  // Tool input for the TypeScript compiler.
  const typescriptManifest = join(repoRoot, "node_modules/typescript/package.json");
  require(existsSync(typescriptManifest), "typescript is not installed; run npm ci");
  addInput({ id: "tool-typescript", source: typescriptManifest, materializedPath: "tools/typescript.json", originKind: "tool", originId: "typescript", originalPath: "tools/typescript.json", licenseId: TYPESCRIPT_LICENSE_ID });

  const typescriptArchive = archiveDigestsPath ? readJson(archiveDigestsPath)["typescript@5.9.3"]?.sha256 : undefined;
  const committedBuildTool = existsSync(committedInventoryPath)
    ? readJson(committedInventoryPath).buildTools?.find((tool) => tool.id === "typescript")?.archiveDigest
    : undefined;
  const typescriptDigestValue = typescriptArchive ? `sha256:${String(typescriptArchive).replace(/^sha256:/u, "")}` : committedBuildTool;
  require(typeof typescriptDigestValue === "string" && /^sha256:[0-9a-f]{64}$/u.test(typescriptDigestValue), "typescript archive digest is unavailable; pass --archive-digests");
  const buildTools = [
    {
      id: "typescript",
      name: "typescript",
      version: readJson(typescriptManifest).version,
      sourceUrl: readJson(lockPath).packages["node_modules/typescript"].resolved,
      archiveDigest: typescriptDigestValue,
      lockIntegrity: readJson(lockPath).packages["node_modules/typescript"].integrity,
      dependencyIds: [],
    },
  ];

  outputs.sort((left, right) => byString(left.path, right.path));
  inputs.sort((left, right) => byString(left.id, right.id));
  derivations.sort((left, right) => byString(left.outputPath, right.outputPath));
  const inventory = {
    schemaVersion: "1.0",
    artifactId: "solidcraft-mastra-runtime-linux-x64",
    target: TARGET,
    runtimeVersion: RUNTIME_VERSION,
    protocolVersions: ["1.0"],
    node: { version: "22.23.4", sourceUrl: NODE_SOURCE_URL, archiveDigest: NODE_ARCHIVE_DIGEST, executablePath: "node" },
    packageManager: { name: "npm", version: "11.19.0", sourceUrl: NPM_SOURCE_URL, archiveDigest: NPM_ARCHIVE_DIGEST },
    lockfile: { path: "package-lock.json", digest: digest(readFileSync(lockPath)) },
    contractManifest: { path: "schema/mastra-runtime/v1/manifest.json", digest: digest(readFileSync(contractManifestPath)) },
    rootDependencyIds: dependencyData.rootDependencyIds,
    dependencies: dependencyData.dependencies,
    rootToolIds: ["typescript"],
    buildTools,
    licenses,
    inputs,
    outputs,
    derivations,
  };
  const inventoryBytes = Buffer.from(`${JSON.stringify(inventory, null, 2)}\n`);
  require(inventoryBytes.length < 16 * 1024 * 1024, `inventory is ${inventoryBytes.length} bytes; over 16 MiB`);
  writeFileSync(join(artifactDir, "inventory.json"), inventoryBytes);
  if (!check) writeFileSync(inventoryPath, inventoryBytes);

  const policy = readJson(policyPath);
  policy.approvals = [];
  if (!check) writeJson(policyPath, policy);
  const policyBytes = readFileSync(policyPath);
  const runtime = {
    schemaVersion: "1.0",
    runtimeVersion: RUNTIME_VERSION,
    protocolVersions: ["1.0"],
    targetTriple: TARGET,
    nodeVersion: "22.23.4",
    repositoryRevision: revision,
    contractDigest: digest(readFileSync(contractManifestPath)),
    policyDigest: digest(policyBytes),
    sourceRecordsDigest: digest(readFileSync(sourcesPath)),
    provenanceInventory: { path: "inventory.json", digest: digest(inventoryBytes) },
    artifactDigests: outputs.map((output) => ({ path: output.path, digest: output.digest })),
  };
  writeJson(join(artifactDir, "runtime.json"), runtime);
  return { inventory, inventoryBytes, runtime, outputs, inputs, dependencies: dependencyData.dependencies };
}

function enterpriseContent(bytes) {
  return ["Mastra Enterprise License", "@license Mastra Enterprise License", "src/ee/"].some((marker) => bytes.includes(Buffer.from(marker)));
}

function compare() {
  require(existsSync(committedInventoryPath), "packaging/mastra/inventory.json is missing; run --generate");
  const committed = readJson(committedInventoryPath);
  const built = build();
  const generated = JSON.parse(built.inventoryBytes.toString("utf8"));
  const committedText = JSON.stringify(committed);
  const generatedText = JSON.stringify(generated);
  if (committedText !== generatedText) {
    const keys = new Set([...Object.keys(committed), ...Object.keys(generated)]);
    for (const key of keys) {
      if (JSON.stringify(committed[key]) !== JSON.stringify(generated[key])) {
        let detail = "";
        if (Array.isArray(committed[key]) && Array.isArray(generated[key])) {
          const length = Math.max(committed[key].length, generated[key].length);
          for (let index = 0; index < length; index += 1) {
            if (JSON.stringify(committed[key][index]) !== JSON.stringify(generated[key][index])) {
              const committedItem = committed[key][index];
              const generatedItem = generated[key][index];
              detail = `; first difference at index ${index}: committed=${JSON.stringify(committedItem)} generated=${JSON.stringify(generatedItem)}`;
              break;
            }
          }
        }
        throw new Error(`inventory drift at ${key}: rebuild does not match packaging/mastra/inventory.json${detail}`);
      }
    }
    throw new Error("inventory drift: rebuild does not match packaging/mastra/inventory.json");
  }
  const policy = readJson(policyPath);
  const ee = listFiles(artifactDir).map((path) => relative(artifactDir, path).split(sep).join("/")).filter(eeComponent);
  require(ee.length === 0, `artifact still contains ee paths: ${ee.slice(0, 5).join(", ")}`);
  const outputs = new Map(committed.outputs.map((output) => [output.path, output]));
  const enterpriseFiles = listFiles(artifactDir)
    .map((path) => relative(artifactDir, path).split(sep).join("/"))
    .filter((path) => path.startsWith("node_modules/@mastra/core/") && [".js", ".cjs", ".mjs", ".map"].some((extension) => path.endsWith(extension)))
    .filter((path) => enterpriseContent(readFileSync(join(artifactDir, path))));
  const licenses = new Map(committed.licenses.map((license) => [license.id, license]));
  const enterpriseLicense = licenses.get("lic-mastra-enterprise");
  const evidenceErrors = [];
  if (enterpriseFiles.length > 0 && (!enterpriseLicense || enterpriseLicense.verdict !== "needs-human")) {
    evidenceErrors.push("enterprise-license content is present but lic-mastra-enterprise is not recorded needs-human");
  }
  for (const path of enterpriseFiles) {
    const output = outputs.get(path);
    if (!output || !output.licenseIds.includes("lic-mastra-enterprise")) {
      evidenceErrors.push(`enterprise-license content is not recorded as needs-human: ${path}`);
    }
  }
  const unapprovedLicenses = committed.licenses.filter((license) => license.verdict !== "verified").map((license) => license.id);
  const inventoryDigest = digest(readFileSync(committedInventoryPath));
  const approvals = policy.approvals ?? [];
  const approval = approvals.find((item) => item.id === APPROVAL_ID && item.target === TARGET && item.inventoryDigest === inventoryDigest);
  const approvalBlockers = [];
  if (!approval) approvalBlockers.push("policy.approvals is empty or does not bind this inventory");
  if (unapprovedLicenses.length > 0) approvalBlockers.push(`licences need legal review: ${unapprovedLicenses.join(", ")}`);
  if (enterpriseFiles.length > 0) approvalBlockers.push(`enterprise-license content: ${enterpriseFiles.join(", ")}`);
  const artifactBytes = built.outputs.reduce((total, output) => total + output.sizeBytes, 0) + readFileSync(join(artifactDir, "inventory.json")).length + readFileSync(join(artifactDir, "runtime.json")).length;
  const report = {
    mode: "check",
    target: TARGET,
    dependencyCount: built.dependencies.length,
    outputCount: built.outputs.length,
    inputCount: built.inputs.length,
    inventoryDigest,
    artifactFiles: built.outputs.length + 2,
    artifactBytes,
    eePaths: 0,
  };
  if (evidenceErrors.length > 0) {
    console.log(JSON.stringify({ ...report, status: "failed", errors: evidenceErrors }));
    process.exitCode = 1;
    return;
  }
  if (requireApproved && (!approval || approvalBlockers.length > 0)) {
    console.log(JSON.stringify({ ...report, status: "unapproved", blockers: approvalBlockers }));
    process.exitCode = 1;
    return;
  }
  if (!approval || approvalBlockers.length > 0) {
    console.log(JSON.stringify({ ...report, status: "unapproved", blockers: approvalBlockers }));
    return;
  }
  console.log(JSON.stringify({ ...report, status: "verified" }));
}

try {
  if (check) compare();
  else {
    const built = build();
    console.log(JSON.stringify({
      mode: "generate",
      status: "written",
      target: TARGET,
      dependencyCount: built.dependencies.length,
      outputCount: built.outputs.length,
      inputCount: built.inputs.length,
      inventoryDigest: digest(built.inventoryBytes),
      artifactDir,
      inputsDir,
    }));
  }
} catch (error) {
  console.error(`runtime-artifact: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
