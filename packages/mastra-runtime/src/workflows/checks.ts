/**
 * Exact stage checks over #9's read-only native inspection.
 *
 * Requirements are fixed before planning. The model cannot add checks, mark
 * them optional, change a protected baseline or widen a tolerance. Missing or
 * non-finite native evidence never passes. Display meshes are never used.
 */

import { invalidRequest, ProtocolError, asProtocolError } from "../errors.js";
import { isObject, type Limits, DEFAULT_LIMITS } from "../limits.js";
import type { JsonObject } from "../protocol.js";
import type {
  CadPort,
  CheckRequirement,
  CheckResult,
  ExactMeasurement,
  FixedRequirements,
  ParameterRequirement,
} from "./types.js";

export function validateRequirements(requirements: FixedRequirements, limits: Limits = DEFAULT_LIMITS): FixedRequirements {
  if (!Array.isArray(requirements.checks)) {
    throw invalidRequest("Fixed requirements need a check list.", { field: "requirements" });
  }
  if (requirements.checks.length > limits.checks) {
    throw invalidRequest("The fixed requirements exceed the check limit.", { field: "requirements" });
  }
  const ids = new Set<string>();
  const checks: CheckRequirement[] = [];
  for (const check of requirements.checks) {
    if (!isObject(check)) {
      throw invalidRequest("A fixed check is not an object.", { field: "requirements" });
    }
    const checkId = check["checkId"];
    const kind = check["kind"];
    const required = check["required"];
    if (typeof checkId !== "string" || checkId.length === 0 || ids.has(checkId)) {
      throw invalidRequest("Fixed check ids must be unique and nonempty.", { field: "requirements" });
    }
    if (typeof kind !== "string" || typeof required !== "boolean" || !isObject(check["parameters"])) {
      throw invalidRequest(`Fixed check \`${checkId}\` is malformed.`, { field: "requirements" });
    }
    ids.add(checkId);
    validateCheckParameters(checkId, kind, check["parameters"]);
    checks.push({
      checkId,
      kind,
      required,
      parameters: freezeJson(check["parameters"]),
    });
  }
  return { checks };
}

function validateCheckParameters(checkId: string, kind: string, parameters: JsonObject): void {
  if (kind === "parameters.required") {
    const entries = parameters["requirements"];
    if (!Array.isArray(entries) || entries.length === 0 || entries.length > 64) {
      throw invalidRequest(`Check \`${checkId}\` needs 1..64 parameter requirements.`, { field: "requirements" });
    }
    for (const entry of entries) {
      if (!isObject(entry) || typeof entry["name"] !== "string" || entry["name"].length === 0) {
        throw invalidRequest(`Check \`${checkId}\` has a parameter without a name.`, { field: "requirements" });
      }
      if (!finite(entry["value"]) || !finite(entry["absTolerance"]) || (entry["absTolerance"] as number) < 0) {
        throw invalidRequest(`Check \`${checkId}\` has a non-finite value or tolerance.`, { field: "requirements" });
      }
    }
    return;
  }
  if (kind === "geometry.exact") {
    const entries = parameters["measurements"];
    if (!Array.isArray(entries) || entries.length === 0 || entries.length > 64) {
      throw invalidRequest(`Check \`${checkId}\` needs 1..64 exact measurements.`, { field: "requirements" });
    }
    for (const entry of entries) {
      if (!isObject(entry) || typeof entry["command"] !== "string" || entry["command"].length === 0) {
        throw invalidRequest(`Check \`${checkId}\` has a measurement without a read-only command.`, {
          field: "requirements",
        });
      }
      if (typeof entry["resultPointer"] !== "string" || entry["resultPointer"].length === 0) {
        throw invalidRequest(`Check \`${checkId}\` has a measurement without a result pointer.`, {
          field: "requirements",
        });
      }
      if (!finite(entry["expected"]) || !finite(entry["absTolerance"]) || !finite(entry["relTolerance"])) {
        throw invalidRequest(`Check \`${checkId}\` has a non-finite expected value or tolerance.`, {
          field: "requirements",
        });
      }
      if ((entry["absTolerance"] as number) < 0 || (entry["relTolerance"] as number) < 0) {
        throw invalidRequest(`Check \`${checkId}\` has a negative tolerance.`, { field: "requirements" });
      }
      if (entry["params"] !== undefined && !isObject(entry["params"])) {
        throw invalidRequest(`Check \`${checkId}\` measurement parameters must be an object.`, { field: "requirements" });
      }
    }
    return;
  }
  // Unknown check kinds are retained as fixed but can never count as a pass.
}

/** Read the exact native evidence a profile asks for. */
export async function evaluateFixedChecks(cad: CadPort, requirements: FixedRequirements): Promise<CheckResult[]> {
  let parameters: Readonly<Record<string, number | null>> | null = null;
  const results: CheckResult[] = [];
  for (const check of requirements.checks) {
    if (check.kind === "parameters.required") {
      if (parameters === null) {
        try {
          parameters = await readParameters(cad);
        } catch (error) {
          const wire = asProtocolError(error);
          results.push({
            checkId: check.checkId,
            required: check.required,
            state: "error",
            expected: { requirements: check.parameters["requirements"] ?? [] },
            actual: {},
            detail: `Native parameter inspection failed: ${wire.message}`,
          });
          parameters = {};
          continue;
        }
      }
      results.push(evaluateParameterCheck(check, parameters));
      continue;
    }
    if (check.kind === "geometry.exact") {
      results.push(await evaluateMeasurementCheck(cad, check));
      continue;
    }
    results.push({
      checkId: check.checkId,
      required: check.required,
      state: "unsupported",
      expected: {},
      actual: {},
      detail: `Check kind \`${check.kind}\` is not implemented by this workflow.`,
    });
  }
  return results;
}

export function requiredChecksPassed(results: readonly CheckResult[]): boolean {
  return results.filter((result) => result.required).every((result) => result.state === "passed");
}

export function failedCheckIds(results: readonly CheckResult[]): string[] {
  return results
    .filter((result) => result.required && result.state !== "passed")
    .map((result) => result.checkId);
}

function evaluateParameterCheck(
  check: CheckRequirement,
  parameters: Readonly<Record<string, number | null>>,
): CheckResult {
  const entries = check.parameters["requirements"];
  const requirements = Array.isArray(entries) ? entries.filter(isParameterRequirement) : [];
  const expected: JsonObject = {};
  const actual: JsonObject = {};
  const mismatches: string[] = [];
  for (const requirement of requirements) {
    const value = parameters[requirement.name];
    expected[requirement.name] = requirement.value;
    actual[requirement.name] = value ?? null;
    if (typeof value !== "number" || !Number.isFinite(value)) {
      mismatches.push(`${requirement.name} is missing`);
      continue;
    }
    if (Math.abs(value - requirement.value) > requirement.absTolerance) {
      mismatches.push(
        `${requirement.name}=${value} is outside ${requirement.value}±${requirement.absTolerance}`,
      );
    }
  }
  return {
    checkId: check.checkId,
    required: check.required,
    state: mismatches.length === 0 ? "passed" : "failed",
    expected,
    actual,
    detail: mismatches.length === 0 ? "Parameter requirements match the exact native values." : mismatches.join("; "),
  };
}

async function evaluateMeasurementCheck(cad: CadPort, check: CheckRequirement): Promise<CheckResult> {
  const entries = check.parameters["measurements"];
  const measurements = Array.isArray(entries) ? entries.filter(isExactMeasurement) : [];
  const expected: JsonObject = {};
  const actual: JsonObject = {};
  const mismatches: string[] = [];
  let readFailure: string | null = null;
  for (const measurement of measurements) {
    const key = `${measurement.command}:${measurement.resultPointer}`;
    expected[key] = measurement.expected;
    try {
      const reply = await cad.readTool(measurement.command, measurement.params);
      const body = replyValue(reply);
      const value = resolvePointer(body, measurement.resultPointer);
      actual[key] = typeof value === "number" && Number.isFinite(value) ? value : null;
      if (typeof value !== "number" || !Number.isFinite(value)) {
        mismatches.push(`${key} produced no finite exact value`);
        continue;
      }
      const tolerance = Math.max(measurement.absTolerance, measurement.relTolerance * Math.abs(measurement.expected));
      if (Math.abs(value - measurement.expected) > tolerance) {
        mismatches.push(`${key}=${value} is outside ${measurement.expected}±${tolerance}`);
      }
    } catch (error) {
      const wire = error instanceof ProtocolError ? error : asProtocolError(error);
      actual[key] = null;
      readFailure = `Native measurement \`${measurement.command}\` failed: ${wire.message}`;
      mismatches.push(readFailure);
    }
  }
  if (measurements.length === 0) {
    mismatches.push("No usable exact measurements are configured.");
  }
  const state: CheckResult["state"] =
    mismatches.length === 0 ? "passed" : readFailure !== null ? "unsupported" : "failed";
  return {
    checkId: check.checkId,
    required: check.required,
    state,
    expected,
    actual,
    detail: mismatches.length === 0 ? "Exact native measurements match the fixed requirements." : mismatches.join("; "),
  };
}

async function readParameters(cad: CadPort): Promise<Readonly<Record<string, number | null>>> {
  const reply = await cad.readTool("inspect_design", { measure: true, sketches: false });
  const body = replyValue(reply);
  return parameterMap(body);
}

export function parameterMap(design: JsonObject | null): Readonly<Record<string, number | null>> {
  const result: Record<string, number | null> = {};
  if (design === null) {
    return result;
  }
  // The native `document.inspect` reply uses `params` (crates/engine inspect).
  // `parameters` stays only as a compatibility fallback for older fixtures.
  const raw = design["params"] ?? design["parameters"];
  if (Array.isArray(raw)) {
    for (const entry of raw) {
      if (!isObject(entry) || typeof entry["name"] !== "string") {
        continue;
      }
      const value = entry["value"];
      result[entry["name"]] = typeof value === "number" && Number.isFinite(value) ? value : null;
    }
    return result;
  }
  if (isObject(raw)) {
    for (const [name, entry] of Object.entries(raw)) {
      if (typeof entry === "number" && Number.isFinite(entry)) {
        result[name] = entry;
      } else if (isObject(entry) && typeof entry["value"] === "number" && Number.isFinite(entry["value"])) {
        result[name] = entry["value"];
      } else {
        result[name] = null;
      }
    }
  }
  return result;
}

export function replyValue(reply: { readonly value: JsonObject | null; readonly text: string | null }): JsonObject | null {
  if (reply.value !== null) {
    return reply.value;
  }
  if (reply.text === null || reply.text.length === 0) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(reply.text);
    return isObject(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function resolvePointer(value: unknown, pointer: string): unknown {
  if (pointer.length === 0) {
    return value;
  }
  let rest = pointer;
  if (rest.startsWith("/")) {
    rest = rest.slice(1);
  }
  const parts = rest.split(/[./]/u).filter((part) => part.length > 0);
  let current: unknown = value;
  for (const raw of parts) {
    const part = raw.replaceAll("~1", "/").replaceAll("~0", "~");
    if (Array.isArray(current)) {
      const index = Number(part);
      if (!Number.isInteger(index) || index < 0 || index >= current.length) {
        return undefined;
      }
      current = current[index];
      continue;
    }
    if (!isObject(current)) {
      return undefined;
    }
    current = current[part];
  }
  return current;
}

function isParameterRequirement(value: unknown): value is ParameterRequirement {
  return (
    isObject(value) &&
    typeof value["name"] === "string" &&
    finite(value["value"]) &&
    finite(value["absTolerance"])
  );
}

function isExactMeasurement(value: unknown): value is ExactMeasurement {
  return (
    isObject(value) &&
    typeof value["command"] === "string" &&
    typeof value["resultPointer"] === "string" &&
    finite(value["expected"]) &&
    finite(value["absTolerance"]) &&
    finite(value["relTolerance"]) &&
    (value["params"] === undefined || isObject(value["params"]))
  );
}

function finite(value: unknown): boolean {
  return typeof value === "number" && Number.isFinite(value);
}

function freezeJson(value: JsonObject): JsonObject {
  return JSON.parse(JSON.stringify(value)) as JsonObject;
}
