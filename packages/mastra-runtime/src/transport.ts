/**
 * Byte-level NDJSON framing for protocol 1.0.
 *
 * A frame is exactly one JSON object and one LF, capped including the LF.
 * Oversized input is discarded incrementally, without retaining its remainder.
 */

import type { Readable, Writable } from "node:stream";

import { encodeFrame, type Frame } from "./protocol.js";

export interface TransportHandlers {
  readonly onFrame: (bytes: Buffer) => void;
  readonly onViolation: (reason: string) => void;
  readonly onEnd: (partial: boolean) => void;
}

export interface TransportOptions {
  readonly frameBytes: number;
  readonly queueBytes: number;
}

export class NdjsonTransport {
  readonly #input: Readable;
  readonly #output: Writable;
  #options: TransportOptions;
  readonly #handlers: TransportHandlers;
  #chunks: Buffer[] = [];
  #length = 0;
  #discarding = false;
  #closed = false;
  #violated = false;

  constructor(input: Readable, output: Writable, options: TransportOptions, handlers: TransportHandlers) {
    this.#input = input;
    this.#output = output;
    this.#options = options;
    this.#handlers = handlers;
  }

  start(): void {
    this.#input.on("data", (chunk: Buffer | string) => {
      this.#consume(typeof chunk === "string" ? Buffer.from(chunk, "utf8") : chunk);
    });    this.#input.on("end", () => {
      if (this.#closed || this.#violated) {
        return;
      }
      this.#closed = true;
      this.#handlers.onEnd(this.#length > 0);
    });
    this.#input.on("error", (error: Error) => {
      this.#violate(`stdin error: ${error.message}`);
    });
    this.#output.on("error", (error: Error) => {
      this.#closed = true;
      this.#handlers.onViolation(`stdout error: ${error.message}`);
    });
  }

  /** Apply a narrowed limit set after successful hello negotiation. */
  setLimits(options: TransportOptions): void {
    this.#options = options;
  }

  write(frame: Frame): void {
    if (this.#closed) {
      throw new Error("transport is closed");
    }
    const bytes = encodeFrame(frame);
    const queued = this.#output.writableLength ?? 0;
    if (queued + bytes.length > this.#options.queueBytes) {
      this.#violate("output queue exceeded queueBytes");
      return;
    }
    if (!this.#output.write(bytes)) {
      this.#output.once("drain", () => undefined);
    }
  }

  end(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#input.pause();
    this.#output.end();
  }

  closeInput(): void {
    this.#input.pause();
  }

  #violate(reason: string): void {
    if (this.#violated) {
      return;
    }
    this.#violated = true;
    this.#handlers.onViolation(reason);
  }

  #consume(chunk: Buffer): void {
    if (this.#closed || this.#violated) {
      return;
    }
    let start = 0;
    while (start < chunk.length) {
      const newline = chunk.indexOf(0x0a, start);
      if (newline === -1) {
        const rest = chunk.subarray(start);
        if (!this.#discarding) {
          if (this.#length + rest.length > this.#options.frameBytes) {
            this.#discarding = true;
            this.#chunks = [];
            this.#length = 0;
            this.#violate("frame exceeds frameBytes");
            return;
          }
          this.#chunks.push(rest);
          this.#length += rest.length;
        }
        return;
      }
      const piece = chunk.subarray(start, newline + 1);
      if (!this.#discarding) {
        if (this.#length + piece.length > this.#options.frameBytes) {
          this.#chunks = [];
          this.#length = 0;
          this.#violate("frame exceeds frameBytes");
          return;
        }
        this.#chunks.push(piece);
        this.#length += piece.length;
        const frame = Buffer.concat(this.#chunks, this.#length);
        this.#chunks = [];
        this.#length = 0;
        this.#handlers.onFrame(frame);
      }
      this.#discarding = false;
      start = newline + 1;
    }
  }
}
