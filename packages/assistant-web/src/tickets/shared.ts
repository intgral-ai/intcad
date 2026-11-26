// What the tickets view's parts share: the words for a state, dates in the person's own form, and
// the order a person walks the tickets in.
import type { Ticket, TicketStateName, TicketsState } from '../host';
import { STATE_ORDER } from './status-icons';

export type Words = TicketsState['strings'];

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

/** The tickets in the list's order: by state group, then by number. Previous and next walk it. */
export function ordered(tickets: Ticket[]) {
  return STATE_ORDER.flatMap(state => tickets.filter(ticket => ticket.state === state));
}

/** A person may change or remove a ticket nobody has started. */
export const editable = (ticket: Ticket) => ticket.state === 'draft' || ticket.state === 'confirmed';
