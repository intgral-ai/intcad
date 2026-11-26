// What the tickets view's parts share: the words for a state, dates in the person's own form, a
// ticket with the page's new fields filled in, the order a person walks the tickets in, and why a
// step cannot be taken now.
import type { Ticket, TicketStateName, TicketsState } from '../host';
import { STATE_ORDER } from './status-icons';

export type Words = TicketsState['strings'];

type Filled = 'parts' | 'attachments' | 'waits' | 'worker' | 'deleted' | 'checks' | 'checksOff' | 'recheck' | 'finished' | 'whyNotDone' | 'openWaits' | 'unchecked';

/** A ticket with every optional field filled in, as a ticket without it reads. */
export type Full = Ticket & Required<Pick<Ticket, Filled>>;

export const full = (ticket: Ticket): Full => ({
  ...ticket,
  parts: ticket.parts ?? (ticket.part ? [ticket.part] : []),
  attachments: ticket.attachments ?? [],
  waits: ticket.waits ?? [],
  worker: ticket.worker ?? null,
  deleted: ticket.deleted ?? false,
  checks: ticket.checks ?? true,
  checksOff: ticket.checksOff ?? false,
  recheck: ticket.recheck ?? null,
  finished: ticket.finished ?? false,
  whyNotDone: ticket.whyNotDone ?? null,
  openWaits: ticket.openWaits ?? [],
  unchecked: ticket.unchecked ?? false,
});

/** The same limits as the store (crates/engine/src/project/tickets.rs). */
export const MAX_TITLE = 200;
export const MAX_TEXT = 8000;
const MAX_LINES = 40;
const MAX_LINE = 400;

/** Requirement lines as typed, one per line, blank ones dropped; null when they break the store's limits. */
export function lines(typed: string) {
  const kept = typed
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
  return kept.length <= MAX_LINES && kept.every(line => line.length <= MAX_LINE) ? kept : null;
}

export function stateWord(words: Words, state: TicketStateName) {
  const key = { draft: 'stateDraft', confirmed: 'stateConfirmed', in_progress: 'stateInProgress', waiting: 'stateWaiting', done: 'stateDone', blocked: 'stateBlocked' } as const;
  return words[key[state]];
}

/** A word with `{name}` places filled in. */
export function fill(text: string, values: Record<string, string>) {
  return text.replace(/\{(\w+)\}/g, (all, name: string) => values[name] ?? all);
}

function format(at: string, language: string, options: Intl.DateTimeFormatOptions) {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return at;
  try {
    return new Intl.DateTimeFormat(language || undefined, options).format(date);
  } catch {
    return date.toLocaleString();
  }
}

/** The day, as a list row shows it: `11月26日`, `26 Nov`. */
export const day = (at: string, language: string) => format(at, language, { month: 'short', day: 'numeric' });

/** Day and time in the person's own time zone, for the record. */
export const moment = (at: string, language: string) => format(at, language, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });

/** Ticket numbers as a list in the person's language. */
export function names(ids: string[], language: string) {
  try {
    return new Intl.ListFormat(language || undefined, { type: 'conjunction' }).format(ids);
  } catch {
    return ids.join(', ');
  }
}

export const childrenOf = (ticket: Ticket, tickets: Full[]) => tickets.filter(other => other.parent === ticket.id);

/** `2/5`: a parent's finished sub-tickets out of all of them; null without any. */
export function progress(ticket: Ticket, tickets: Full[]) {
  const children = childrenOf(ticket, tickets);
  return children.length ? `${children.filter(child => child.state === 'done').length}/${children.length}` : null;
}

/** The tickets in the list's order: by state group, then by number, each sub-ticket after its parent. Previous and next walk it. */
export function ordered(tickets: Full[]) {
  const top = tickets.filter(ticket => !ticket.parent || !tickets.some(other => other.id === ticket.parent));
  return STATE_ORDER.flatMap(state => top.filter(ticket => ticket.state === state)).flatMap(ticket => [ticket, ...childrenOf(ticket, tickets)]);
}

/** Why it cannot be finished now, in the person's words: the store's reason (open sub-tickets, a check that has not run, a change after its check). */
export function whyNotDone(ticket: Full, words: Words) {
  const why = ticket.whyNotDone;
  if (!why) return null;
  if (why.kind === 'subs') return fill(words.whySubs, { n: String(why.ids.length) });
  return why.kind === 'check' ? words.whyCheck : words.whyRecheck;
}

/** Why the Assistant cannot take it: tickets it waits for (the store's list), no agent connected, or another ticket in its hands (one at a time). */
export function whyNotAssistant(ticket: Full, state: TicketsState, tickets: Full[]) {
  const waits = ticket.openWaits;
  if (waits.length) return fill(state.strings.whyWaits, { ids: names(waits, state.language) });
  if (state.assistant === false) return state.strings.whyOffline;
  const busy = tickets.find(other => other.state === 'in_progress' && other.worker === 'assistant');
  return busy ? fill(state.strings.whyBusy, { id: busy.id }) : null;
}
