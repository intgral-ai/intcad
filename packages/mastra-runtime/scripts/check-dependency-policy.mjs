#!/usr/bin/env node
/**
 * Locked dependency and enterprise-module check for the SolidCraft workflow runtime.
 *
 * Cargo's licence gate does not cover npm/Node. This script reports the direct
 * production closure and proves the runtime source and built artifact import no
 * Mastra SDK or enterprise module. The full lockfile remains the inventory for
 * #15/BM-3; this is not a substitute for that review.
 */

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "../..");
const failures = [];

function fail(message) {
  failures.push(message);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

const pkg = readJson(join(packageRoot, "package.json"));
const lock = readJson(join(repoRoot, "package-lock.json"));
const nvmrc = readFileSync(join(repoRoot, ".nvmrc"), "utf8").trim();
if (nvmrc !== "22.23.4") {
  fail(`.nvmrc is ${nvmrc}, expected 22.23.4`);
}
if (pkg.engines?.node !== ">=22.23.4 <23") {
  fail(`package engines.node is ${pkg.engines?.node}`);
}
if (pkg.private !== true) {
  fail("the runtime package must stay private");
}
if (lock.packages?.["packages/mastra-runtime"]?.version !== pkg.version) {
  fail("package-lock does not record the workspace version");
}

const direct = Object.keys(pkg.dependencies ?? {}).sort();
for (const name of direct) {
  const locked = lock.packages?.[`node_modules/${name}`]?.version;
  if (locked !== pkg.dependencies[name]) {
    fail(`${name} is declared ${pkg.dependencies[name]} but locked as ${locked}`);
  }
  const manifestPath = join(repoRoot, "node_modules", name, "package.json");
  if (!existsSync(manifestPath)) {
    fail(`${name} is not installed; run npm ci first`);
    continue;
  }
  const manifest = readJson(manifestPath);
  console.log(`${name}@${manifest.version} license=${manifest.license ?? "unknown"}`);
}

for (const name of Object.keys(lock.packages ?? {})) {
  if (name.includes("@mastra/")) {
    fail(`lockfile contains ${name}; the Mastra SDK must not be a runtime dependency`);
  }
}

function files(directory, out = []) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const absolute = join(directory, entry.name);
    if (entry.isDirectory()) {
      files(absolute, out);
    } else if (entry.isFile()) {
      out.push(absolute);
    }
  }
  return out;
}

for (const root of [join(packageRoot, "src"), join(packageRoot, "dist", "src")]) {
  if (!existsSync(root)) {
    if (root.includes("dist")) {
      fail("dist/src is missing; run npm run build before check:deps");
    } else {
      fail("src is missing");
    }
    continue;
  }
  for (const file of files(root).filter((path) => path.endsWith(".ts") || path.endsWith(".js"))) {
    const source = readFileSync(file, "utf8");
    if (/@mastra\//u.test(source) || /\/ee(?:\/|["'])/u.test(source)) {
      fail(`${file} imports the Mastra SDK or an enterprise module`);
    }
  }
}

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`FAIL ${failure}`);
  }
  process.exitCode = 1;
} else {
  console.log(`direct production dependencies: ${direct.join(", ")}`);
  console.log("Mastra SDK / enterprise imports: none");
  console.log("lockfile: committed at the repository root; reviewed closure in packaging/mastra/inventory.json");
}
