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
