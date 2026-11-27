// The ChatGPT browser sign-in ends with the browser calling a local address. Mastra Code's own
// listener for that call gives up after 60 seconds, and what the browser sends later is lost. Intcad
// waits until the user signs in or cancels, so this file holds the two ports itself.
//
// How that fits Mastra's flow without changing it: when both of its ports (1455 and 1457) are taken,
// Mastra Code still builds the sign-in address (pointing at 1457) and then waits, without a limit,
// for the address the browser was sent back to, which a person would paste by hand. Here the
// listener that holds the ports receives that address and hands it over.

export const CALLBACK_PORTS = [1455, 1457];

// What the browser shows once it has called back. The sign-in is not finished then (the code is
// still to be exchanged), so the page sends the user back to the app and promises nothing.
const DONE = 'Go back to Intcad: the sign-in finishes there. You can close this page.';

const escaped = text => String(text).replace(/[&<>"']/g, char => `&#${char.charCodeAt(0)};`);

function donePage(sentence) {
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Intcad</title></head>
<body style="font: 16px system-ui, sans-serif; margin: 3rem auto; max-width: 28rem; padding: 0 1rem">
<p>${escaped(sentence)}</p>
</body></html>`;
}

function portBusy(port) {
  return Object.assign(new Error(`port ${port} is in use`), { code: 'PORT_BUSY' });
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
        // Mastra Code asks whoever holds the port to step aside (`/cancel`) before it gives up on it.
        // The holder is this listener, which stays.
        response.statusCode = url.pathname === '/cancel' ? 200 : url.pathname === '/auth/callback' ? 400 : 404;
      } catch {
        response.statusCode = 400;
      }
      response.end();
    });
    server.once('error', error => reject(error?.code === 'EADDRINUSE' ? portBusy(port) : error));
    server.once('listening', () => resolve(server));
    server.listen(port, '127.0.0.1');
  });
}

/**
 * Takes both ports and calls `onCallback` with the address the browser was sent to, whenever that
 * is. `onCallback` answers whether the call is the one waited for: one that is not (another
 * sign-in's, or not a sign-in's at all) is refused and the wait goes on. `close()` frees the ports
 * at once. The second port is the one the sign-in address will name, so it must be had; the first
 * only has to be out of Mastra Code's reach, which it also is when another program holds it.
 *
 * @param options.ports     the two ports, for the tests
 * @param options.sentence  () => what the browser shows after the call, in the user's language
 */
export async function holdCallbackPorts(http, onCallback, { ports = CALLBACK_PORTS, sentence = () => DONE } = {}) {
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
    } catch (error) {
      if (error?.code !== 'PORT_BUSY') throw error;
    }
    servers.push(await listen(http, ports[1], onCallback, sentence));
  } catch (error) {
    close();
    throw error;
  }
  return { close };
}
