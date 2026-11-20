// The session the agent opens for the app.
//
// This is Mastra Code's own ACP session builder (`createAcpSession`, `src/acp/runtime.ts` of
// @mastra/code-sdk 1.19.1, Apache-2.0), adapted for one difference: the session has no vector
// store. Mastra Code's builder takes no option for that, and with a vector store its memory
// embeds text with a model run on this computer (`@mastra/fastembed`, downloaded on first use),
// which Intcad's package does not carry. Without one, Mastra's memory still observes a long
// conversation and recalls from it; it does so without vectors (`retrieval: true`, no embedder).
//
// `vector: false` is what switches it off: Mastra Code makes its default store only when the
// option is absent, and treats a false one as it treats the case it has itself, a session on
// storage handed in from outside. The option is documented for a vector store to use instead,
// not for `false`, so the build checks that the SDK's file is still the one this was written
// against (`../agent-package/adapted-from.json`, checked by `scripts/agent-package.mjs`). When
// that check fails after a new version of the SDK, compare its builder with this one again.

/**
 * `sdk` is what the entry loaded from Mastra Code: `createMastraCode`, `loadSettings`,
 * `resolveDefaultThinkingLevel`, `resolveSubagentsSetting`, `withCleanupFailure`, and ACP's
 * `RequestError`. Returns the function the ACP agent calls for `session/new`.
 */
export function sessionBuilder(sdk) {
  const { RequestError } = sdk;
  return async function createSession(request) {
    const names = new Set();
    const servers = request.mcpServers.map(server => {
      if ('type' in server && server.type === 'sse') throw RequestError.invalidParams(undefined, 'Legacy SSE MCP servers are unsupported; use HTTP or stdio');
      if (names.has(server.name)) throw RequestError.invalidParams(undefined, `Duplicate MCP server name: ${server.name}`);
      names.add(server.name);
      const config = 'command' in server
        ? { command: server.command, args: server.args, env: Object.fromEntries(server.env.map(item => [item.name, item.value])), cwd: request.cwd }
        : { url: server.url, headers: Object.fromEntries(server.headers.map(item => [item.name, item.value])) };
      return [server.name, config];
    });
    const settings = sdk.loadSettings();
    const result = await sdk.createMastraCode({
      cwd: request.cwd,
      initialState: { projectPath: request.cwd, yolo: false, permissionRules: { categories: {}, tools: { ask_user: 'deny' } } },
      mcpServers: Object.fromEntries(servers),
      // ACP clients can answer permission requests, but have no free-text tool response method.
      disabledTools: ['ask_user'],
      planApprovalStartsGoal: false,
      unixSocketPubSub: false,
      disableMcp: false,
      disableHooks: false,
      // Project environment files must not mutate other ACP sessions.
      disableEnvFile: true,
      subagents: sdk.resolveSubagentsSetting(settings.preferences.subagentsEnabled),
      // Intcad's difference: no vector store, so no embedding model.
      vector: false,
    });
    let cleanupPromise;
    const runtime = {
      controller: result.controller,
      session: result.session,
      modes: result.controller.listModes(),
      getSkills: async () => (await result.controller.resolveWorkspace({ session: result.session }))?.skills,
      getThinkingLevel: () => result.session.state.get().thinkingLevel ?? sdk.resolveDefaultThinkingLevel(settings, result.session.mode.get()).level,
      cleanup: () => (cleanupPromise ??= (async () => {
        try {
          result.releaseThreadClaims();
        } catch {
          // a claim that is already gone
        }
        await Promise.allSettled([
          Promise.resolve().then(() => result.session.abort()),
          Promise.resolve().then(() => result.session.thread.detachFromCurrent()),
          Promise.resolve().then(() => result.stopPluginSignalProviders()),
          Promise.resolve().then(() => result.githubSignals?.stopAllPolling()),
          Promise.resolve().then(() => result.threadScheduler.stop()),
        ]);
        let dispatchTimeout;
        await Promise.race([
          Promise.resolve().then(() => result.stopNotificationDispatch()).catch(() => {}),
          new Promise(done => {
            dispatchTimeout = setTimeout(done, 2000);
            dispatchTimeout.unref();
          }),
        ]).finally(() => clearTimeout(dispatchTimeout));
        await Promise.allSettled([
          Promise.resolve().then(() => result.session.thread.clearAndReleaseLock()),
          Promise.resolve().then(() => result.mcpManager?.disconnect()),
          Promise.resolve().then(() => result.controller.getMastra()?.stopWorkers()),
          Promise.resolve().then(() => result.controller.stopIntervals()),
        ]);
        await Promise.resolve().then(() => result.signalsPubSub?.close?.()).catch(() => {});
        await result.storage.close();
      })()),
    };
    try {
      const failed = (await result.mcpManager?.initInBackground())?.failed.filter(server => names.has(server.name)) ?? [];
      if (failed.length) throw RequestError.internalError(undefined, failed.map(server => `MCP server ${server.name}: ${server.error ?? 'connection failed'}`).join('; '));
      return runtime;
    } catch (error) {
      try {
        await runtime.cleanup?.();
      } catch (cleanupError) {
        throw sdk.withCleanupFailure(error, cleanupError);
      }
      throw error;
    }
  };
}
