/**
 * TypeScript conformance against `schema/mastra-runtime/v1/cases.json`.
 *
 * What this layer owns: JSON shape (locked Draft 2020-12 evaluator), byte
 * framing, duplicate keys, UTF-8, nonfinite numbers, nesting/size ceilings,
 * envelope direction/scope, parameter byte caps, tool-result identity and the
 * manifest/path rules. It deliberately does not pretend to run the Rust host
 * state machine, so `transcript` cases and raw native/guard carriers are
 * reported as skipped rather than claimed as passing.
 */

import { DEFAULT_LIMITS, type Limits } from "./limits.js";
import {
  decodeFrameBytes,
  SchemaBundle,
  validateDataValue,
  type DecodeResult,
  type SchemaCase,
} from "./protocol.js";
import { parseStrictJson } from "./strict-json.js";

export interface ConformanceReport {
  readonly bundleDigest: string;
  readonly total: number;
  readonly checked: number;
  readonly passed: number;
  readonly skipped: number;
  readonly byKind: Readonly<Record<string, number>>;
  readonly skippedKinds: Readonly<Record<string, number>>;
  readonly failures: readonly string[];
}

function decodeOutcome(bundle: SchemaBundle, raw: Buffer, item: SchemaCase, limits: Limits): DecodeResult {
  return decodeFrameBytes(raw, item.phase === "bootstrap" ? "bootstrap" : "negotiated", item.sender, limits, bundle);
}

function expectedHandling(result: DecodeResult): { handling: string; code: string | null } {
  if (result.kind === "frame") {
    return { handling: "accept", code: null };
  }
  if (result.kind === "request_error") {
    return {
      handling: result.code === "METHOD_NOT_FOUND" ? "method_not_found" : "invalid_request",
      code: result.code,
    };
  }
  return { handling: "transport_error", code: "TRANSPORT_ERROR" };
}

export function runConformance(bundle: SchemaBundle, limits: Limits = DEFAULT_LIMITS): ConformanceReport {
  const failures: string[] = [];
  const byKind: Record<string, number> = {};
  const skippedKinds: Record<string, number> = {};
  let checked = 0;
  let passed = 0;
  let skipped = 0;

  for (const item of bundle.cases) {
    byKind[item.kind] = (byKind[item.kind] ?? 0) + 1;
    const raw = bundle.read(item.path);

    // Shape is a separate layer. Raw .ndjson/.bin fixtures are parsed only to
    // ask the published schema; the schema never sees framing bytes.
    if (item.schemaRef !== null) {
      let shape = false;
      try {
        const text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
        const value = parseStrictJson(text, Number.MAX_SAFE_INTEGER);
        shape = bundle.validate(item.schemaRef, value);
      } catch {
        shape = false;
      }
      const expected = item.shapeExpected === "valid";
      if (shape !== expected) {
        failures.push(`${item.id}: schema acceptance ${shape ? "true" : "false"} != shapeExpected ${item.shapeExpected}`);
      }
    } else if (item.shapeExpected !== "not_applicable") {
      failures.push(`${item.id}: missing schema reference but shapeExpected is ${item.shapeExpected}`);
    }

    if (item.kind === "transcript") {
      skipped += 1;
      skippedKinds["transcript"] = (skippedKinds["transcript"] ?? 0) + 1;
      if (item.handlingExpected !== "transcript") {
        failures.push(`${item.id}: transcript case must expect transcript handling`);
      }
      continue;
    }
    if (item.id.startsWith("guard-") || item.id.startsWith("native-stamp-")) {
      // Raw #8 carriers and native revisions have no TypeScript-side protocol
      // decision. Their catalog expectation is `accept`; do not invent one.
      skipped += 1;
      skippedKinds[item.kind] = (skippedKinds[item.kind] ?? 0) + 1;
      if (item.handlingExpected !== "accept") {
        failures.push(`${item.id}: non-protocol carrier must expect accept`);
      }
      continue;
    }
    if (item.schemaRef === null && item.kind === "data") {
      failures.push(`${item.id}: data case without a schema reference is not a TypeScript decision`);
      continue;
    }

    checked += 1;
    let handling: string;
    let code: string | null;
    const schemaRef = item.schemaRef;
    if (item.kind === "data") {
      let value: unknown;
      try {
        value = parseStrictJson(raw.toString("utf8"), Number.MAX_SAFE_INTEGER);
      } catch {
        value = undefined;
      }
      const valid = value !== undefined && schemaRef !== null && bundle.validate(schemaRef, value) && validateDataValue(schemaRef, value, limits);
      handling = valid ? "accept" : "invalid_request";
      code = valid ? null : "INVALID_REQUEST";
    } else {
      const outcome = decodeOutcome(bundle, raw, item, limits);
      const mapped = expectedHandling(outcome);
      handling = mapped.handling;
      code = mapped.code;
    }

    if (handling !== item.handlingExpected || code !== item.errorCode) {
      failures.push(`${item.id}: handling ${handling}/${code ?? "null"} != ${item.handlingExpected}/${item.errorCode ?? "null"}`);
    } else {
      passed += 1;
    }
  }

  return {
    bundleDigest: bundle.bundleDigest,
    total: bundle.cases.length,
    checked,
    passed,
    skipped,
    byKind,
    skippedKinds,
    failures,
  };
}

export function formatReport(report: ConformanceReport): string {
  const kinds = Object.entries(report.byKind)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([kind, count]) => `${kind}=${count}`)
    .join(" ");
  const skipped = Object.entries(report.skippedKinds)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([kind, count]) => `${kind}=${count}`)
    .join(" ");
  const lines = [
    `schema bundle: ${report.bundleDigest}`,
    `cases: total=${report.total} checked=${report.checked} passed=${report.passed} failed=${report.failures.length} skipped=${report.skipped} (${kinds})`,
    `skipped layers (not TypeScript decisions): ${skipped}`,
  ];
  if (report.failures.length > 0) {
    lines.push(...report.failures.map((failure) => `FAIL ${failure}`));
  }
  return lines.join("\n");
}
