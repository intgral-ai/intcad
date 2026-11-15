/**
 * Bounded normalized session history.
 *
 * A round is appended atomically. If a new round would cross `historyBytes` or
 * `historyMessages`, only complete old rounds are trimmed. There is never a
 * partially appended user/tool round. If the new round alone cannot fit, the
 * append fails explicitly and leaves the previous history unchanged.
 */

import { resourceExhausted } from "./errors.js";
import { canonicalize } from "./jcs.js";
import { utf8Bytes } from "./protocol.js";
import type { ModelMessage } from "./host-ai/model-client.js";

export interface HistoryLimits {
  readonly historyBytes: number;
  readonly historyMessages: number;
}

export interface HistoryAppendResult {
  readonly trimmedRounds: number;
  readonly messages: number;
  readonly bytes: number;
}

export type HistoryLimit = "historyBytes" | "historyMessages";

function messageBytes(message: ModelMessage): number {
  return utf8Bytes(canonicalize(message));
}

function roundBytes(round: readonly ModelMessage[]): number {
  let bytes = 0;
  for (const message of round) {
    bytes += messageBytes(message);
  }
  return bytes;
}

export class SessionHistory {
  #rounds: ModelMessage[][] = [];
  #messages: ModelMessage[] = [];
  #bytes = 0;

  get messages(): readonly ModelMessage[] {
    return this.#messages;
  }

  get byteSize(): number {
    return this.#bytes;
  }

  get length(): number {
    return this.#messages.length;
  }

  get roundCount(): number {
    return this.#rounds.length;
  }

  reset(): void {
    this.#rounds = [];
    this.#messages = [];
    this.#bytes = 0;
  }

  /** The first limit that blocks this round after trimming complete old rounds. */
  blockedBy(round: readonly ModelMessage[], limits: HistoryLimits): HistoryLimit | null {
    if (round.length === 0) {
      return null;
    }
    let first = 0;
    let messages = this.#messages.length + round.length;
    let bytes = this.#bytes + roundBytes(round);
    while ((bytes > limits.historyBytes || messages > limits.historyMessages) && first < this.#rounds.length) {
      const dropped = this.#rounds[first];
      if (dropped === undefined) {
        break;
      }
      messages -= dropped.length;
      bytes -= roundBytes(dropped);
      first += 1;
    }
    if (bytes > limits.historyBytes) {
      return "historyBytes";
    }
    if (messages > limits.historyMessages) {
      return "historyMessages";
    }
    return null;
  }

  canAppend(round: readonly ModelMessage[], limits: HistoryLimits): boolean {
    return this.blockedBy(round, limits) === null;
  }

  appendRound(round: readonly ModelMessage[], limits: HistoryLimits): HistoryAppendResult {
    if (round.length === 0) {
      return { trimmedRounds: 0, messages: this.#messages.length, bytes: this.#bytes };
    }
    const blocked = this.blockedBy(round, limits);
    if (blocked !== null) {
      throw resourceExhausted(blocked, limits[blocked]);
    }
    let first = 0;
    let messages = this.#messages.length + round.length;
    let bytes = this.#bytes + roundBytes(round);
    while ((bytes > limits.historyBytes || messages > limits.historyMessages) && first < this.#rounds.length) {
      const dropped = this.#rounds[first];
      if (dropped === undefined) {
        break;
      }
      messages -= dropped.length;
      bytes -= roundBytes(dropped);
      first += 1;
    }
    const kept = this.#rounds.slice(first);
    kept.push([...round]);
    this.#rounds = kept;
    this.#messages = kept.flat();
    this.#bytes = bytes;
    return { trimmedRounds: first, messages: this.#messages.length, bytes: this.#bytes };
  }
}
