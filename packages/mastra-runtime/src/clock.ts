/**
 * Protocol 1.0 deadlines and expiry use a monotonic clock inside the process.
 * Wall-clock time is only for display and for the host's timestamps.
 */

export interface Clock {
  now(): number;
}

export const monotonicClock: Clock = Object.freeze({
  now: () => performance.now(),
});
