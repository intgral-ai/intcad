/**
 * Protocol 1.0 finite-resource ceilings (contract section 4). These are the
 * hard defaults; a host may negotiate lower values in `runtime.hello`.
 */

export interface Limits {
  frameBytes: number;
  jsonDepth: number;
  queueBytes: number;
  controlLaneBytes: number;
  pendingRequests: number;
  controlSlots: number;
  requestLedgerEntries: number;
  runtimeInstances: number;
  sessions: number;
  activeTurns: number;
  stderrLines: number;
  stderrLineScalars: number;
  stderrBytes: number;
  modelMessages: number;
  toolDefinitions: number;
  toolCalls: number;
  textBytes: number;
  toolJsonBytes: number;
  attachments: number;
  attachmentBytes: number;
  attachmentTotalBytes: number;
  historyBytes: number;
  historyMessages: number;
  continuationHandles: number;
  continuationBytes: number;
  continuationTtlSeconds: number;
  httpPerConnection: number;
  httpPerHost: number;
  cadWritesInFlight: number;
  cadReadsInFlight: number;
  handshakeSeconds: number;
  sessionStartSeconds: number;
  callDefaultSeconds: number;
  callMaxSeconds: number;
  cancelAckSeconds: number;
  shutdownSeconds: number;
  validationWorkers: number;
  validationQueue: number;
  validationSeconds: number;
  validationQueueSeconds: number;
  snapshotBytes: number;
  snapshotInputs: number;
  manifestBytes: number;
  reportBytes: number;
  checks: number;
  evidenceArtifacts: number;
  evidenceBytes: number;
  evidenceTotalBytes: number;
  /** Canonical decimal string; 8 GiB exceeds the protocol integer range. */
  storeBytes: string;
  jobRecords: number;
  retentionSeconds: number;
}

export interface Budgets {
  modelCalls: number;
  agentTasks: number;
  cadCommands: number;
  repairs: number;
  validationRuns: number;
  wallTimeSeconds: number;
}

export const DEFAULT_LIMITS: Readonly<Limits> = Object.freeze({
  frameBytes: 8_388_608,
  jsonDepth: 64,
  queueBytes: 16_777_216,
  controlLaneBytes: 65_536,
  pendingRequests: 32,
  controlSlots: 1,
  requestLedgerEntries: 4_096,
  runtimeInstances: 8,
  sessions: 1,
  activeTurns: 1,
  stderrLines: 40,
  stderrLineScalars: 400,
  stderrBytes: 65_536,
  modelMessages: 128,
  toolDefinitions: 64,
  toolCalls: 32,
  textBytes: 262_144,
  toolJsonBytes: 65_536,
  attachments: 4,
  attachmentBytes: 4_194_304,
  attachmentTotalBytes: 16_777_216,
  historyBytes: 16_777_216,
  historyMessages: 128,
  continuationHandles: 32,
  continuationBytes: 16_777_216,
  continuationTtlSeconds: 86_400,
  httpPerConnection: 2,
  httpPerHost: 8,
  cadWritesInFlight: 1,
  cadReadsInFlight: 4,
  handshakeSeconds: 10,
  sessionStartSeconds: 30,
  callDefaultSeconds: 300,
  callMaxSeconds: 600,
  cancelAckSeconds: 5,
  shutdownSeconds: 3,
  validationWorkers: 2,
  validationQueue: 8,
  validationSeconds: 600,
  validationQueueSeconds: 300,
  snapshotBytes: 1_073_741_824,
  snapshotInputs: 256,
  manifestBytes: 1_048_576,
  reportBytes: 4_194_304,
  checks: 128,
  evidenceArtifacts: 64,
  evidenceBytes: 16_777_216,
  evidenceTotalBytes: 67_108_864,
  storeBytes: "8589934592",
  jobRecords: 10_000,
  retentionSeconds: 604_800,
});

export const DEFAULT_BUDGETS: Readonly<Budgets> = Object.freeze({
  modelCalls: 20,
  agentTasks: 4,
  cadCommands: 64,
  repairs: 2,
  validationRuns: 3,
  wallTimeSeconds: 1_200,
});

export const LIMIT_KEYS = Object.freeze(Object.keys(DEFAULT_LIMITS).sort() as readonly (keyof Limits)[]);

const MINIMUM_CONTROL_FRAME_BYTES = 4_096;

/** Cross-field limits that JSON Schema cannot express (contract section 4). */
export function limitsValid(limits: Limits): boolean {
  const dataLane = limits.queueBytes - limits.controlLaneBytes;
  const ledgerReserve = (limits.pendingRequests + limits.controlSlots) * 2 + 8;
  return (
    dataLane >= limits.frameBytes &&
    limits.frameBytes >= MINIMUM_CONTROL_FRAME_BYTES &&
    limits.controlLaneBytes >= MINIMUM_CONTROL_FRAME_BYTES &&
    limits.requestLedgerEntries > ledgerReserve &&
    limits.callDefaultSeconds <= limits.callMaxSeconds &&
    limits.httpPerConnection <= limits.httpPerHost &&
    limits.attachmentBytes <= limits.attachmentTotalBytes &&
    limits.evidenceBytes <= limits.evidenceTotalBytes
  );
}

/** Lower a host-offered limit set. A runtime may only narrow, never raise, a ceiling. */
export function narrowerLimits(host: Limits): Limits {
  const result = { ...DEFAULT_LIMITS };
  for (const key of LIMIT_KEYS) {
    if (key === "storeBytes") {
      result.storeBytes = (BigInt(host.storeBytes) < BigInt(DEFAULT_LIMITS.storeBytes) ? host.storeBytes : DEFAULT_LIMITS.storeBytes);
    } else {
      result[key] = Math.min(host[key], DEFAULT_LIMITS[key]);
    }
  }
  return result;
}

export function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
