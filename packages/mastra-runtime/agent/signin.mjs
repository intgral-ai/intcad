// What a sign-in leaves to go by when it fails (INT-1521): what the provider's token endpoint
// answered. Mastra Code's SDK keeps nothing of it: it prints the HTTP status of a refused exchange
// and throws "Token exchange failed".

// Where Mastra Code's ChatGPT sign-in trades the browser's code for a token (its
// `auth/providers/openai-codex.js`, `TOKEN_URL`).
export const OPENAI_TOKEN_URL = 'https://auth.openai.com/oauth/token';

// Longest part of a refusal read.
const MAX_BODY = 16384;

const word = value => (typeof value === 'string' && /^[A-Za-z0-9_.-]{1,64}$/.test(value) ? value : null);

/**
 * What a refusal says: its status and the provider's error code (a short word, never its
 * description). Only the code leaves this function.
 */
function refusalOf(status, body) {
  let json = null;
  try {
    json = JSON.parse(body);
  } catch {
    // Not JSON (a proxy's or a firewall's page): the status says what there is.
  }
  const code = word(json?.error?.code) ?? word(json?.error) ?? word(json?.code) ?? word(json?.error?.type);
  return { status, code, kind: 'refused' };
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
 * `{ status, code, kind: 'refused' }` when it was refused (refusalOf), `{ error }` when there was none. The body
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
