import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { RUNTIME_VERSION } from "../src/server.js";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const repoRoot = resolve(packageRoot, "../..");

function files(directory: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const absolute = join(directory, entry.name);
    if (entry.isDirectory()) {
      out.push(...files(absolute));
    } else if (entry.isFile()) {
      out.push(absolute);
    }
  }
  return out;
}

test("the package pins the intended Node line and keeps the lockfile in the repository", () => {
  const pkg = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8")) as {
    version: string;
    engines: { node: string };
    dependencies: Record<string, string>;
    private: boolean;
  };
  const rootPkg = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8")) as {
    packageManager: string;
    dependencies?: Record<string, string>;
  };
  assert.equal(rootPkg.packageManager, "npm@11.19.0");
  assert.equal(rootPkg.dependencies?.["zod"], undefined, "root package must not redeclare a workspace dependency");
  assert.equal(pkg.private, true);
  assert.equal(pkg.engines.node, ">=22.23.4 <23");
  assert.equal(readFileSync(join(repoRoot, ".nvmrc"), "utf8").trim(), "22.23.4");
  assert.equal(pkg.version, RUNTIME_VERSION);
  assert.equal(pkg.dependencies["@mastra/core"], undefined, "the runtime must not depend on the Mastra SDK");
  assert.equal(pkg.dependencies["zod"], undefined, "the legacy Mastra adapter was the only zod user");
  assert.equal(pkg.dependencies["ajv"], "8.20.0");
  assert.ok(existsSync(join(repoRoot, "package-lock.json")), "the repository lockfile is committed");
  const lock = JSON.parse(readFileSync(join(repoRoot, "package-lock.json"), "utf8")) as {
    packages: Record<string, { version?: string; dependencies?: Record<string, string> }>;
  };
  assert.equal(Object.keys(lock.packages).some((name) => name.includes("@mastra/")), false);
  assert.equal(lock.packages["packages/mastra-runtime"]?.version, "0.1.0");
  assert.equal(lock.packages[""]?.dependencies?.["zod"], undefined);
});

test("the built entry point imports only declared production packages and no enterprise module", () => {
  const pkg = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8")) as {
    dependencies: Record<string, string>;
    main: string;
  };
  assert.equal(pkg.main, "dist/src/main.js");
  assert.ok(existsSync(join(packageRoot, pkg.main)), "npm run build produced the entry point");
  const allowed = new Set(["ajv"]);
  for (const file of files(join(packageRoot, "dist", "src")).filter((path) => path.endsWith(".js"))) {
    const source = readFileSync(file, "utf8");
    assert.ok(!/@mastra\//u.test(source), `${file} imports the Mastra SDK`);
    assert.ok(!/["'][^"']*\/ee(?:\/|["'])/u.test(source), `${file} imports an enterprise module`);
    for (const match of source.matchAll(/(?:from|import)\s*\(\s*["']([^"']+)["']|from\s+["']([^"']+)["']/gu)) {
      const specifier = match[1] ?? match[2];
      if (specifier === undefined || specifier.startsWith(".") || specifier.startsWith("node:")) {
        continue;
      }
      const name = specifier.startsWith("@") ? specifier.split("/").slice(0, 2).join("/") : specifier.split("/")[0];
      assert.ok(name !== undefined && allowed.has(name), `${file} imports undeclared production package ${specifier}`);
    }
  }
});

test("the runtime source and lockfile contain no Mastra SDK or enterprise module", () => {
  const pkg = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8")) as {
    dependencies: Record<string, string>;
  };
  assert.equal(pkg.dependencies["@mastra/core"], undefined);
  const lock = JSON.parse(readFileSync(join(repoRoot, "package-lock.json"), "utf8")) as {
    packages: Record<string, unknown>;
  };
  assert.equal(Object.keys(lock.packages).some((name) => name.includes("@mastra/")), false);
  const source = files(join(packageRoot, "src"))
    .filter((path) => path.endsWith(".ts"))
    .map((path) => readFileSync(path, "utf8"))
    .join("\n");
  assert.equal(/@mastra\//u.test(source), false, "runtime source must not import the Mastra SDK");
  assert.equal(/(^|\/)ee(?:\/|["'])/u.test(source), false, "runtime source must not import an enterprise module");
});
