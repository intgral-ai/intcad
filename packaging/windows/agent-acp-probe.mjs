// Starts the installed packaged Agent entry over its ACP stdio with the installed Node, sends one
// `initialize` request and prints the reply line. `packaging/windows/installed-check.ps1` runs it
// with the installed folder as the working directory and the installed Node as the program, so a
// reply proves that the packaged entry and its SDK load, not that a development tree does.
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const entry = join(process.cwd(), 'agent', 'main.mjs');
if (!existsSync(entry)) {
  process.stdout.write(`${JSON.stringify({ ok: false, error: 'agent/main.mjs is missing' })}\n`);
  process.exit(1);
}
const child = spawn(process.execPath, [entry], { cwd: process.cwd(), stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
let out = '';
let err = '';
const timer = setTimeout(() => {
  process.stdout.write(`${JSON.stringify({ ok: false, error: 'no ACP reply within 30 s', stderr: err.slice(-400) })}\n`);
  child.kill();
  process.exit(1);
}, 30000);
child.stdout.on('data', (block) => {
  out += block;
  if (out.includes('\n')) {
    clearTimeout(timer);
    const line = out.split('\n', 1)[0];
    let reply = null;
    try { reply = JSON.parse(line); } catch { reply = null; }
    const ok = !!(reply && reply.id === 1 && reply.result && (reply.result.agentInfo || reply.result.protocolVersion));
    process.stdout.write(`${JSON.stringify({ ok, reply })}\n`);
    child.kill();
    process.exit(ok ? 0 : 1);
  }
});
child.stderr.on('data', (block) => { err += block; });
child.on('error', (error) => {
  clearTimeout(timer);
  process.stdout.write(`${JSON.stringify({ ok: false, error: String(error.message) })}\n`);
  process.exit(1);
});
child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: 1, clientCapabilities: {}, clientInfo: { name: 'intcad-installed-check', version: '1' } } })}\n`);
