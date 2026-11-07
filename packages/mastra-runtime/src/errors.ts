/**
 * Protocol 1.0 error values. The wire shape is defined by
 * `schema/mastra-runtime/v1/protocol.schema.json#/$defs/WireError`; this file
 * never invents a second catalog of codes.
 */

export const ERROR_CODES = [
  "INCOMPATIBLE_VERSION",
  "INVALID_REQUEST",
  "METHOD_NOT_FOUND",
  "UNSUPPORTED_CAPABILITY",
  "AUTH_REQUIRED",
  "CONNECTION_CHANGED",
  "PERMISSION_DENIED",
  "MODEL_ERROR",
  "MALFORMED_RESPONSE",
  "TASK_FAILED",
  "INVALID_PROPOSAL",
  "STALE_DOCUMENT",
  "DEPENDENCY_CHANGED",
  "INVALID_CANDIDATE",
  "PROFILE_UNSUPPORTED",
  "REPORT_MISMATCH",
  "NOT_READY",
  "NOT_FOUND",
  "IDEMPOTENCY_CONFLICT",
  "REQUEST_EXPIRED",
  "RESOURCE_EXHAUSTED",
  "TIMEOUT",
  "CANCELLED",
  "TRANSPORT_ERROR",
  "OUTCOME_UNKNOWN",
  "INTERNAL_ERROR",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];
export type RetryAdvice = "never" | "read_only" | "same_submission";
export type ErrorDetails = Readonly<Record<string, unknown>>;

export interface WireErrorValue {
  readonly code: ErrorCode;
  readonly message: string;
  readonly retry: RetryAdvice;
  readonly details: ErrorDetails;
}

const MESSAGE_BYTE_LIMIT = 2048;

/** Redact and bound diagnostic text before it reaches the wire. */
export function redactedMessage(message: string): string {
  const redacted = message
    .replace(/\bAuthorization\s*:\s*[^\r\n]*/giu, "Authorization: [redacted]")
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]+/giu, "Bearer [redacted]")
    .replace(/\bsk-[A-Za-z0-9_-]{8,}/gu, "[redacted]");
  if (Buffer.byteLength(redacted, "utf8") <= MESSAGE_BYTE_LIMIT) {
    return redacted;
  }
  // Cut on a code-point boundary; the protocol counts UTF-8 bytes, not UTF-16 units.
  let end = 0;
  let used = 0;
  for (const character of redacted) {
    const size = Buffer.byteLength(character, "utf8");
    if (used + size > MESSAGE_BYTE_LIMIT) {
      break;
    }
    used += size;
    end += character.length;
  }
  return redacted.slice(0, end);
}

export class ProtocolError extends Error {
  readonly value: WireErrorValue;

  constructor(code: ErrorCode, message: string, retry: RetryAdvice = "never", details: ErrorDetails = {}) {
    super(redactedMessage(message));
    this.name = "ProtocolError";
    this.value = { code, message: redactedMessage(message), retry, details };
  }

  toWire(): WireErrorValue {
    return { ...this.value, details: { ...this.value.details } };
  }
}

export function invalidRequest(message: string, details: ErrorDetails = {}): ProtocolError {
  return new ProtocolError("INVALID_REQUEST", message, "never", details);
}

export function methodNotFound(method: string): ProtocolError {
  return new ProtocolError("METHOD_NOT_FOUND", `Unknown method ${method}.`, "never", { method });
}

export function unsupportedCapability(capability: string): ProtocolError {
  return new ProtocolError("UNSUPPORTED_CAPABILITY", `This runtime does not support ${capability}.`, "never", { capability });
}

export function resourceExhausted(resource: string, limit: number): ProtocolError {
  return new ProtocolError("RESOURCE_EXHAUSTED", `Resource ${resource} is exhausted.`, "never", { resource, limit });
}

export function cancelled(operation: string): ProtocolError {
  return new ProtocolError("CANCELLED", "The operation was cancelled.", "never", { operation });
}

export function timeout(operation: string): ProtocolError {
  return new ProtocolError("TIMEOUT", "The operation timed out.", "same_submission", { operation });
}

export function asProtocolError(error: unknown, fallback = "The runtime failed."): ProtocolError {
  if (error instanceof ProtocolError) {
    return error;
  }
  if (error instanceof Error) {
    return new ProtocolError("INTERNAL_ERROR", error.message.length > 0 ? error.message : fallback);
  }
  return new ProtocolError("INTERNAL_ERROR", fallback);
}
