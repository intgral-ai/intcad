#!/usr/bin/env node
// Build the Assistant's page from this folder's source and the pinned Mastra checkout, and say
// exactly what went into it.
//
//   node packages/assistant-web/tools/build.mjs --mastra <checkout> --check     (the default)
//   node packages/assistant-web/tools/build.mjs --mastra <checkout> --write
//
// --check builds and compares with what is checked in (dist/, THIRD-PARTY-NOTICES.txt,
// inventory.json) byte for byte, and fails on any difference. --write puts the build in place.
// The checkout is `mastra-ai/mastra` at the commit in mastra.json with its packages installed
// (`pnpm install --frozen-lockfile`); MASTRA_CHECKOUT names it too, and the default is the
// workspace layout of README.md.
//
// What makes the build the same everywhere:
// - It runs in a scratch folder that holds nothing but the source (src/, index.html, the Vite
//   and TypeScript configuration) in the folder layout the stylesheet's paths assume. A build
//   in place reads whatever else lies in this folder: Tailwind used to scan the last build's
//   dist/ for class names, so the stylesheet depended on what had been built before.
// - The checkout must be at the pinned commit with no change in the folders the page reads.
// - inventory.json names every npm package in the page's script with its exact version, from
//   the bundler's own list of modules, so other installed versions show up as a difference.
//
// What it refuses:
// - a script module, an imported stylesheet or a font under a directory named `ee/` (Mastra's
//   enterprise licence, see its LICENSE.md), and a stylesheet that does not keep Tailwind from
//   scanning Mastra's `ee/` files for class names;
// - a module from anywhere but this page, the Mastra checkout, an npm package or the bundler;
// - an npm package whose licence is not in the reviewed vocabulary of docs/mastra-distribution.md.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCES = ['src', 'index.html', 'vite.config.mjs', 'tsconfig.json'];
const OUTPUTS = ['THIRD-PARTY-NOTICES.txt', 'inventory.json'];
// The lines of src/assistant.css that keep Tailwind's scan to what the page renders.
const SCAN_LIMITS = [
  "@source not '../../../../../open-source/mastra/packages/playground-ui/src/ee';",
  "@source not '../dist';",
];
// The licence expressions a package may state: exactly the reviewed vocabulary of
// docs/mastra-distribution.md and packaging/mastra/policy.json, no wider. A package with any
// other expression stops the build until a person has read its licence and the policy says so.
const LICENCES = new Set(['Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'BlueOak-1.0.0', 'ISC', 'MIT', 'MIT OR Apache-2.0']);
const COPYLEFT = /GPL|SSPL|CPAL|EUPL|OSL|MPL|EPL|CDDL|CeCILL|CC-BY-SA/i;

function fail(message) {
  console.error(`assistant-web: ${message}`);
  process.exit(1);
}

function options() {
  const args = process.argv.slice(2);
  const found = { mode: 'check', mastra: process.env.MASTRA_CHECKOUT ?? resolve(HERE, '../../../../open-source/mastra'), keep: null };
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === '--check' || args[i] === '--write') found.mode = args[i].slice(2);
    else if (args[i] === '--mastra' && args[i + 1]) found.mastra = args[(i += 1)];
    else if (args[i] === '--keep' && args[i + 1]) found.keep = args[(i += 1)];
    else fail(`unknown argument ${args[i]} (use --mastra <checkout>, --check or --write, --keep <folder>)`);
  }
  found.mastra = resolve(found.mastra);
  return found;
}

function git(checkout, ...args) {
  const done = spawnSync('git', ['-C', checkout, ...args], { encoding: 'utf8' });
  if (done.status !== 0) fail(`git ${args.join(' ')} failed in ${checkout}: ${done.stderr.trim()}`);
  return done.stdout.trim();
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
// The same order on every machine: by code unit, never by the machine's language settings.
const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const text = path => readFileSync(path, 'utf8').replace(/\r\n/g, '\n');

/** The npm package a module belongs to: its folder, name and version. */
function packageOf(id) {
  const at = id.lastIndexOf(`${sep}node_modules${sep}`);
  const rest = id.slice(at + 14).split(sep);
  const name = rest[0].startsWith('@') ? `${rest[0]}/${rest[1]}` : rest[0];
  const folder = join(id.slice(0, at + 14), name);
  const manifest = JSON.parse(readFileSync(join(folder, 'package.json'), 'utf8'));
  return { folder, name: manifest.name ?? name, version: manifest.version ?? '', stated: manifest.license ?? manifest.licenses ?? null };
}

/** Whether a path of the checkout lies under a directory named ee/ (Mastra's enterprise licence). */
const underEe = path => path.split('/').some(part => part.toLowerCase() === 'ee');

/** The stylesheets a stylesheet imports by path, followed to the end (not packages named in an import). */
function importedStylesheets(file, found = new Set()) {
  for (const match of readFileSync(file, 'utf8').matchAll(/@import\s+(?:url\()?['"]([^'"]+)['"]/g)) {
    if (!match[1].startsWith('.')) continue;
    const target = realpathSync(resolve(dirname(file), match[1]));
    if (!found.has(target)) {
      found.add(target);
      importedStylesheets(target, found);
    }
  }
  return found;
}

/** The licence and notice files at a package's top level, by name. */
function licenceFiles(folder) {
  return readdirSync(folder, { withFileTypes: true })
    .filter(entry => entry.isFile() && /^(licen[cs]e|copying|notice)/i.test(entry.name))
    .map(entry => entry.name)
    .sort();
}

function main() {
  const { mode, mastra, keep } = options();
  const pin = JSON.parse(readFileSync(join(HERE, 'mastra.json'), 'utf8'));
  const ui = join(mastra, pin.package);
  if (!existsSync(join(ui, 'node_modules', 'vite', 'bin', 'vite.js'))) {
    fail(`no installed Mastra checkout at ${mastra} (clone ${pin.repository} at ${pin.commit} and run pnpm install --frozen-lockfile)`);
  }

  // The checkout is the pinned one, unchanged where the page reads it.
  const head = git(mastra, 'rev-parse', 'HEAD');
  if (head !== pin.commit) fail(`the Mastra checkout is at ${head}, and mastra.json pins ${pin.commit}`);
  const changed = git(mastra, 'status', '--porcelain', '--', pin.package, 'packages/playground/src', 'pnpm-lock.yaml');
  if (changed) fail(`the Mastra checkout has changes where the page reads it:\n${changed}`);
  const version = JSON.parse(readFileSync(join(ui, 'package.json'), 'utf8')).version;
  if (version !== pin.version) fail(`${pin.package} is version ${version}, and mastra.json says ${pin.version}`);

  const css = text(join(HERE, 'src', 'assistant.css'));
  for (const line of SCAN_LIMITS) {
    if (!css.includes(line)) fail(`src/assistant.css must keep Tailwind's scan off Mastra's ee/ files and off dist/; it lacks:\n  ${line}`);
  }

  // A scratch workspace in the layout the stylesheet's relative paths assume, with only source.
  const scratch = realpathSync(mkdtempSync(join(tmpdir(), 'assistant-web-')));
  const work = join(scratch, 'platform', 'open-skyfall-desktop', 'packages', 'assistant-web');
  mkdirSync(work, { recursive: true });
  mkdirSync(join(scratch, 'open-source'));
  symlinkSync(mastra, join(scratch, 'open-source', 'mastra'), 'dir');
  for (const name of SOURCES) cpSync(join(HERE, name), join(work, name), { recursive: true });
  symlinkSync(join(ui, 'node_modules'), join(work, 'node_modules'), 'dir');
  // The page's own configuration, with one plugin that only writes down what the bundler used.
  const audit = join(scratch, 'modules.json');
  writeFileSync(
    join(work, 'audit.config.mjs'),
    `import { writeFileSync } from 'node:fs';
import base from './vite.config.mjs';
const audit = {
  name: 'assistant-web-audit',
  generateBundle(_options, bundle) {
    const seen = { modules: [], assets: [] };
    for (const [file, item] of Object.entries(bundle)) {
      if (item.type === 'chunk') for (const [id, module] of Object.entries(item.modules)) seen.modules.push([id, module.renderedLength]);
      else seen.assets.push([file, item.originalFileNames ?? []]);
    }
    writeFileSync(${JSON.stringify(audit)}, JSON.stringify(seen));
  },
};
export default { ...base, plugins: [...base.plugins, audit] };
`,
  );
  const built = spawnSync(process.execPath, ['node_modules/vite/bin/vite.js', 'build', '--config', 'audit.config.mjs', '--configLoader', 'native'], {
    cwd: work,
    encoding: 'utf8',
    env: { ...process.env, MASTRA_CHECKOUT: join(scratch, 'open-source', 'mastra') },
  });
  if (built.status !== 0) fail(`the build failed:\n${built.stdout}\n${built.stderr}`);
  const seen = JSON.parse(readFileSync(audit, 'utf8'));

  // Where every module of the script comes from.
  const mastraReal = realpathSync(mastra);
  const packages = new Map();
  const fromMastra = [];
  const ours = [];
  const bundler = [];
  for (const [id, length] of seen.modules) {
    if (!length) continue; // named by an import and left out of the script whole
    if (id.startsWith('\0')) {
      bundler.push(id.slice(1));
    } else if (id.includes(`${sep}node_modules${sep}`)) {
      const found = packageOf(id);
      const key = `${found.name}@${found.version}`;
      const entry = packages.get(key) ?? { ...found, modules: 0 };
      entry.modules += 1;
      packages.set(key, entry);
    } else if (id.startsWith(mastraReal + sep) || id.startsWith(join(scratch, 'open-source', 'mastra') + sep)) {
      fromMastra.push(relative(id.startsWith(mastraReal + sep) ? mastraReal : join(scratch, 'open-source', 'mastra'), id).split(sep).join('/'));
    } else if (id.startsWith(work + sep)) {
      ours.push(relative(work, id).split(sep).join('/'));
    } else {
      fail(`the script holds a module from an unexpected place: ${id}`);
    }
  }
  const enterprise = fromMastra.filter(underEe);
  if (enterprise.length) fail(`the script holds Mastra files under an ee/ directory (enterprise licence):\n  ${enterprise.join('\n  ')}`);
  const outside = fromMastra.filter(path => !path.startsWith(`${pin.package}/src/`));
  if (outside.length) fail(`the script holds Mastra files from outside ${pin.package}/src:\n  ${outside.join('\n  ')}`);

  // The fonts, and the licence files beside them in the checkout.
  const fonts = [];
  for (const [file, origins] of seen.assets) {
    if (file === 'style.css' || file === 'index.html') continue;
    const origin = origins.map(path => realpathSync(resolve(work, path))).find(path => path.startsWith(mastraReal + sep));
    if (!origin) fail(`the build wrote ${file}, which does not come from the Mastra checkout`);
    fonts.push({ file, from: relative(mastraReal, origin).split(sep).join('/') });
  }
  fonts.sort((a, b) => byText(a.file, b.file));
  if (fonts.some(font => underEe(font.from))) fail(`a font comes from under an ee/ directory: ${fonts.map(font => font.from).join(', ')}`);

  // The stylesheets the page's own stylesheet imports from the checkout, to the end of the chain.
  const stylesheets = [];
  for (const path of importedStylesheets(join(work, 'src', 'assistant.css'))) {
    if (!path.startsWith(mastraReal + sep)) fail(`the stylesheet imports a file from outside the Mastra checkout: ${path}`);
    stylesheets.push(relative(mastraReal, path).split(sep).join('/'));
  }
  stylesheets.sort();
  if (stylesheets.some(underEe)) fail(`the stylesheet imports Mastra files under an ee/ directory:\n  ${stylesheets.filter(underEe).join('\n  ')}`);
  const fontFolder = [...new Set(fonts.map(font => dirname(font.from)))];
  if (fontFolder.length > 1) fail(`the fonts come from more than one folder: ${fontFolder.join(', ')}`);
  const fontLicences = fontFolder.length
    ? readdirSync(join(mastraReal, fontFolder[0])).filter(name => /^OFL.*\.txt$/i.test(name)).sort()
    : [];
  if (fonts.length && !fontLicences.length) fail(`no OFL licence text beside the fonts in ${fontFolder[0]}`);

  // Tailwind writes its base styles into the stylesheet: it is in the page too.
  const tailwind = packageOf(join(realpathSync(join(ui, 'node_modules', 'tailwindcss')), 'package.json'));
  const stylesheetKey = `${tailwind.name}@${tailwind.version}`;
  if (!packages.has(stylesheetKey)) packages.set(stylesheetKey, { ...tailwind, modules: 0 });

  // Licences: a reviewed expression for every package, and the texts each package ships.
  const listed = [...packages.values()].sort((a, b) => byText(`${a.name}@${a.version}`, `${b.name}@${b.version}`));
  const refused = [];
  for (const entry of listed) {
    const stated = typeof entry.stated === 'string' ? entry.stated : JSON.stringify(entry.stated);
    entry.stated = stated;
    entry.licence = LICENCES.has(stated) ? stated : null;
    if (!entry.licence || COPYLEFT.test(stated)) refused.push(`${entry.name}@${entry.version}: ${stated}`);
    entry.files = licenceFiles(entry.folder);
  }
  if (refused.length) fail(`packages whose licence is not in the reviewed vocabulary (docs/mastra-distribution.md):\n  ${refused.join('\n  ')}`);

  const rule = '='.repeat(80);
  const notices = [
    "Third-party notices for the Assistant's page",
    '',
    "The Assistant's interface in the desktop app is a page built from open-skyfall's own source",
    '(packages/assistant-web/src), the components of Mastra\'s @mastra/playground-ui, and the npm',
    'packages those import. This file lists everything in that page that is not open-skyfall\'s own,',
    'with the licence each part is used under and the licence and notice texts each part ships.',
    'It is written by packages/assistant-web/tools/build.mjs from the bundler\'s list of modules;',
    'do not edit it by hand.',
    '',
    rule,
    `${pin.package.split('/').pop()} (Mastra), ${pin.version}`,
    `${pin.repository} at commit ${pin.commit}`,
    `Licence: Apache-2.0. ${fromMastra.length} source files of ${pin.package}/src are in the page's script, and its`,
    `stylesheet is built from ${stylesheets.length} stylesheets of the checkout. Nothing under a directory named ee/`,
    'is in the page: those parts of Mastra are under a separate enterprise licence, and the build',
    'stops if a script module, an imported stylesheet or a font comes from one, and keeps its',
    'scan for class names off those directories. Mastra\'s LICENSE.md follows in full; the Apache',
    'License 2.0 itself is in LICENSE-APACHE, which open-skyfall ships and shows beside this text.',
    rule,
    '',
    text(join(mastraReal, 'LICENSE.md')).trimEnd(),
    '',
    rule,
    `Fonts: ${fonts.map(font => font.file).join(', ')}`,
    `From ${pin.repository} at commit ${pin.commit}, ${fontFolder[0] ?? ''}`,
    'Licence: SIL Open Font License 1.1. The licence texts beside the fonts:',
    rule,
    ...fontLicences.flatMap(name => ['', `--- ${name}`, '', text(join(mastraReal, fontFolder[0], name)).trimEnd()]),
    '',
  ];
  for (const entry of listed) {
    notices.push(rule, `${entry.name} ${entry.version}`, `Licence: ${entry.licence}`);
    if (!entry.files.length) notices.push('The package ships no licence file.');
    notices.push(rule);
    for (const name of entry.files) notices.push('', `--- ${name}`, '', text(join(entry.folder, name)).trimEnd());
    notices.push('');
  }

  const out = join(work, 'dist');
  const files = readdirSync(out).sort();
  const tool = name => JSON.parse(readFileSync(join(realpathSync(join(ui, 'node_modules', ...name.split('/'))), 'package.json'), 'utf8')).version;
  const inventory = {
    about: "What the Assistant's page is built from. Written by tools/build.mjs; do not edit by hand.",
    mastra: { repository: pin.repository, commit: pin.commit, package: pin.package, version: pin.version, licence: 'Apache-2.0', sourceFilesInScript: fromMastra.sort(), stylesheetsImported: stylesheets, filesUnderEe: 0 },
    ownSource: ours.sort(),
    bundler: { vite: tool('vite'), tailwindcss: tool('tailwindcss'), '@tailwindcss/vite': tool('@tailwindcss/vite'), '@vitejs/plugin-react': tool('@vitejs/plugin-react'), helpers: bundler.sort() },
    fonts: fonts.map(font => ({ ...font, licence: 'OFL-1.1' })),
    fontLicenceFiles: fontLicences,
    packages: listed.map(entry => ({ name: entry.name, version: entry.version, licence: entry.licence, modulesInScript: entry.modules, licenceFiles: entry.files })),
    built: Object.fromEntries(files.map(name => [name, { bytes: readFileSync(join(out, name)).length, sha256: sha256(readFileSync(join(out, name))) }])),
  };
  writeFileSync(join(work, 'THIRD-PARTY-NOTICES.txt'), `${notices.join('\n').trimEnd()}\n`);
  writeFileSync(join(work, 'inventory.json'), `${JSON.stringify(inventory, null, 2)}\n`);

  const summary = `${files.length} files in dist/, ${fromMastra.length} Mastra source files, ${listed.length} npm packages, none under ee/`;
  if (mode === 'write') {
    rmSync(join(HERE, 'dist'), { recursive: true, force: true });
    cpSync(out, join(HERE, 'dist'), { recursive: true });
    for (const name of OUTPUTS) cpSync(join(work, name), join(HERE, name));
    console.log(`assistant-web: wrote ${summary}`);
  } else {
    const differ = [];
    const checkedIn = existsSync(join(HERE, 'dist')) ? readdirSync(join(HERE, 'dist')).sort() : [];
    for (const name of new Set([...files, ...checkedIn])) {
      const a = existsSync(join(out, name)) ? sha256(readFileSync(join(out, name))) : 'not built';
      const b = existsSync(join(HERE, 'dist', name)) ? sha256(readFileSync(join(HERE, 'dist', name))) : 'not checked in';
      if (a !== b) differ.push(`dist/${name}: built ${a.slice(0, 16)}, checked in ${b.slice(0, 16)}`);
    }
    for (const name of OUTPUTS) {
      const b = existsSync(join(HERE, name)) ? readFileSync(join(HERE, name)) : null;
      if (!b || !readFileSync(join(work, name)).equals(b)) differ.push(`${name}: ${b ? 'differs from the build' : 'not checked in'}`);
    }
    if (differ.length) {
      if (keep) cpSync(work, resolve(keep), { recursive: true, dereference: false, filter: path => basename(path) !== 'node_modules' });
      rmSync(scratch, { recursive: true, force: true });
      fail(`what is checked in is not what the source builds (${summary}):\n  ${differ.join('\n  ')}\nRun the build with --write and commit the result, or find out what changed.`);
    }
    console.log(`assistant-web: what is checked in is what the source builds (${summary})`);
  }
  if (keep) cpSync(work, resolve(keep), { recursive: true, dereference: false, filter: path => basename(path) !== 'node_modules' });
  rmSync(scratch, { recursive: true, force: true });
}

main();
