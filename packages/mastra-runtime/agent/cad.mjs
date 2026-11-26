// The ACP desktop owns authorization. The SDK request context carries a turn
// correlation key across its worker queue, not an account or operation grant.
import { AsyncLocalStorage } from 'node:async_hooks';
import { randomUUID } from 'node:crypto';
import { APP_SERVER } from './reads.mjs';

const TURN_CONTEXT = 'intcad:desktop-cad-turn';

export function cadBridge({ createTool, jsonSchema, RequestContext, invoke }) {
  const preparing = new WeakMap();
  const sessions = new WeakMap();
  const submission = new AsyncLocalStorage();
  const turns = new Map();

  return {
    tools(request) {
      const contract = request?._meta?.intcad?.cadBridge;
      if (contract === undefined) return undefined;
      if (contract?.version !== 1 || !Array.isArray(contract.tools) || contract.tools.length > 128) {
        throw new Error('Unsupported Intcad CAD bridge catalog');
      }
      if (request.mcpServers.some(server => server.name === APP_SERVER)) {
        throw new Error('The desktop CAD tools cannot share an MCP server name');
      }
      const tools = {};
      const binding = {};
      for (const item of contract.tools) {
        const { name, description, inputSchema } = item ?? {};
        if (typeof name !== 'string' || !/^[a-z][a-z0-9_]{0,63}$/u.test(name) ||
            typeof description !== 'string' || inputSchema?.type !== 'object') {
          throw new Error('Invalid Intcad CAD tool definition');
        }
        const id = `${APP_SERVER}_${name}`;
        if (Object.hasOwn(tools, id)) throw new Error('Duplicate Intcad CAD tool');
        tools[id] = createTool({
          id, description, inputSchema: jsonSchema(inputSchema),
          execute: (args, context) => {
            const scope = turns.get(context?.requestContext?.get(TURN_CONTEXT));
            if (!scope || scope.binding !== binding) throw new Error('The Intcad CAD turn is no longer active');
            return invoke('_intcad/cad/tool', { sessionId: scope.sessionId, turn: scope.turn, name, arguments: args });
          },
        });
      }
      preparing.set(request, binding);
      return tools;
    },
    bind(request, entry) {
      const binding = preparing.get(request);
      preparing.delete(request);
      if (!binding) return;
      sessions.set(entry, binding);
      const sendMessage = entry.session.sendMessage.bind(entry.session);
      entry.session.sendMessage = options => {
        const scope = submission.getStore();
        if (scope?.binding !== binding || !turns.has(scope.key)) return sendMessage(options);
        const requestContext = new RequestContext(options.requestContext?.entries());
        requestContext.set(TURN_CONTEXT, scope.key);
        return sendMessage({ ...options, requestContext });
      };
    },
    async prompt(request, entry, run) {
      const binding = sessions.get(entry);
      if (!binding) return run();
      const turn = request._meta?.intcad?.cadTurn;
      if (typeof turn !== 'string' || !turn || turn.length > 128) throw new Error('Missing Intcad CAD turn binding');
      const key = randomUUID();
      const scope = { key, binding, turn, sessionId: request.sessionId };
      turns.set(key, scope);
      try {
        return await submission.run(scope, run);
      } finally {
        turns.delete(key);
      }
    },
  };
}
