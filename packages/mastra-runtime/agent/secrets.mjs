// Where a key must not get to. A key reaches Mastra through this process's environment (see
// README.md), so two things that copy from the process are kept away from it: the environment
// Mastra Code gives the commands a model runs, and what ACP's SDK prints when it refuses a request.

/** The name of every environment variable Mastra reads a provider's key from. */
export function keyVariables(registry) {
  const names = new Set(['MASTRA_GATEWAY_API_KEY']);
  for (const config of Object.values(registry?.providers ?? {})) {
    for (const name of [config?.apiKeyEnvVar ?? []].flat()) if (typeof name === 'string' && name) names.add(name);
  }
  return names;
}

/**
 * Mastra Code starts every command of its command tool, and every process it keeps running, with
 * the environment `buildEnv` of its local sandbox returns, which begins as a copy of this
 * process's. After this the copy leaves the keys out, so a model that runs `env` sees none.
 *
 * `Sandbox` is Mastra's `LocalSandbox`. If it has no `buildEnv` this is not the SDK the agent was
 * written against, and starting without the guard would hand the keys over: it throws instead.
 */
export function keepKeysFromCommands(Sandbox, names) {
  const build = Sandbox?.prototype?.buildEnv;
  if (typeof build !== 'function') throw new Error("Mastra's command sandbox is not the one this agent was written for: commands would be given the keys");
  Sandbox.prototype.buildEnv = function buildEnv(...args) {
    const env = { ...build.apply(this, args) };
    for (const name of names) delete env[name];
    return env;
  };
}

/**
 * ACP's SDK prints a request it answers with an error, whole, and the app keeps the agent's last
 * lines to show when it exits. A request's parameters can be a key, a pasted code or what the user
 * wrote, so they are not printed: the method and the error say enough.
 */
export function withoutParams(args) {
  return args.map(arg => (arg && typeof arg === 'object' && !(arg instanceof Error) && typeof arg.method === 'string' && 'params' in arg ? { ...arg, params: '[not shown]' } : arg));
}

/**
 * Text with what could let someone in, or say whose account it is, taken out, for the sign-in log
 * (INT-1521): the secrets given (the keys the agent holds), a user and password in an address (a
 * proxy's), every value in an address's query (a sign-in's code and state), the fields that hold
 * tokens, JWTs, e-mail addresses, and any other run of 32 or more letters and digits (a key, a
 * code, a verifier).
 */
export function scrub(text, secrets = []) {
  let out = String(text ?? '');
  for (const secret of secrets) if (typeof secret === 'string' && secret.length >= 8) out = out.split(secret).join('[removed]');
  return out
    .replace(/(\b[a-z][a-z0-9+.-]*:\/\/)[^\s/?#@]+@/gi, '$1[removed]@')
    .replace(/([?&][^=\s&#]+=)[^\s&#"']+/g, '$1[removed]')
    .replace(/"(access_token|refresh_token|id_token|code_verifier|api_?key|token|password)"\s*:\s*"[^"]*"/gi, '"$1": "[removed]"')
    .replace(/\beyJ[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+){1,2}/g, '[removed]')
    .replace(/[\w.+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+/g, '[removed]')
    .replace(/[A-Za-z0-9_+=-]{32,}/g, '[removed]');
}
