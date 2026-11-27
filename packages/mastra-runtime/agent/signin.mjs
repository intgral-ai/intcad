// What a sign-in leaves to go by when it fails (INT-1521): what the provider's token endpoint
// answered, and a log in the app's own folder. Mastra Code's SDK keeps neither: it prints the HTTP
// status of a refused exchange and throws "Token exchange failed".

import { appendFileSync, mkdirSync, statSync, truncateSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { scrub } from './secrets.mjs';

// Where Mastra Code's ChatGPT sign-in trades the browser's code for a token (its
// `auth/providers/openai-codex.js`, `TOKEN_URL`).
export const OPENAI_TOKEN_URL = 'https://auth.openai.com/oauth/token';

// Longest part of a refusal read, and the log's size before it starts over.
const MAX_BODY = 16384;
const MAX_LOG = 256 * 1024;

const word = value => (typeof value === 'string' && /^[A-Za-z0-9_.-]{1,64}$/.test(value) ? value : null);

/**
 * What a refusal says: its status, the provider's error code (a short word, never its description)
 * and whether it is about the region or the plan. Only the code leaves this function.
 */
function refusalOf(status, body) {
  let json = null;
  try {
    json = JSON.parse(body);
  } catch {
    // Not JSON (a proxy's or a firewall's page): the status says what there is.
  }
  const code = word(json?.error?.code) ?? word(json?.error) ?? word(json?.code) ?? word(json?.error?.type) ?? word(json?.detail?.code);
  const text = `${code ?? ''} ${body}`;
  const kind = /unsupported_country|unsupported_region|country,? region,? or territory|not (yet )?(available|supported) in your (country|region)/i.test(text)
    ? 'region'
    : /usage_not_included|plan_not_supported|no_active_subscription|upgrade (your|to a) plan|plan does not include/i.test(text)
      ? 'plan'
      : 'refused';
  return { status, code, kind };
}

/**
 * A short word for why a request had no answer: the system's code (`ECONNREFUSED`, `ENOTFOUND`),
 * `timeout`, or `no-connection` when nothing says more (a proxy that refused the tunnel, say).
 */
export function failureCode(error) {
  const cause = error?.cause;
  if (error?.name === 'TimeoutError' || cause?.name === 'TimeoutError') return 'timeout';
  const named = [cause?.name, error?.name].find(name => word(name) && !['Error', 'TypeError', 'DOMException'].includes(name));
  return word(cause?.code) ?? word(error?.code) ?? named ?? 'no-connection';
}

/**
 * `fetch`, with what the answer to `url` was told to `onAnswer`: `{ status }` when it was taken,
 * `{ status, code, kind }` when it was refused (refusalOf), `{ error }` when there was none. The body
 * of a refusal is read from a copy, which leaves the answer as it is; that of an answer taken is
 * never read here.
 */
export function watchExchange(fetch, url, onAnswer) {
  return async function watched(input, init) {
    const target = typeof input === 'string' ? input : input instanceof URL ? input.href : input?.url;
    if (typeof target !== 'string' || target.split('?')[0] !== url) return fetch(input, init);
    let response;
    try {
      response = await fetch(input, init);
    } catch (error) {
      onAnswer({ error });
      throw error;
    }
    if (response.ok) {
      onAnswer({ status: response.status });
      return response;
    }
    let body = '';
    try {
      body = (await response.clone().text()).slice(0, MAX_BODY);
    } catch {
      // A body that cannot be read leaves the status.
    }
    onAnswer(refusalOf(response.status, body));
    return response;
  };
}

/**
 * The sign-in log, `logs/sign-in.log` in the agent's data folder: one line per step and per line
 * the agent printed while a sign-in was under way, each with secrets taken out (`scrub`). It starts
 * over when it has grown past 256 KiB. Writing never fails a sign-in.
 *
 * @param secrets () => the keys the agent holds, taken out too
 */
export function signInLog(dataDir, secrets = () => []) {
  if (!dataDir) return null;
  const path = join(dataDir, 'logs', 'sign-in.log');
  return {
    path,
    start() {
      try {
        mkdirSync(dirname(path), { recursive: true });
        if (statSync(path, { throwIfNoEntry: false })?.size > MAX_LOG) truncateSync(path, 0);
      } catch {
        // Written to as far as it goes.
      }
    },
    write(text) {
      try {
        appendFileSync(path, `${new Date().toISOString()} ${scrub(text, secrets()).replace(/\s*\n\s*/g, ' ')}\n`, { mode: 0o600 });
      } catch {
        // A full disk or a folder that cannot be written must not end the sign-in.
      }
    },
  };
}
