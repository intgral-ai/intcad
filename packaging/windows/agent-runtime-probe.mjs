// Proves, inside the installed Agent's own folder (the working directory when this runs), that
// Node loads the bundled Microsoft runtime and that the two native modules which import it
// actually work. `packaging/windows/installed-check.ps1` reads the JSON line, checks the loaded
// modules while this process waits, then closes stdin.
import { createRequire } from 'node:module';
import { join } from 'node:path';

const require = createRequire(join(process.cwd(), 'noop.cjs'));
const result = {};
try {
  const libsql = require('@libsql/win32-x64-msvc');
  result.libsql = Object.keys(libsql).length > 0;
} catch (error) {
  result.libsql = false;
  result.libsqlError = String(error && error.message).slice(0, 160);
}
try {
  const { parse } = require('@ast-grep/napi');
  const matches = parse('JavaScript', 'const answer = 42;')?.root()?.findAll({ rule: { pattern: '42' } }) ?? [];
  result.astGrep = matches.length === 1;
  result.astGrepMatches = matches.length;
} catch (error) {
  result.astGrep = false;
  result.astGrepError = String(error && error.message).slice(0, 160);
}
result.node = process.version;
result.ok = result.libsql === true && result.astGrep === true;
process.stdout.write(`${JSON.stringify(result)}\n`);
// Wait for the caller to inspect the loaded modules; EOF or a second line ends the probe.
process.stdin.resume();
process.stdin.once('data', () => process.exit(result.ok ? 0 : 1));
process.stdin.once('end', () => process.exit(result.ok ? 0 : 1));
