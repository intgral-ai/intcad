/**
 * Turn-local budget accounting (contract section 4).
 *
 * Model calls, ACP tasks, CAD command applications, repairs and validation
 * submissions are reserved before work starts and stay separate. Wall time is
 * checked against the host-frozen deadline before dispatch and after replies.
 */

import { monotonicClock, type Clock } from "../clock.js";
import { resourceExhausted } from "../errors.js";
import type { Budgets } from "../limits.js";
import type { BudgetUsage } from "./types.js";

export class BudgetMeter {
  readonly #budgets: Budgets;
  readonly #clock: Clock;
  readonly #deadline: number;
  #modelCalls = 0;
  #agentTasks = 0;
  #cadCommands = 0;
  #repairs = 0;
  #validationRuns = 0;

  constructor(budgets: Budgets, clock: Clock = monotonicClock) {
    this.#budgets = budgets;
    this.#clock = clock;
    this.#deadline = clock.now() + Math.max(0, budgets.wallTimeSeconds) * 1_000;
  }

  get usage(): BudgetUsage {
    return {
      modelCalls: this.#modelCalls,
      agentTasks: this.#agentTasks,
      cadCommands: this.#cadCommands,
      repairs: this.#repairs,
      validationRuns: this.#validationRuns,
    };
  }

  get remainingMilliseconds(): number {
    return Math.max(0, this.#deadline - this.#clock.now());
  }

  remainingSeconds(): number {
    const seconds = Math.floor(this.remainingMilliseconds / 1_000);
    if (seconds < 1) {
      throw resourceExhausted("wallTimeSeconds", this.#budgets.wallTimeSeconds);
    }
    return seconds;
  }

  assertTime(): void {
    if (this.#clock.now() >= this.#deadline) {
      throw resourceExhausted("wallTimeSeconds", this.#budgets.wallTimeSeconds);
    }
  }

  reserveModel(): void {
    if (this.#modelCalls >= this.#budgets.modelCalls) {
      throw resourceExhausted("modelCalls", this.#budgets.modelCalls);
    }
    this.#modelCalls += 1;
  }

  reserveAgentTask(): void {
    if (this.#agentTasks >= this.#budgets.agentTasks) {
      throw resourceExhausted("agentTasks", this.#budgets.agentTasks);
    }
    this.#agentTasks += 1;
  }

  /** Reserve every command in a batch before the batch is dispatched. */
  reserveCad(count: number): void {
    if (!Number.isInteger(count) || count < 0) {
      throw resourceExhausted("cadCommands", this.#budgets.cadCommands);
    }
    if (this.#cadCommands + count > this.#budgets.cadCommands) {
      throw resourceExhausted("cadCommands", this.#budgets.cadCommands);
    }
    this.#cadCommands += count;
  }

  tryReserveRepair(): boolean {
    if (this.#repairs >= this.#budgets.repairs) {
      return false;
    }
    this.#repairs += 1;
    return true;
  }

  reserveValidation(): void {
    if (this.#validationRuns >= this.#budgets.validationRuns) {
      throw resourceExhausted("validationRuns", this.#budgets.validationRuns);
    }
    this.#validationRuns += 1;
  }
}
