// The ACP desktop owns authorization. The SDK request context carries a turn
// correlation key across its worker queue, not an account or operation grant.
import { AsyncLocalStorage } from 'node:async_hooks';
import { randomUUID } from 'node:crypto';
import { APP_SERVER } from './reads.mjs';

const TURN_CONTEXT = 'intcad:desktop-cad-turn';

// Match the bundled MCP adapter: errors fail the tool, successful structured
// results retain their content channel, and content-only results keep media.
function toolResult(result, name, { MastraError, ErrorDomain, ErrorCategory }) {
  if (result.isError) {
    const text = result.content?.map(part => part?.type === 'text' ? part.text : part?.type === 'resource' ? part.resource?.text : undefined)
      .filter(text => typeof text === 'string').join('\n').trim();
    throw new MastraError({
      id: 'MCP_CLIENT_TOOL_EXECUTION_FAILED',
      domain: ErrorDomain.MCP,
      category: ErrorCategory.THIRD_PARTY,
      text: text || 'MCP tool execution failed',
      details: {
        toolName: name, serverName: APP_SERVER,
        ...(result.structuredContent !== undefined ? { structuredContent: JSON.stringify(result.structuredContent) } : {}),
      },
    });
  }
  const structured = result.structuredContent;
  if (structured === undefined) return result;
  if (structured !== null && typeof structured === 'object') {
    Object.defineProperty(structured, Symbol.for('mastra.mcp.callToolContent'), { value: result.content, configurable: true });
    if (result._meta !== undefined) {
      Object.defineProperty(structured, Symbol.for('mastra.mcp.callToolMeta'), { value: result._meta, configurable: true });
    }
  }
  return structured;
}

export function cadBridge({ createTool, jsonSchema, RequestContext, errors, invoke }) {
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
          execute: async (args, context) => {
            const scope = turns.get(context?.requestContext?.get(TURN_CONTEXT));
            if (!scope || scope.binding !== binding) throw new Error('The Intcad CAD turn is no longer active');
            const result = await invoke('_intcad/cad/tool', { sessionId: scope.sessionId, turn: scope.turn, name, arguments: args });
            return toolResult(result, name, errors);
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
