// Why a turn failed, in the five kinds the Assistant has words for. The provider's own message is all
// there is to go by, so the patterns are the phrases and status codes the providers use.

const KINDS = [
  // Nothing left on the plan or key. Before `rate`: providers send 429 for both and name the reason.
  ['quota', /insufficient[_ ]quota|insufficient[_ ]balance|exceeded your current quota|quota|credit balance|billing|usage limit|payment required|\b402\b/i],
  ['auth', /\b401\b|unauthori[sz]ed|invalid[_ ]api[_ ]key|incorrect api key|invalid x-api-key|could not find api key|api key (is )?(missing|required)|authentication|invalid[_ ]grant|token (has )?expired|expired token|sign in again|login required|not logged in/i],
  ['rate', /\b429\b|rate[_ -]?limit|too many requests|overloaded/i],
  ['model', /\b403\b|\b404\b|model[^.]{0,60}(not found|does not exist|not supported|not available|no access|not allowed)|unsupported model|do(es)? not have access to/i],
  ['network', /fetch failed|ENOTFOUND|ECONNREFUSED|ECONNRESET|ETIMEDOUT|EAI_AGAIN|EHOSTUNREACH|ENETUNREACH|socket hang up|getaddrinfo|network|timed out|cannot connect/i],
];

/** `auth`, `quota`, `rate`, `network`, `model`, or `other`. */
export function classifyFailure(message) {
  const text = String(message ?? '');
  for (const [kind, pattern] of KINDS) if (pattern.test(text)) return kind;
  return 'other';
}

/**
 * Why a sign-in could not finish: `port` (taken), `listen` (the system would not open it),
 * `offline` or `failed`. A sign-in whose time ran out is `timeout`, said by connections.mjs.
 */
export function classifySignInFailure(error) {
  if (error?.code === 'PORT_BUSY') return 'port';
  if (error?.code === 'LISTEN_FAILED') return 'listen';
  const text = `${error?.message ?? error} ${error?.cause?.code ?? ''} ${error?.cause?.message ?? ''}`;
  return classifyFailure(text) === 'network' ? 'offline' : 'failed';
}

/** The snapshot's `problem` for a sign-in that failed with `error`, without its `id`. */
export function signInProblem(error) {
  const kind = classifySignInFailure(error);
  if (kind === 'port') return error.holder ? { kind, holder: error.holder } : { kind };
  if (kind === 'listen') return { kind, port: error.port, code: error.errno ?? null };
  return { kind };
}
