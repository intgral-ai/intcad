// The ChatGPT browser sign-in ends with the browser calling a local address. Mastra Code's own
// listener for that call gives up after 60 seconds, and what the browser sends later is lost. Intcad
// waits until the user signs in, cancels or the sign-in's time is up (connections.mjs), so this file
// holds the two ports itself.
//
// How that fits Mastra's flow without changing it: when both of its ports (1455 and 1457) are taken,
// Mastra Code still builds the sign-in address (pointing at 1457) and then waits, without a limit,
// for the address the browser was sent back to, which a person would paste by hand. Here the
// listener that holds the ports receives that address and hands it over.

import { execFile } from 'node:child_process';
import { basename } from 'node:path';

export const CALLBACK_PORTS = [1455, 1457];

// What the browser shows once it has called back. The sign-in is not finished then (the code is
// still to be exchanged), so the page sends the user back to the app and promises nothing.
const DONE = 'Go back to Intcad: the sign-in finishes there. You can close this page.';

// Where this listener says that it is Intcad's, and which app started it: a sign-in that finds the
// port taken asks here, to say who holds it.
const WHO = '/intcad-sign-in';

const escaped = text => String(text).replace(/[&<>"']/g, char => `&#${char.charCodeAt(0)};`);

function donePage(sentence) {
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Intcad</title></head>
<body style="font: 16px system-ui, sans-serif; margin: 3rem auto; max-width: 28rem; padding: 0 1rem">
<p>${escaped(sentence)}</p>
</body></html>`;
}

function portBusy(port, holder) {
  return Object.assign(new Error(`port ${port} is in use`), { code: 'PORT_BUSY', port, holder });
}

function listenFailed(port, error) {
  const errno = typeof error?.code === 'string' ? error.code : null;
  return Object.assign(new Error(`port ${port} could not be opened: ${errno ?? error}`), { code: 'LISTEN_FAILED', port, errno });
}

function listen(http, port, onCallback, sentence) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((request, response) => {
      // Anything on this computer can call the port, with anything: nothing it sends may end the agent.
      try {
        const url = new URL(request.url ?? '/', `http://localhost:${port}`);
        if (url.pathname === '/auth/callback' && url.searchParams.get('code') && onCallback(url.toString())) {
          response.statusCode = 200;
          response.setHeader('Content-Type', 'text/html; charset=utf-8');
          response.end(donePage(sentence() || DONE));
          return;
        }
        if (url.pathname === WHO) {
          response.statusCode = 200;
          response.setHeader('Content-Type', 'application/json');
          response.end(JSON.stringify({ app: 'intcad', parent: process.ppid }));
          return;
        }
        // Mastra Code asks whoever holds the port to step aside (`/cancel`) before it gives up on it.
        // The holder is this listener, which stays.
        response.statusCode = url.pathname === '/cancel' ? 200 : url.pathname === '/auth/callback' ? 400 : 404;
      } catch {
        response.statusCode = 400;
      }
      response.end();
    });
    server.once('error', reject);
    server.once('listening', () => resolve(server));
    server.listen(port, '127.0.0.1');
  });
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** A program's output, or `''` when it cannot be run or fails: finding a holder is best effort. */
function output(program, args) {
  return new Promise(resolve => {
    try {
      execFile(program, args, { timeout: 2000, windowsHide: true, maxBuffer: 1 << 20 }, (error, stdout) => resolve(error ? '' : String(stdout)));
    } catch {
      resolve('');
    }
  });
}

/** What a listener on the port says of itself at `WHO`: `{ app, parent }`, or null. */
function ask(http, port) {
  return new Promise(resolve => {
    try {
      const request = http.get({ host: '127.0.0.1', port, path: WHO, timeout: 500 }, response => {
        let body = '';
        response.setEncoding('utf8');
        response.on('data', chunk => {
          body += chunk;
          if (body.length > 1000) request.destroy();
        });
        response.on('end', () => {
          try {
            resolve(response.statusCode === 200 ? JSON.parse(body) : null);
          } catch {
            resolve(null);
          }
        });
      });
      request.on('timeout', () => request.destroy());
      request.on('error', () => resolve(null));
    } catch {
      resolve(null);
    }
  });
}

/** The program a path names: the app it sits in (`…/Codex.app/…` is Codex), else its file name. */
export function programName(path) {
  const text = String(path ?? '').trim();
  const app = /([^/\\]+)\.app[/\\]/i.exec(text)?.[1];
  const name = (app ?? basename(text.replace(/\\/g, '/'))).replace(/[\p{Cc}\p{Cf}]/gu, '').trim();
  return name ? name.slice(0, 64) : null;
}

/** The process listening on `port` of 127.0.0.1, by what `lsof -F p` or `netstat -ano` prints. */
export function pidIn(text, port, platform = process.platform) {
  if (platform === 'win32') {
    for (const line of String(text).split(/\r?\n/)) {
      const [proto, local, , state, pid] = line.trim().split(/\s+/);
      if (proto === 'TCP' && local === `127.0.0.1:${port}` && state === 'LISTENING' && /^\d+$/.test(pid ?? '')) return Number(pid);
    }
    return null;
  }
  const pid = /^p(\d+)$/m.exec(String(text))?.[1];
  return pid ? Number(pid) : null;
}

/** The name of the program listening on `port`, where the system lets this process find it. */
async function programOn(port, platform) {
  if (platform === 'win32') {
    const pid = pidIn(await output('netstat', ['-ano', '-p', 'TCP']), port, platform);
    if (!pid) return null;
    // `"node.exe","1234",…`
    return programName(/^"([^"]+)"/.exec(await output('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH']))?.[1]);
  }
  const lsof = platform === 'darwin' ? '/usr/sbin/lsof' : 'lsof';
  const pid = pidIn(await output(lsof, ['-nP', `-iTCP:${port}`, '-sTCP:LISTEN', '-Fp']), port, platform);
  if (!pid) return null;
  return programName(await output(platform === 'darwin' ? '/bin/ps' : 'ps', ['-o', 'comm=', '-p', String(pid)]));
}

/**
 * Who holds `port`: `{ kind: 'here' }`, a sign-in of another design of this app (its agent has the
 * same parent); `{ kind: 'intcad' }`, one of another Intcad; `{ kind: 'program', name }`; or null
 * when the system does not say.
 */
export async function holderOf(http, port, platform = process.platform) {
  const answer = await ask(http, port);
  if (answer?.app === 'intcad') return { kind: answer.parent === process.ppid ? 'here' : 'intcad' };
  const name = await programOn(port, platform).catch(() => null);
  return name ? { kind: 'program', name } : null;
}

/**
 * Takes both ports and calls `onCallback` with the address the browser was sent to, whenever that
 * is. `onCallback` answers whether the call is the one waited for: one that is not (another
 * sign-in's, or not a sign-in's at all) is refused and the wait goes on. `close()` frees the ports
 * at once. The second port is the one the sign-in address will name, so it must be had; the first
 * only has to be out of Mastra Code's reach, which it also is when another program holds it or the
 * system will not open it (it would not open it for Mastra Code either: this is one process).
 *
 * A second port that is taken fails with `PORT_BUSY`, its `port` and its `holder` ([`holderOf`]),
 * any other refusal with `LISTEN_FAILED`, the `port` and the system's `errno`. A sign-in of another
 * design of this app lets go of the port a moment after the app cancels it, so that one is waited
 * for, at most `retryMs`.
 *
 * @param options.ports     the two ports, for the tests
 * @param options.sentence  () => what the browser shows after the call, in the user's language
 */
export async function holdCallbackPorts(http, onCallback, { ports = CALLBACK_PORTS, sentence = () => DONE, retryMs = 2000 } = {}) {
  const servers = [];
  const close = () => {
    for (const server of servers.splice(0)) {
      server.close();
      server.closeAllConnections?.();
    }
  };
  try {
    try {
      servers.push(await listen(http, ports[0], onCallback, sentence));
    } catch {
      // Out of Mastra Code's reach either way.
    }
    const until = Date.now() + retryMs;
    for (;;) {
      try {
        servers.push(await listen(http, ports[1], onCallback, sentence));
        break;
      } catch (error) {
        if (error?.code !== 'EADDRINUSE') throw listenFailed(ports[1], error);
        const holder = await holderOf(http, ports[1]);
        if (holder?.kind !== 'here' || Date.now() >= until) throw portBusy(ports[1], holder);
        await sleep(200);
      }
    }
  } catch (error) {
    close();
    throw error;
  }
  return { close };
}
