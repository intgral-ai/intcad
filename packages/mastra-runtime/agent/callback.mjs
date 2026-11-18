// The ChatGPT browser sign-in ends with the browser calling a local address. Mastra Code's own
// listener for that call gives up after 60 seconds, and what the browser sends later is lost. Intcad
// waits until the user signs in or cancels, so this file holds the two ports itself.
//
// How that fits Mastra's flow without changing it: when both of its ports (1455 and 1457) are taken,
// Mastra Code still builds the sign-in address (pointing at 1457) and then waits, without a limit,
// for the address the browser was sent back to, which a person would paste by hand. Here the
// listener that holds the ports receives that address and hands it over.

export const CALLBACK_PORTS = [1455, 1457];

const DONE_PAGE = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Signed in</title></head>
<body style="font: 16px system-ui, sans-serif; margin: 3rem auto; max-width: 28rem; padding: 0 1rem">
<p>Signed in. You can close this page and go back to Intcad.</p>
</body></html>`;

function portBusy(port) {
  return Object.assign(new Error(`port ${port} is in use`), { code: 'PORT_BUSY' });
}

function listen(http, port, onCallback) {
  return new Promise((resolve, reject) => {
    const server = http.createServer((request, response) => {
      const url = new URL(request.url ?? '/', `http://localhost:${port}`);
      if (url.pathname !== '/auth/callback' || !url.searchParams.get('code')) {
        // Mastra Code asks whoever holds the port to step aside (`/cancel`) before it gives up on it.
        // The holder is this listener, which stays.
        response.statusCode = url.pathname === '/cancel' ? 200 : 404;
        response.end();
        return;
      }
      response.statusCode = 200;
      response.setHeader('Content-Type', 'text/html; charset=utf-8');
      response.end(DONE_PAGE);
      onCallback(url.toString());
    });
    server.once('error', error => reject(error?.code === 'EADDRINUSE' ? portBusy(port) : error));
    server.once('listening', () => resolve(server));
    server.listen(port, '127.0.0.1');
  });
}

/**
 * Takes both ports and calls `onCallback` with the address the browser was sent to, whenever that
 * is. `close()` frees them at once. The second port is the one the sign-in address will name, so it
 * must be had; the first only has to be out of Mastra Code's reach, which it also is when another
 * program holds it.
 */
export async function holdCallbackPorts(http, onCallback, ports = CALLBACK_PORTS) {
  const servers = [];
  const close = () => {
    for (const server of servers.splice(0)) {
      server.close();
      server.closeAllConnections?.();
    }
  };
  try {
    try {
      servers.push(await listen(http, ports[0], onCallback));
    } catch (error) {
      if (error?.code !== 'PORT_BUSY') throw error;
    }
    servers.push(await listen(http, ports[1], onCallback));
  } catch (error) {
    close();
    throw error;
  }
  return { close };
}
