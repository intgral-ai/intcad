// The skill sets the built-in agent carries, and how they get to where Mastra Code looks for them.
//
// Mastra Code (SDK 1.19.1, `buildSkillPaths` in `dist/agents/workspace.js`) reads skills from
// `.mastracode/skills`, `.claude/skills` and `.agents/skills` under the session's folder and under
// its home. The agent's home is its own (main.mjs), so at start the agent puts each set the
// package carries in `<home>/.agents/skills`, one folder per skill, and touches nothing else there.
//
// A set is a list of the repository's skill folders, copied whole when the package is built
// (agent-package.mjs). Its stamp, `skill-set.json` beside the copies, names every file with its
// SHA-256; the stamp's `version` is the hash of the whole set. The package check fails when a
// copy is missing, differs from its stamp, or differs from the repository's folder.
//
// Each skill folder keeps its name: Mastra loads a skill only when the `name` in its SKILL.md is
// the folder's name. Two skills of one name in two of the folders it reads make Mastra refuse to
// pick either, so a name is in one set only, and no set installs into the session's folder.

import { createHash } from "node:crypto";
import { cpSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The sets, by name: where in the repository their folders are, and which. Another set (a skill
 * Intcad writes for this agent alone, say) is another entry, with names no other set uses.
 */
export const SKILL_SETS = [
  {
    name: "modelling-skills",
    from: "skills",
    skills: ["solidcraft-checking", "solidcraft-editing", "solidcraft-mechanisms", "solidcraft-modeling"],
  },
  // INT-1473: how the built-in agent works from the project brief (agent/brief.mjs). Written for
  // this agent alone, so it lives beside it, not with the repository's contributor skills.
  {
    name: "context-skills",
    from: "packages/mastra-runtime/agent-skills",
    skills: ["intcad-context"],
  },
];

/** The stamp beside a set's copies in the package. */
export const STAMP = "skill-set.json";

/** A skill's or a set's name: what Mastra accepts, and nothing that could leave a folder. */
const NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/u;
export const isName = (name) => typeof name === "string" && name.length <= 64 && NAME.test(name);

/** What a copy leaves out: hidden entries, Python's caches, installed packages. */
const skipped = (name) => name.startsWith(".") || name === "__pycache__" || name === "node_modules" || name.endsWith(".pyc");

/** Every file of a skill folder that a copy carries, as paths with "/", sorted. Links are not followed or carried. */
export function skillFiles(dir) {
  const found = [];
  const walk = (folder, prefix) => {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      if (skipped(entry.name)) continue;
      const path = `${prefix}${entry.name}`;
      if (entry.isDirectory()) walk(join(folder, entry.name), `${path}/`);
      else if (entry.isFile()) found.push(path);
    }
  };
  walk(dir, "");
  return found.sort();
}

const sha256 = (data) => createHash("sha256").update(data).digest("hex");
/** One hash for a list of named hashes, as `sha256sum` prints them, sorted by name. */
const listHash = (named) => sha256(Object.keys(named).sort().map((name) => `${named[name]}  ${name}\n`).join(""));

/** A skill folder's files with their hashes, and the hash of the whole. Null when the folder is not there. */
export function skillStamp(dir) {
  if (!existsSync(dir) || !lstatSync(dir).isDirectory()) return null;
  const files = {};
  for (const path of skillFiles(dir)) files[path] = sha256(readFileSync(join(dir, ...path.split("/"))));
  return { sha256: listHash(files), files };
}

/** The stamp of the skills `names` in the folder `root`: what `skill-set.json` holds. A missing skill is null. */
export function setStamp(name, from, root, names) {
  const skills = {};
  for (const skill of [...names].sort()) skills[skill] = skillStamp(join(root, skill));
  const whole = Object.values(skills).every(Boolean);
  return { set: name, from, version: whole ? `sha256:${listHash(Object.fromEntries(Object.entries(skills).map(([skill, stamp]) => [skill, stamp.sha256])))}` : null, skills };
}

/** How two stamps of one skill differ, as short sentences; empty when they are the same. */
export function stampDifference(want, have) {
  if (!want || !have) return [want ? "missing" : "not expected"];
  const said = [];
  for (const path of Object.keys(want.files)) {
    if (!(path in have.files)) said.push(`${path} missing`);
    else if (have.files[path] !== want.files[path]) said.push(`${path} differs`);
  }
  for (const path of Object.keys(have.files)) if (!(path in want.files)) said.push(`${path} not expected`);
  return said;
}

/**
 * Copy each set from the repository into `out/<set>/`, with its stamp. Used by the package build;
 * fails when a skill folder is missing, so no package is made without it.
 */
export function packSkillSets(repoRoot, out, sets = SKILL_SETS) {
  const packed = {};
  const owner = new Map();
  for (const set of sets) {
    if (!isName(set.name)) throw new Error(`a skill set's name is not one Mastra accepts: ${set.name}`);
    for (const skill of set.skills) {
      if (!isName(skill)) throw new Error(`${set.name}: a skill's name is not one Mastra accepts: ${skill}`);
      if (owner.has(skill)) throw new Error(`${skill} is in two sets, ${owner.get(skill)} and ${set.name}`);
      owner.set(skill, set.name);
    }
    const root = join(repoRoot, ...set.from.split("/"));
    const stamp = setStamp(set.name, set.from, root, set.skills);
    const missing = Object.keys(stamp.skills).filter((skill) => !stamp.skills[skill]);
    if (missing.length > 0) throw new Error(`${set.name}: not in the repository: ${missing.map((skill) => `${set.from}/${skill}`).join(", ")}`);
    const into = join(out, set.name);
    mkdirSync(into, { recursive: true });
    for (const skill of Object.keys(stamp.skills)) copySkill(join(root, skill), join(into, skill));
    writeFileSync(join(into, STAMP), `${JSON.stringify(stamp, null, 2)}\n`);
    packed[set.name] = { version: stamp.version, from: set.from, skills: Object.keys(stamp.skills) };
  }
  return packed;
}

/** Copy the files `skillFiles` names, and nothing else, from one skill folder into a new one. */
function copySkill(from, to) {
  for (const path of skillFiles(from)) {
    const parts = path.split("/");
    mkdirSync(join(to, ...parts.slice(0, -1)), { recursive: true });
    cpSync(join(from, ...parts), join(to, ...parts));
  }
  mkdirSync(to, { recursive: true });
}

/**
 * The sets to install: the package's (`<agent folder>/skills/<set>/skill-set.json`), or, when the
 * agent runs out of the repository, the repository's own folders. Each is `{ name, root, skills }`.
 */
export function setsToInstall(agentFolder, repoRoot, log = () => {}) {
  const packaged = join(agentFolder, "skills");
  if (existsSync(packaged)) {
    const sets = [];
    for (const entry of readdirSync(packaged, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      try {
        const stamp = JSON.parse(readFileSync(join(packaged, entry.name, STAMP), "utf8"));
        const skills = Object.keys(stamp?.skills ?? {});
        if (stamp?.set !== entry.name || !isName(entry.name) || skills.length === 0 || !skills.every(isName)) throw new Error("its stamp does not name it and its skills");
        sets.push({ name: entry.name, root: join(packaged, entry.name), skills });
      } catch (error) {
        log(`skills: the package's set ${entry.name} is left out: ${error.message}`);
      }
    }
    return sets;
  }
  if (repoRoot && SKILL_SETS.every((set) => set.skills.every((skill) => existsSync(join(repoRoot, ...set.from.split("/"), skill, "SKILL.md"))))) {
    return SKILL_SETS.map((set) => ({ name: set.name, root: join(repoRoot, ...set.from.split("/")), skills: set.skills }));
  }
  return [];
}

/** Where a set notes what it put in the skills folder: a file, which Mastra does not read as a skill. */
const noteOf = (target, set) => join(target, `.${set}.json`);
/** A copy under way or set aside, by this installer: `.<skill>.<pid>.new` or `.old`. */
const LEFTOVER = /^\.([a-z0-9]+(?:-[a-z0-9]+)*)\.\d+\.(?:new|old)$/u;

/**
 * Make `<home>/.agents/skills` hold each set's skills as `sets` has them. A skill folder whose
 * files are the same is left alone; one that differs (an older set, or the model wrote into it)
 * is replaced whole; one that an earlier set put there and this one does not carry is removed.
 * Nothing else in the folder or the home is touched. Never throws: a failure is logged and the
 * agent starts with what is there. Returns what it did, for the tests.
 */
export function installSkillSets({ sets, home, log = () => {} }) {
  const target = join(home, ".agents", "skills");
  const done = { installed: [], kept: [], removed: [], failed: [] };
  if (sets.length === 0) return done;
  try {
    mkdirSync(target, { recursive: true });
  } catch (error) {
    log(`skills: could not make ${target}: ${error.message}`);
    done.failed.push(target);
    return done;
  }
  const ours = new Set(sets.flatMap((set) => set.skills));
  // What a run that stopped half-way left, of the skills this agent installs.
  for (const name of safeList(target)) {
    const leftover = LEFTOVER.exec(name);
    if (leftover && ours.has(leftover[1])) remove(join(target, name), log);
  }
  for (const set of sets) {
    let before = [];
    try {
      const note = JSON.parse(readFileSync(noteOf(target, set.name), "utf8"));
      if (Array.isArray(note?.skills)) before = note.skills.filter(isName);
    } catch {
      // no note: the set was not installed here before
    }
    for (const skill of before) {
      if (ours.has(skill)) continue;
      const path = join(target, skill);
      if (existsSync(path) && lstatSync(path).isDirectory() && remove(path, log)) done.removed.push(skill);
    }
    for (const skill of set.skills) {
      const from = join(set.root, skill);
      const to = join(target, skill);
      try {
        const want = skillStamp(from);
        if (!want) throw new Error(`the package has no ${skill}`);
        // A link is replaced: what it points at is not this set's to keep.
        const have = existsSync(to) && lstatSync(to).isDirectory() ? skillStamp(to) : null;
        if (have && stampDifference(want, have).length === 0) {
          done.kept.push(skill);
          continue;
        }
        const fresh = join(target, `.${skill}.${process.pid}.new`);
        const old = join(target, `.${skill}.${process.pid}.old`);
        rmSync(fresh, { recursive: true, force: true });
        copySkill(from, fresh);
        if (existsSync(to) || isLink(to)) renameSync(to, old);
        renameSync(fresh, to);
        rmSync(old, { recursive: true, force: true });
        done.installed.push(skill);
      } catch (error) {
        log(`skills: could not install ${skill}: ${error.message}`);
        done.failed.push(skill);
      }
    }
    try {
      const note = noteOf(target, set.name);
      const text = `${JSON.stringify({ set: set.name, skills: set.skills }, null, 2)}\n`;
      if (!existsSync(note) || readFileSync(note, "utf8") !== text) {
        writeFileSync(`${note}.${process.pid}`, text);
        renameSync(`${note}.${process.pid}`, note);
      }
    } catch (error) {
      log(`skills: could not note what ${set.name} installed: ${error.message}`);
    }
  }
  if (done.installed.length > 0 || done.removed.length > 0) log(`skills: installed ${done.installed.join(", ") || "nothing new"}${done.removed.length > 0 ? `; removed ${done.removed.join(", ")}` : ""}`);
  return done;
}

const isLink = (path) => {
  try {
    return lstatSync(path).isSymbolicLink();
  } catch {
    return false;
  }
};

function safeList(dir) {
  try {
    return readdirSync(dir);
  } catch {
    return [];
  }
}

function remove(path, log) {
  try {
    rmSync(path, { recursive: true, force: true });
    return true;
  } catch (error) {
    log(`skills: could not remove ${path}: ${error.message}`);
    return false;
  }
}
