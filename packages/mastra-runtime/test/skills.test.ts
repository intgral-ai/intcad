// The skill sets the built-in agent carries (`agent/skills.mjs`): what the package build copies,
// and what the agent's entry installs in its own home at start. Run on folders made for each test,
// with the repository's own skill folders as the originals: no Mastra, no model, no network.

import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";

type Json = Record<string, any>;
// Compiled, this file is in `dist/test/`; the modules stay where they are written.
const compiled = import.meta.url.includes("/dist/test/");
const agentFolder = compiled ? "../../agent/" : "../agent/";
const load = (name: string): Promise<Json> => import(new URL(agentFolder + name, import.meta.url).href);
const { SKILL_SETS, STAMP, installSkillSets, packSkillSets, setsToInstall, skillFiles } = await load("skills.mjs");

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), compiled ? "../../../.." : "../../..");
const MODELLING = ["solidcraft-checking", "solidcraft-editing", "solidcraft-mechanisms", "solidcraft-modeling"];
const CONTEXT = ["intcad-context"];
/** The sets the agent carries: the repository's modelling skills, and the skill written for this agent alone (INT-1473). */
const SETS: [string, { from: string; skills: string[] }][] = [
  ["modelling-skills", { from: "skills", skills: MODELLING }],
  ["context-skills", { from: "packages/mastra-runtime/agent-skills", skills: CONTEXT }],
];
const ALL = [...MODELLING, ...CONTEXT].sort();
const fromOf = (set: string): string => SETS.find(([name]) => name === set)?.[1].from ?? assert.fail(`no set ${set}`);
/** A skill's folder in the repository. */
const original = (set: string, skill: string) => join(repoRoot, ...fromOf(set).split("/"), skill);
const work = mkdtempSync(join(tmpdir(), "intcad-skills-test-"));
let made = 0;

/** A package's skills folder made from the repository, and a home the agent takes. */
function world() {
  const root = join(work, `case-${++made}`);
  const agent = join(root, "mastra-agent");
  const home = join(root, "data", "home");
  packSkillSets(repoRoot, join(agent, "skills"));
  mkdirSync(home, { recursive: true });
  const lines: string[] = [];
  const log = (line: string) => void lines.push(line);
  const install = () => installSkillSets({ sets: setsToInstall(agent, null, log), home, log });
  return { root, agent, home, skills: join(home, ".agents", "skills"), lines, log, install };
}

/** Every file under a folder, with "/", sorted. */
function files(dir: string): string[] {
  const out: string[] = [];
  const walk = (folder: string) => {
    for (const entry of readdirSync(folder, { withFileTypes: true })) {
      const path = join(folder, entry.name);
      if (entry.isDirectory()) walk(path);
      else out.push(relative(dir, path).split(sep).join("/"));
    }
  };
  if (existsSync(dir)) walk(dir);
  return out.sort();
}

/** The front matter of a SKILL.md as plain `key: value` lines, quotes taken off. */
function frontMatter(text: string): Record<string, string> {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n/u.exec(text);
  assert.ok(match, "a SKILL.md starts with front matter");
  const fields: Record<string, string> = {};
  for (const line of (match[1] ?? "").split(/\r?\n/u)) {
    const field = /^([a-z-]+):\s*(.*)$/u.exec(line);
    if (!field) continue;
    const value = (field[2] ?? "").trim();
    fields[field[1] ?? ""] = value.startsWith('"') && value.endsWith('"') ? JSON.parse(value) : value;
  }
  return fields;
}

test("the agent carries two sets, modelling-skills and context-skills, with no skill in both; Mastra would load each skill", () => {
  assert.deepEqual(
    SKILL_SETS.map((set: Json) => [set.name, set.from, [...set.skills].sort()]),
    SETS.map(([name, { from, skills }]) => [name, from, skills]),
  );
  const named = SKILL_SETS.flatMap((set: Json) => set.skills);
  assert.equal(new Set(named).size, named.length, "a name is in one set only: Mastra refuses two skills of one name");
  for (const [set, { skills }] of SETS) {
    for (const skill of skills) {
      const fields = frontMatter(readFileSync(join(original(set, skill), "SKILL.md"), "utf8"));
      // Mastra's own rules (`@mastra/core` 1.84.0, validateSkillMetadata): it skips a skill that breaks one.
      assert.equal(fields["name"], skill, `${skill}: Mastra loads a skill only when its name is its folder's`);
      assert.match(skill, /^[a-z0-9]+(-[a-z0-9]+)*$/u);
      assert.ok(skill.length <= 64, `${skill}: Mastra takes a name of at most 64 characters`);
      const description = fields["description"] ?? "";
      assert.ok(description.trim().length > 0 && description.length <= 1024, `${skill}: Mastra needs a description of 1 to 1024 characters (it has ${description.length})`);
    }
  }
});

test("the build copies each set's skills byte for byte, with a stamp, and leaves out caches and hidden files", () => {
  const { agent } = world();
  assert.deepEqual(readdirSync(join(agent, "skills")).sort(), SETS.map(([name]) => name).sort());
  const versions: Record<string, string> = {};
  for (const [set, { from, skills }] of SETS) {
    const into = join(agent, "skills", set);
    assert.deepEqual(readdirSync(into).sort(), [...skills, STAMP].sort());
    const stamp = JSON.parse(readFileSync(join(into, STAMP), "utf8"));
    assert.equal(stamp.set, set);
    assert.equal(stamp.from, from);
    assert.match(stamp.version, /^sha256:[0-9a-f]{64}$/u);
    versions[set] = stamp.version;
    for (const skill of skills) {
      const folder = original(set, skill);
      assert.deepEqual(files(join(into, skill)), skillFiles(folder), `${skill}: the same files`);
      for (const path of skillFiles(folder)) assert.ok(readFileSync(join(into, skill, path)).equals(readFileSync(join(folder, path))), `${skill}/${path} is copied as it is`);
      assert.deepEqual(Object.keys(stamp.skills[skill].files), skillFiles(folder));
      assert.ok(skillFiles(folder).every((path: string) => !path.includes("__pycache__") && !path.split("/").some((part) => part.startsWith("."))));
    }
  }
  assert.notEqual(versions["modelling-skills"], versions["context-skills"], "each set has its own version");
  // The same files give the same stamps.
  const again = world();
  for (const [set] of SETS) assert.equal(JSON.parse(readFileSync(join(again.agent, "skills", set, STAMP), "utf8")).version, versions[set]);
});

test("a first start puts each set's skills in <home>/.agents/skills and touches nothing else in the home", () => {
  const { home, skills, agent, install } = world();
  writeFileSync(join(home, "notes.txt"), "the agent's own\n");
  mkdirSync(join(home, ".agents", "other"), { recursive: true });
  writeFileSync(join(home, ".agents", "other", "kept.txt"), "not the skills folder\n");
  const before = files(home);
  const done = install();
  assert.deepEqual([...done.installed].sort(), ALL);
  assert.deepEqual(done.failed, []);
  const expected = [...before];
  for (const [set, { skills: names }] of SETS) {
    const packaged = join(agent, "skills", set);
    expected.push(`.agents/skills/.${set}.json`);
    for (const skill of names) {
      for (const path of skillFiles(join(packaged, skill))) {
        expected.push(`.agents/skills/${skill}/${path}`);
        assert.ok(readFileSync(join(skills, skill, path)).equals(readFileSync(join(packaged, skill, path))), `${skill}/${path}`);
      }
    }
    const carried = SKILL_SETS.find((one: Json) => one.name === set)?.skills;
    assert.deepEqual(JSON.parse(readFileSync(join(skills, `.${set}.json`), "utf8")), { set, skills: carried });
  }
  assert.deepEqual(files(home), expected.sort());
});

test("a second start changes nothing", () => {
  const { skills, install, lines } = world();
  install();
  const when = Object.fromEntries(files(skills).map((path) => [path, statSync(join(skills, path)).mtimeMs]));
  lines.length = 0;
  const done = install();
  assert.deepEqual(done.installed, []);
  assert.deepEqual([...done.kept].sort(), ALL);
  assert.deepEqual(Object.keys(when), files(skills));
  for (const path of files(skills)) assert.equal(statSync(join(skills, path)).mtimeMs, when[path], `${path} was not written again`);
  assert.deepEqual(lines, [], "nothing to say when nothing changed");
});

test("a skill the model changed, or a file added to one, is put back as the package has it", () => {
  const { skills, agent, install } = world();
  install();
  writeFileSync(join(skills, "solidcraft-modeling", "SKILL.md"), "---\nname: solidcraft-modeling\ndescription: changed\n---\n");
  writeFileSync(join(skills, "solidcraft-editing", "extra.md"), "added\n");
  rmSync(join(skills, "solidcraft-checking", "reference"), { recursive: true });
  const done = install();
  assert.deepEqual([...done.installed].sort(), ["solidcraft-checking", "solidcraft-editing", "solidcraft-modeling"]);
  assert.deepEqual([...done.kept].sort(), ["intcad-context", "solidcraft-mechanisms"]);
  const packaged = join(agent, "skills", "modelling-skills");
  assert.ok(readFileSync(join(skills, "solidcraft-modeling", "SKILL.md")).equals(readFileSync(join(packaged, "solidcraft-modeling", "SKILL.md"))));
  assert.ok(!existsSync(join(skills, "solidcraft-editing", "extra.md")));
  assert.deepEqual(files(join(skills, "solidcraft-checking")), skillFiles(join(packaged, "solidcraft-checking")));
});

test("a skill an earlier set carried is removed; a folder no set of the agent's put there is left", () => {
  const { skills, install } = world();
  install();
  writeFileSync(join(skills, ".modelling-skills.json"), JSON.stringify({ set: "modelling-skills", skills: [...MODELLING, "solidcraft-retired"] }));
  mkdirSync(join(skills, "solidcraft-retired"));
  writeFileSync(join(skills, "solidcraft-retired", "SKILL.md"), "old\n");
  // Beside the set: another skill, such as one INT-1477 installs into the same folder.
  mkdirSync(join(skills, "intcad-text-to-cad"));
  writeFileSync(join(skills, "intcad-text-to-cad", "SKILL.md"), "another set's\n");
  const done = install();
  assert.deepEqual(done.removed, ["solidcraft-retired"]);
  assert.ok(!existsSync(join(skills, "solidcraft-retired")));
  assert.equal(readFileSync(join(skills, "intcad-text-to-cad", "SKILL.md"), "utf8"), "another set's\n");
});

test("each set's install leaves the other set's folders and note alone", () => {
  const { agent, home, skills, log } = world();
  const all = setsToInstall(agent, null, log);
  assert.deepEqual(all.map((set: Json) => set.name).sort(), SETS.map(([name]) => name).sort());
  const only = (name: string) => all.filter((set: Json) => set.name === name);
  // Alone, in a home of its own, each set puts only its own skills and note there.
  for (const [set, { skills: names }] of SETS) {
    const alone = join(home, "..", `alone-${set}`);
    const done = installSkillSets({ sets: only(set), home: alone, log });
    assert.deepEqual([...done.installed].sort(), [...names].sort());
    assert.deepEqual(readdirSync(join(alone, ".agents", "skills")).sort(), [`.${set}.json`, ...names].sort());
  }
  installSkillSets({ sets: all, home, log });
  // The model changed a skill of each set: each set's install puts back its own skill only.
  writeFileSync(join(skills, "solidcraft-modeling", "SKILL.md"), "changed by the model\n");
  writeFileSync(join(skills, "intcad-context", "SKILL.md"), "changed by the model\n");
  const modellingNote = readFileSync(join(skills, ".modelling-skills.json"), "utf8");
  let done = installSkillSets({ sets: only("modelling-skills"), home, log });
  assert.deepEqual(done.installed, ["solidcraft-modeling"]);
  assert.equal(readFileSync(join(skills, "intcad-context", "SKILL.md"), "utf8"), "changed by the model\n", "the context set's folder is not the modelling set's to put back");
  done = installSkillSets({ sets: only("context-skills"), home, log });
  assert.deepEqual(done.installed, ["intcad-context"]);
  assert.notEqual(readFileSync(join(skills, "intcad-context", "SKILL.md"), "utf8"), "changed by the model\n");
  // A skill the context set once carried is removed by it; the modelling set's skills and note stay.
  writeFileSync(join(skills, ".context-skills.json"), JSON.stringify({ set: "context-skills", skills: ["intcad-context", "intcad-retired"] }));
  mkdirSync(join(skills, "intcad-retired"));
  writeFileSync(join(skills, "intcad-retired", "SKILL.md"), "old\n");
  writeFileSync(join(skills, "solidcraft-editing", "extra.md"), "added by the model\n");
  done = installSkillSets({ sets: only("context-skills"), home, log });
  assert.deepEqual(done.removed, ["intcad-retired"]);
  assert.deepEqual(done.installed, []);
  for (const skill of MODELLING) assert.ok(existsSync(join(skills, skill, "SKILL.md")), `${skill} stays`);
  assert.ok(existsSync(join(skills, "solidcraft-editing", "extra.md")), "the modelling set's folders are not the context set's to put back");
  assert.equal(readFileSync(join(skills, ".modelling-skills.json"), "utf8"), modellingNote);
  // A skill that moved from one set to the other is kept, whichever note still names it.
  writeFileSync(join(skills, ".context-skills.json"), JSON.stringify({ set: "context-skills", skills: ["intcad-context", "solidcraft-modeling"] }));
  done = installSkillSets({ sets: all, home, log });
  assert.deepEqual(done.removed, []);
  assert.ok(existsSync(join(skills, "solidcraft-modeling", "SKILL.md")));
});

test("what a stopped start left is cleared, so Mastra finds no second copy of a skill", () => {
  const { skills, install } = world();
  install();
  const leftover = join(skills, `.solidcraft-modeling.${process.pid + 1}.new`);
  mkdirSync(leftover);
  writeFileSync(join(leftover, "SKILL.md"), "half a copy\n");
  const unrelated = join(skills, ".someone-else.1.new");
  mkdirSync(unrelated);
  install();
  assert.ok(!existsSync(leftover));
  assert.ok(existsSync(unrelated), "only this installer's own leftovers are cleared");
});

test("a note or a stamp that names a path outside the folder moves nothing outside it", () => {
  const { root, home, skills, agent, install, lines } = world();
  install();
  mkdirSync(join(home, "outside"));
  writeFileSync(join(home, "outside", "keep.txt"), "keep\n");
  writeFileSync(join(skills, ".modelling-skills.json"), JSON.stringify({ set: "modelling-skills", skills: ["../../outside", "..", "../outside", "/etc"] }));
  install();
  assert.equal(readFileSync(join(home, "outside", "keep.txt"), "utf8"), "keep\n");
  // A package whose stamp names a skill that is not a name: that set is left out, and the agent starts.
  const bad = join(agent, "skills", "modelling-skills", STAMP);
  const stamp = JSON.parse(readFileSync(bad, "utf8"));
  writeFileSync(bad, JSON.stringify({ ...stamp, skills: { ...stamp.skills, "../../outside": stamp.skills["solidcraft-modeling"] } }));
  lines.length = 0;
  const names = (sets: Json[]) => sets.map((set) => set.name);
  assert.deepEqual(names(setsToInstall(agent, null, (line: string) => void lines.push(line))), ["context-skills"], "the other set still installs");
  assert.match(lines.join("\n"), /modelling-skills is left out/u);
  writeFileSync(bad, "not json");
  assert.deepEqual(names(setsToInstall(agent, null, () => {})), ["context-skills"]);
  assert.ok(existsSync(join(root, "data", "home", "outside", "keep.txt")));
});

test("with no package and no repository nothing is installed; a home that cannot hold the folder, or a package's skills that is a file, does not stop the agent", () => {
  const empty = mkdtempSync(join(work, "empty-"));
  assert.deepEqual(setsToInstall(empty, null), []);
  assert.deepEqual(setsToInstall(empty, empty), []);
  const home = join(empty, "home");
  installSkillSets({ sets: [], home, log: () => {} });
  assert.ok(!existsSync(home));
  const { agent } = world();
  const blocked = join(empty, "blocked");
  mkdirSync(blocked);
  writeFileSync(join(blocked, ".agents"), "a file where the folder would be\n");
  const lines: string[] = [];
  const done = installSkillSets({ sets: setsToInstall(agent, null), home: blocked, log: (line: string) => void lines.push(line) });
  assert.equal(done.failed.length, 1);
  assert.match(lines.join("\n"), /could not make/u);
  // A package whose `skills` is a file, not a folder: logged, and the agent starts with no set.
  const broken = mkdtempSync(join(work, "broken-"));
  writeFileSync(join(broken, "skills"), "a file where the folder would be\n");
  lines.length = 0;
  assert.deepEqual(setsToInstall(broken, repoRoot, (line: string) => void lines.push(line)), []);
  assert.match(lines.join("\n"), /skills folder cannot be read/u);
});

test("run out of the repository, the agent installs the repository's folders, and only when every set is there", () => {
  const empty = mkdtempSync(join(work, "repo-run-"));
  const sets = setsToInstall(empty, repoRoot);
  assert.deepEqual(
    sets.map((set: Json) => [set.name, set.root, [...set.skills].sort()]),
    [
      ["modelling-skills", join(repoRoot, "skills"), MODELLING],
      ["context-skills", join(repoRoot, "packages", "mastra-runtime", "agent-skills"), CONTEXT],
    ],
  );
  // A repository that has the modelling skills and not the context skill is not one to install from.
  const partial = join(empty, "partial");
  for (const skill of MODELLING) cpSync(original("modelling-skills", skill), join(partial, "skills", skill), { recursive: true });
  assert.deepEqual(setsToInstall(empty, partial), []);
});

test("the build refuses a set with a missing skill, a name Mastra refuses, or a skill in two sets", () => {
  const out = mkdtempSync(join(work, "pack-"));
  assert.throws(() => packSkillSets(repoRoot, out, [{ name: "modelling-skills", from: "skills", skills: ["solidcraft-modeling", "solidcraft-gone"] }]), /not in the repository: skills\/solidcraft-gone/u);
  assert.throws(() => packSkillSets(repoRoot, out, [{ name: "context-skills", from: "packages/mastra-runtime/agent-skills", skills: ["intcad-gone"] }]), /not in the repository: packages\/mastra-runtime\/agent-skills\/intcad-gone/u);
  assert.throws(() => packSkillSets(repoRoot, out, [{ name: "Modelling", from: "skills", skills: ["solidcraft-modeling"] }]), /name is not one Mastra accepts/u);
  assert.throws(() => packSkillSets(repoRoot, out, [{ name: "a", from: "skills", skills: ["../skills"] }]), /name is not one Mastra accepts/u);
  assert.throws(
    () =>
      packSkillSets(repoRoot, out, [
        { name: "a", from: "skills", skills: ["solidcraft-modeling"] },
        { name: "b", from: "skills", skills: ["solidcraft-modeling"] },
      ]),
    /in two sets/u,
  );
  assert.throws(
    () =>
      packSkillSets(repoRoot, out, [
        { name: "a", from: "skills", skills: ["solidcraft-modeling"] },
        { name: "b", from: "packages/mastra-runtime/agent-skills", skills: ["intcad-context", "solidcraft-modeling"] },
      ]),
    /solidcraft-modeling is in two sets, a and b/u,
  );
});

test("the entry installs the sets in its own home before anything of Mastra's is loaded", () => {
  const entry = readFileSync(join(repoRoot, "packages", "mastra-runtime", "agent", "main.mjs"), "utf8");
  const ownHome = /if \(process\.env\.MASTRA_APP_DATA_DIR\) \{([\s\S]*?)\r?\n\}\r?\n/u.exec(entry)?.[1] ?? "";
  assert.ok(ownHome.includes("installSkillSets({ sets, home, log })"), "only with a home of its own (MASTRA_APP_DATA_DIR)");
  assert.ok(ownHome.indexOf("process.env.HOME = home") < ownHome.indexOf("installSkillSets("), "after the home is the agent's");
  const firstLoad = entry.indexOf("await Promise.all(");
  assert.ok(firstLoad > 0 && entry.indexOf("installSkillSets(") < firstLoad, "before Mastra's files are loaded");
  assert.equal(entry.split("installSkillSets(").length, 2, "in one place");
});

after(() => rmSync(work, { recursive: true, force: true }));
