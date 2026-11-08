/**
 * RFC 8785 JSON Canonicalization Scheme, limited to the data model the protocol
 * admits (finite numbers, strings, booleans, null, arrays and objects).
 *
 * Key order is UTF-16 code-unit order, which is exactly JavaScript's default
 * string comparison. Number formatting follows ECMAScript `Number::toString`,
 * which is the basis chosen by RFC 8785.
 */

export class CanonicalJsonError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CanonicalJsonError";
  }
}

export function canonicalize(value: unknown): string {
  if (value === null) {
    return "null";
  }
  switch (typeof value) {
    case "boolean":
      return value ? "true" : "false";
    case "string":
      return JSON.stringify(value);
    case "number":
      if (!Number.isFinite(value)) {
        throw new CanonicalJsonError("nonfinite number");
      }
      return Object.is(value, -0) ? "0" : JSON.stringify(value);
    case "object": {
      if (Array.isArray(value)) {
        return `[${value.map((item) => canonicalize(item)).join(",")}]`;
      }
      const object = value as Record<string, unknown>;
      const keys = Object.keys(object).sort();
      const members = keys.map((key) => `${JSON.stringify(key)}:${canonicalize(object[key])}`);
      return `{${members.join(",")}}`;
    }
    default:
      throw new CanonicalJsonError(`unsupported JSON type ${typeof value}`);
  }
}

export function canonicalizeToBytes(value: unknown): Buffer {
  return Buffer.from(canonicalize(value), "utf8");
}
