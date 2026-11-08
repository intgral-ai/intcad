/**
 * Ordered native application through #9's guarded adapter.
 *
 * The workflow never turns a proposal into "applied": it keeps the adapter's
 * `WriteOutcome` facts, including command failure, rollback, expectation
 * mismatch, pending, stale, cancellation and unknown effects.
 */

import { ProtocolError } from "../errors.js";
import type { JsonObject } from "../protocol.js";
import type { CadWriteResult } from "../solidcraft/cad-adapter.js";
import type { ApplyProposal } from "./proposals.js";
import type { CadPort, CheckResult } from "./types.js";

export async function applyProposal(cad: CadPort, proposal: ApplyProposal): Promise<CadWriteResult> {
  if (proposal.commands.length === 1) {
    const step = proposal.commands[0];
    if (step === undefined) {
      throw new ProtocolError("INTERNAL_ERROR", "A one-command proposal has no command.");
    }
    return cad.execute(step.command, step.params ?? {});
  }
  return cad.batch(proposal.commands, { rollback: true });
}

export function writeOutput(write: CadWriteResult, checks: readonly CheckResult[] = []): JsonObject {
  const outcome = write.outcome;
  const output: JsonObject = {
    ok: outcome.state === "applied" && checks.every((check) => !check.required || check.state === "passed"),
    state: outcome.state,
    before: outcome.before,
    after: outcome.after,
    completed: outcome.completed,
    rollback: outcome.rollback,
    expectation: outcome.expectation,
    error: outcome.error,
  };
  if (checks.length > 0) {
    output["checks"] = checks.map((check) => ({
      checkId: check.checkId,
      required: check.required,
      state: check.state,
      expected: check.expected,
      actual: check.actual,
      detail: check.detail,
    }));
  }
  const failed = write.evidence.failed;
  if (failed !== null) {
    output["failed"] = failed;
  }
  return output;
}

export function failedCheckSummary(checks: readonly CheckResult[]): string {
  return checks
    .filter((check) => check.required && check.state !== "passed")
    .map((check) => `${check.checkId} (${check.state}): ${check.detail}`)
    .join("; ");
}

export function rollbackRestored(write: CadWriteResult): boolean {
  return write.outcome.rollback.requested && write.outcome.rollback.restored === true;
}
