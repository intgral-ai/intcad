/**
 * Strict JSON parsing for the protocol boundary.
 *
 * `JSON.parse` silently keeps the last duplicate key and turns an overflowing
 * number such as `1e400` into `Infinity`. Protocol 1.0 rejects both before any
 * value reaches the schema evaluator. This parser also enforces the negotiated
 * nesting ceiling while reading, so an oversized document is never retained.
 */

export class StrictJsonError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StrictJsonError";
  }
}

const NUMBER = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/u;

function hexValue(text: string): number {
  return Number.parseInt(text, 16);
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

export function parseStrictJson(text: string, maxDepth: number): unknown {
  let index = 0;

  const fail = (message: string): never => {
    throw new StrictJsonError(`${message} at byte ${index}`);
  };

  const skipWhitespace = (): void => {
    while (index < text.length) {
      const code = text.charCodeAt(index);
      if (code !== 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
        return;
      }
      index += 1;
    }
  };

  const readString = (): string => {
    if (text.charCodeAt(index) !== 0x22) {
      fail("expected string");
    }
    index += 1;
    let out = "";
    while (index < text.length) {
      const code = text.charCodeAt(index);
      if (code === 0x22) {
        index += 1;
        return out;
      }
      if (code < 0x20) {
        fail("unescaped control character in string");
      }
      if (code === 0x5c) {
        index += 1;
        const escape = text[index];
        if (escape === undefined) {
          fail("unterminated escape");
        }
        index += 1;
        switch (escape) {
          case '"':
          case "\\":
          case "/":
            out += escape;
            break;
          case "b":
            out += "\b";
            break;
          case "f":
            out += "\f";
            break;
          case "n":
            out += "\n";
            break;
          case "r":
            out += "\r";
            break;
          case "t":
            out += "\t";
            break;
          case "u": {
            const digits = text.slice(index, index + 4);
            if (!/^[0-9A-Fa-f]{4}$/u.test(digits)) {
              fail("invalid unicode escape");
            }
            index += 4;
            const value = hexValue(digits);
            if (isHighSurrogate(value)) {
              if (text[index] !== "\\" || text[index + 1] !== "u") {
                fail("unpaired high surrogate");
              }
              const lowDigits = text.slice(index + 2, index + 6);
              if (!/^[0-9A-Fa-f]{4}$/u.test(lowDigits)) {
                fail("invalid low surrogate escape");
              }
              const low = hexValue(lowDigits);
              if (!isLowSurrogate(low)) {
                fail("unpaired high surrogate");
              }
              index += 6;
              out += String.fromCharCode(value, low);
            } else if (isLowSurrogate(value)) {
              fail("unpaired low surrogate");
            } else {
              out += String.fromCharCode(value);
            }
            break;
          }
          default:
            fail("invalid escape");
        }
        continue;
      }
      out += text[index];
      index += 1;
    }
    return fail("unterminated string");
  };

  const readNumber = (): number => {
    const match = NUMBER.exec(text.slice(index));
    if (match === null) {
      return fail("invalid number");
    }
    const token = match[0];
    index += token.length;
    const value = Number(token);
    if (!Number.isFinite(value)) {
      fail("nonfinite number");
    }
    return value;
  };

  const readLiteral = (literal: string, value: boolean | null): boolean | null => {
    if (!text.startsWith(literal, index)) {
      fail(`expected ${literal}`);
    }
    index += literal.length;
    return value;
  };

  const readValue = (depth: number): unknown => {
    skipWhitespace();
    const character = text[index];
    if (character === undefined) {
      return fail("unexpected end of input");
    }
    if (character === "{") {
      if (depth >= maxDepth) {
        fail("JSON depth limit");
      }
      index += 1;
      const object: Record<string, unknown> = {};
      const keys = new Set<string>();
      skipWhitespace();
      if (text[index] === "}") {
        index += 1;
        return object;
      }
      for (;;) {
        skipWhitespace();
        const key = readString();
        if (keys.has(key)) {
          fail("duplicate JSON key");
        }
        keys.add(key);
        skipWhitespace();
        if (text[index] !== ":") {
          fail("expected colon");
        }
        index += 1;
        object[key] = readValue(depth + 1);
        skipWhitespace();
        if (text[index] === ",") {
          index += 1;
          continue;
        }
        if (text[index] === "}") {
          index += 1;
          return object;
        }
        fail("expected comma or closing brace");
      }
    }
    if (character === "[") {
      if (depth >= maxDepth) {
        fail("JSON depth limit");
      }
      index += 1;
      const array: unknown[] = [];
      skipWhitespace();
      if (text[index] === "]") {
        index += 1;
        return array;
      }
      for (;;) {
        array.push(readValue(depth + 1));
        skipWhitespace();
        if (text[index] === ",") {
          index += 1;
          continue;
        }
        if (text[index] === "]") {
          index += 1;
          return array;
        }
        fail("expected comma or closing bracket");
      }
    }
    if (character === '"') {
      return readString();
    }
    if (character === "-" || (character >= "0" && character <= "9")) {
      return readNumber();
    }
    if (character === "t") {
      return readLiteral("true", true);
    }
    if (character === "f") {
      return readLiteral("false", false);
    }
    if (character === "n") {
      return readLiteral("null", null);
    }
    return fail("unexpected token");
  };

  const value = readValue(0);
  skipWhitespace();
  if (index !== text.length) {
    fail("trailing data");
  }
  return value;
}

/** Depth of containers in an already parsed value, matching the parser's count. */
export function containerDepth(value: unknown): number {
  if (Array.isArray(value)) {
    let deepest = 0;
    for (const child of value) {
      deepest = Math.max(deepest, containerDepth(child));
    }
    return deepest + 1;
  }
  if (typeof value === "object" && value !== null) {
    let deepest = 0;
    for (const child of Object.values(value)) {
      deepest = Math.max(deepest, containerDepth(child));
    }
    return deepest + 1;
  }
  return 0;
}
