/**
 * Snapshot validation stage (contract §1.5 and §9).
 *
 * After native apply/check, the workflow:
 *   1. captures one immutable candidate through the guarded
 *      `document.snapshot.capture` command;
 *   2. submits it under a fresh durable `<unix-ms>:<uuid-v4>` key;
 *   3. polls `validation.status`, then fetches `validation.report`;
 *   4. verifies the report's run/candidate/manifest/profile/runner identity,
 *      its terminal state and its required check results.
 *
 * The host still owns the authoritative §9.3 check and the live document stamp.
 * A report is evidence; it is never success text and never a substitute for a
 * later candidate.
 */

import { randomUUID } from "node:crypto";

import type { Clock } from "../clock.js";
import { asProtocolError, cancelled, ProtocolError } from "../errors.js";
import { isObject } from "../limits.js";
import type { CadPort } from "./types.js";
import type {
  ArtifactRef,
  CandidateRef,
  ProfileRef,
  ValidationCheckResult,
  ValidationOutcome,
  ValidationPort,
  ValidationReport,
  ValidationReportResult,
  ValidationStatusResult,
} from "./types.js";

const CANDIDATE_COMMAND = "document.snapshot.capture";
const DEFAULT_POLL_MILLISECONDS = 200;
const SUBMIT_TRIES = 2;
const REPORT_TRIES = 5;

export interface ValidationStageOptions {
  readonly port: ValidationPort;
  readonly profile: ProfileRef;
  readonly signal: AbortSignal;
  readonly clock: Clock;
  /** Absolute monotonic deadline shared with the turn's wall-clock budget. */
  readonly deadlineMilliseconds: number;
  /** Tests inject an immediate sleep. */
  readonly sleep?: (milliseconds: number) => Promise<void>;
  readonly pollMilliseconds?: number;
  /** Wall-clock milliseconds for the durable submission key (default `Date.now`). */
  readonly wallClock?: () => number;
}

function digest(value: unknown): value is string {
  return typeof value === "string" && /^sha256:[0-9a-f]{64}$/u.test(value);
}

function isId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(value);
}

function isArtifact(value: unknown): value is ArtifactRef {
  return (
    isObject(value) &&
    isId(value["artifactId"]) &&
    digest(value["digest"]) &&
    Number.isInteger(value["sizeBytes"]) &&
    (value["sizeBytes"] as number) > 0 &&
    typeof value["mediaType"] === "string" &&
    value["mediaType"].length > 0
  );
}

function isCandidate(value: unknown): value is CandidateRef {
  if (!isObject(value) || !isId(value["candidateId"]) || !digest(value["candidateDigest"]) || !digest(value["manifestDigest"])) {
    return false;
  }
  const source = value["source"];
  return (
    isObject(source) &&
    isId(source["documentId"]) &&
    typeof source["revision"] === "string" &&
    /^(0|[1-9][0-9]*)$/u.test(source["revision"]) &&
    isArtifact(value["artifact"])
  );
}

function sameStamp(a: { documentId: string; revision: string }, b: { documentId: string; revision: string }): boolean {
  return a.documentId === b.documentId && a.revision === b.revision;
}

function sameProfile(a: ProfileRef, b: ProfileRef): boolean {
  return a.profileId === b.profileId && a.version === b.version && a.digest === b.digest;
}

function sameCandidate(a: CandidateRef, b: CandidateRef): boolean {
  return (
    a.candidateId === b.candidateId &&
    sameStamp(a.source, b.source) &&
    a.candidateDigest === b.candidateDigest &&
    a.manifestDigest === b.manifestDigest &&
    a.artifact.artifactId === b.artifact.artifactId &&
    a.artifact.digest === b.artifact.digest &&
    a.artifact.sizeBytes === b.artifact.sizeBytes &&
    a.artifact.mediaType === b.artifact.mediaType
  );
}

function findCandidate(value: unknown, depth = 0): CandidateRef | null {
  if (depth > 4) {
    return null;
  }
  if (Array.isArray(value)) {
    for (const entry of value) {
      const found = findCandidate(entry, depth + 1);
      if (found !== null) {
        return found;
      }
    }
    return null;
  }
  if (!isObject(value)) {
    return null;
  }
  if (isCandidate(value["candidate"])) {
    return value["candidate"];
  }
  if (isCandidate(value)) {
    return value;
  }
  for (const key of ["result", "results", "native", "output"]) {
    const found = findCandidate(value[key], depth + 1);
    if (found !== null) {
      return found;
    }
  }
  return null;
}

function noCandidate(detail: string): ProtocolError {
  return new ProtocolError("INVALID_CANDIDATE", `The snapshot capture returned no usable candidate: ${detail}.`, "never");
}

/**
 * Capture one candidate through the guarded command. The command does not edit
 * the design, so the returned source stamp must still equal the expected stamp.
 */
export async function captureCandidate(cad: CadPort, expected: { documentId: string; revision: string }): Promise<CandidateRef> {
  const write = await cad.execute(CANDIDATE_COMMAND, {});
  if (write.outcome.state !== "applied") {
    const failure = write.outcome.error;
    throw new ProtocolError(
      failure?.code ?? "INVALID_CANDIDATE",
      failure?.message ?? `The guarded snapshot capture ended ${write.outcome.state}.`,
      failure?.retry ?? "never",
      failure?.details ?? {},
    );
  }
  const candidate = findCandidate(write.evidence.native) ?? findCandidate(write.evidence.results);
  if (candidate === null) {
    throw noCandidate("the native result had no candidate object");
  }
  if (!sameStamp(candidate.source, expected)) {
    throw new ProtocolError(
      "STALE_DOCUMENT",
      "The snapshot was captured from a different live state than the workflow's frozen stamp.",
      "never",
      { expected, current: candidate.source },
    );
  }
  return candidate;
}

function terminal(state: ValidationStatusResult["state"]): boolean {
  return state !== "queued" && state !== "running";
}

function retryable(error: unknown): boolean {
  const wire = asProtocolError(error);
  return wire.value.code === "TIMEOUT" || wire.value.code === "TRANSPORT_ERROR" || wire.value.code === "OUTCOME_UNKNOWN";
}

function retryableRead(error: unknown): boolean {
  const wire = asProtocolError(error);
  return wire.value.code === "NOT_READY" || retryable(error);
}

function checkReport(
  report: ValidationReport,
  runId: string,
  candidate: CandidateRef,
  profile: ProfileRef,
): string | null {
  if (report.schemaVersion !== "1.0") {
    return "the report schema version is not 1.0";
  }
  if (report.runId !== runId) {
    return "the report belongs to another run";
  }
  if (report.candidateId !== candidate.candidateId) {
    return "the report names another candidate";
  }
  if (report.candidateDigest !== candidate.candidateDigest || report.manifestDigest !== candidate.manifestDigest) {
    return "the report binds different candidate bytes";
  }
  if (!sameProfile(report.profile, profile)) {
    return "the report binds another profile";
  }
  if (
    !isId(report.runner.runnerId) ||
    report.runner.version.length === 0 ||
    report.runner.engineVersion.length === 0
  ) {
    return "the report has no approved runner identity";
  }
  if (!["passed", "quality_failed", "infrastructure_error", "cancelled", "unknown"].includes(report.state)) {
    return "the report has an unknown terminal state";
  }
  const seen = new Set<string>();
  const checks: ValidationCheckResult[] = [];
  for (const result of report.checks) {
    if (!isId(result.checkId) || typeof result.required !== "boolean") {
      return "a report check is malformed";
    }
    if (seen.has(result.checkId)) {
      return `the report repeats check ${result.checkId}`;
    }
    seen.add(result.checkId);
    if (!["passed", "failed", "error", "unsupported", "not_run"].includes(result.state)) {
      return `check ${result.checkId} has an unknown state`;
    }
    if (!Array.isArray(result.evidence) || !result.evidence.every(isArtifact)) {
      return `check ${result.checkId} has unusable evidence`;
    }
    checks.push(result);
  }
  if (report.state !== "passed") {
    return null;
  }
  if (report.error !== null) {
    return "a passing report carries an error";
  }
  const required = checks.filter((result) => result.required);
  if (required.length === 0) {
    return "a passing report has no required check";
  }
  for (const result of required) {
    if (result.state !== "passed") {
      return `required check ${result.checkId} is ${result.state}`;
    }
    if (result.evidence.length === 0) {
      return `required check ${result.checkId} has no evidence`;
    }
  }
  return null;
}

function mismatch(reason: string): ValidationOutcome {
  return {
    state: "infrastructure_error",
    runId: null,
    candidate: null,
    report: null,
    reportBody: null,
    reason: `The validation report cannot be used: ${reason}.`,
    reasonCode: "REPORT_MISMATCH",
  };
}

function defaultSleep(milliseconds: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, milliseconds);
  });
}

/**
 * The workflow entry point: capture, submit, poll and verify one candidate. A
 * quality failure is a reportable result; an infrastructure failure is never a
 * geometry repair.
 */
export async function runValidationStageFor(
  cad: CadPort,
  expected: { documentId: string; revision: string },
  options: ValidationStageOptions,
): Promise<ValidationOutcome> {
  const { port, profile, signal, clock } = options;
  const sleep = options.sleep ?? defaultSleep;
  const pollMilliseconds = options.pollMilliseconds ?? DEFAULT_POLL_MILLISECONDS;
  const wallClock = options.wallClock ?? Date.now;
  if (signal.aborted) {
    throw cancelled("validation");
  }
  const candidate = await captureCandidate(cad, expected);
  return await validateCandidate(port, candidate, profile, options, sleep, pollMilliseconds, wallClock, clock, signal);
}

async function validateCandidate(
  port: ValidationPort,
  candidate: CandidateRef,
  profile: ProfileRef,
  options: ValidationStageOptions,
  sleep: (milliseconds: number) => Promise<void>,
  pollMilliseconds: number,
  wallClock: () => number,
  clock: Clock,
  signal: AbortSignal,
): Promise<ValidationOutcome> {
  const requestId = `${Math.max(0, Math.floor(wallClock()))}:${randomUUID()}`;
  let submitted = null;
  let lastError: unknown = null;
  for (let attempt = 0; attempt < SUBMIT_TRIES; attempt += 1) {
    try {
      submitted = await port.submit(candidate, profile, requestId, signal);
      break;
    } catch (error) {
      lastError = error;
      if (signal.aborted || !retryable(error) || attempt + 1 >= SUBMIT_TRIES) {
        throw error;
      }
      await sleep(pollMilliseconds);
    }
  }
  if (submitted === null) {
    throw asProtocolError(lastError ?? new Error("validation.submit failed"));
  }
  if (!sameCandidate(submitted.candidate, candidate) || !sameProfile(submitted.profile, profile) || !isId(submitted.runId)) {
    return mismatch("the submission response does not echo the submitted candidate and profile");
  }
  const runId = submitted.runId;

  let status: ValidationStatusResult | null = null;
  for (;;) {
    if (signal.aborted) {
      throw cancelled("validation-status");
    }
    let polled: ValidationStatusResult | null = null;
    let statusError: unknown = null;
    for (let attempt = 0; attempt < SUBMIT_TRIES; attempt += 1) {
      try {
        polled = await port.status(runId, signal);
        break;
      } catch (error) {
        statusError = error;
        if (signal.aborted || !retryableRead(error) || attempt + 1 >= SUBMIT_TRIES) {
          break;
        }
        await sleep(pollMilliseconds);
      }
    }
    if (polled === null) {
      return {
        state: "infrastructure_error",
        runId,
        candidate,
        report: null,
        reportBody: null,
        reason: `The validation status is unavailable: ${asProtocolError(statusError ?? new Error("not ready")).message}.`,
        reasonCode: "VALIDATION_STATUS_UNAVAILABLE",
      };
    }
    status = polled;
    if (!sameCandidate(status.candidate, candidate) || !sameProfile(status.profile, profile) || status.runId !== runId) {
      return { ...mismatch("the status response names another run or candidate"), runId };
    }
    if (terminal(status.state)) {
      break;
    }
    if (clock.now() >= options.deadlineMilliseconds) {
      return {
        state: "infrastructure_error",
        runId,
        candidate,
        report: null,
        reportBody: null,
        reason: "The validation run did not finish before the turn's wall clock expired.",
        reasonCode: "VALIDATION_TIMEOUT",
      };
    }
    await sleep(pollMilliseconds);
  }

  let fetched: ValidationReportResult | null = null;
  let reportError: unknown = null;
  for (let attempt = 0; attempt < REPORT_TRIES; attempt += 1) {
    try {
      fetched = await port.report(runId, signal);
      break;
    } catch (error) {
      reportError = error;
      if (signal.aborted || !retryableRead(error) || attempt + 1 >= REPORT_TRIES) {
        break;
      }
      await sleep(pollMilliseconds);
    }
  }
  if (fetched === null) {
    return {
      state: "infrastructure_error",
      runId,
      candidate,
      report: null,
      reportBody: null,
      reason: `The validation report is unavailable: ${asProtocolError(reportError ?? new Error("not ready")).message}.`,
      reasonCode: "VALIDATION_REPORT_UNAVAILABLE",
    };
  }
  if (fetched.report.state !== status.state) {
    return { ...mismatch("the terminal status and the report state disagree"), runId, candidate };
  }
  const problem = checkReport(fetched.report, runId, candidate, profile);
  if (problem !== null) {
    return { ...mismatch(problem), runId, candidate, report: fetched.reference };
  }
  if (fetched.report.state === "passed") {
    return {
      state: "passed",
      runId,
      candidate,
      report: fetched.reference,
      reportBody: fetched.report,
      reason: `Required checks passed for ${candidate.candidateId}.`,
      reasonCode: "VALIDATED_SUCCESS",
    };
  }
  const reasonByState: Record<Exclude<ValidationReport["state"], "passed">, { reason: string; reasonCode: string }> = {
    quality_failed: { reason: `Required checks failed for ${candidate.candidateId}.`, reasonCode: "QUALITY_FAILED" },
    infrastructure_error: { reason: "The local validation runner failed.", reasonCode: "VALIDATION_INFRASTRUCTURE_ERROR" },
    cancelled: { reason: "The validation run was cancelled.", reasonCode: "CANCELLED" },
    unknown: { reason: "The validation run's outcome is unknown.", reasonCode: "OUTCOME_UNKNOWN" },
  };
  const mapped = reasonByState[fetched.report.state];
  return {
    state: fetched.report.state,
    runId,
    candidate,
    report: fetched.reference,
    reportBody: fetched.report,
    reason: fetched.report.error?.message ?? mapped.reason,
    reasonCode: mapped.reasonCode,
  };
}
