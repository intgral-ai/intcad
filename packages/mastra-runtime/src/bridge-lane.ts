/**
 * One reusable line lane over a host-provisioned duplex stream.
 *
 * The host may inject the SolidCraft MCP bridge as an inherited file descriptor
 * (`--bridge-fd`). One runtime process serves several turns; #9's `McpClient`
 * owns one MCP session per turn, so the lane hands each turn a fresh
 * `McpBridgeChannel` view while keeping the underlying stream open.
 *
 * Closing a channel view stops that turn's MCP session but never destroys the
 * host stream. The runtime closes the lane itself when the process stops.
 */

import type { Readable, Writable } from "node:stream";

import type { McpBridgeChannel } from "./solidcraft/mcp-client.js";

const DEFAULT_MAX_LINE_BYTES = 8 * 1024 * 1024;

export interface BridgeLane {
  /** A fresh channel view for one turn's MCP session. */
  channel(): McpBridgeChannel;
  /** Close the underlying lane; later channel sends fail. */
  close(reason?: string): void;
}

export function bridgeLane(
  input: Readable,
  output: Writable,
  maxLineBytes = DEFAULT_MAX_LINE_BYTES,
): BridgeLane {
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let buffered: Buffer = Buffer.alloc(0);
  let active: { onLine: (line: string) => void; onClose: (reason: string) => void } | null = null;
  let closed = false;

  const finish = (reason: string): void => {
    if (closed) {
      return;
    }
    closed = true;
    buffered = Buffer.alloc(0);
    const listener = active;
    active = null;
    listener?.onClose(reason);
  };

  const deliver = (line: string): void => {
    const listener = active;
    if (listener === null) {
      // No MCP session is active between turns. Unsolicited bridge traffic is
      // dropped rather than replayed into a later turn.
      return;
    }
    listener.onLine(line);
  };

  input.on("data", (chunk: Buffer | string) => {
    if (closed) {
      return;
    }
    const bytes = typeof chunk === "string" ? Buffer.from(chunk, "utf8") : chunk;
    if (buffered.length === 0 && bytes.indexOf(0x0a) === -1 && bytes.length > maxLineBytes) {
      finish(`the SolidCraft bridge sent more than ${maxLineBytes} bytes without a line break`);
      return;
    }
    buffered = buffered.length === 0 ? bytes : Buffer.concat([buffered, bytes]);
    for (;;) {
      if (closed) {
        return;
      }
      const newline = buffered.indexOf(0x0a);
      if (newline === -1) {
        if (buffered.length > maxLineBytes) {
          finish(`the SolidCraft bridge sent more than ${maxLineBytes} bytes without a line break`);
        }
        return;
      }
      const raw = buffered.subarray(0, newline);
      buffered = buffered.subarray(newline + 1);
      if (raw.length === 0) {
        continue;
      }
      let line: string;
      try {
        line = decoder.decode(raw);
      } catch {
        finish("the SolidCraft bridge sent invalid UTF-8");
        return;
      }
      deliver(line);
    }
  });
  input.on("end", () => finish("the SolidCraft bridge closed"));
  input.on("error", (error: Error) => finish(`the SolidCraft bridge failed: ${error.message.slice(0, 200)}`));

  return {
    channel(): McpBridgeChannel {
      let detached = false;
      const detach = (): void => {
        if (!detached) {
          detached = true;
          if (active !== null) {
            active = null;
          }
          // A partial line belongs to the closing turn; never prepend it to the
          // next turn's MCP session on this lane.
          buffered = Buffer.alloc(0);
        }
      };
      return {
        send(line: string): void {
          if (closed) {
            throw new Error("the SolidCraft bridge lane is closed");
          }
          if (detached) {
            throw new Error("the SolidCraft bridge channel for this turn is closed");
          }
          output.write(`${line}\n`);
        },
        receive(onLine, onClose): () => void {
          if (closed) {
            onClose("the SolidCraft bridge lane is closed");
            return () => undefined;
          }
          const listener = { onLine, onClose };
          active = listener;
          return () => {
            if (active === listener) {
              active = null;
            }
            detached = true;
          };
        },
        close(reason = "closed by the runtime"): void {
          if (detached) {
            return;
          }
          const listener = active;
          detach();
          listener?.onClose(reason);
        },
      };
    },
    close(reason = "closed by the runtime"): void {
      finish(reason);
    },
  };
}
