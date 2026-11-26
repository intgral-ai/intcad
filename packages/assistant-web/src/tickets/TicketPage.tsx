// Derived from Circle, components/layout/headers/issue/header-nav.tsx and
// components/common/issues/details/issue-details.tsx, issue-properties-panel.tsx and
// activity-feed.tsx (github.com/ln-dev7/circle at b0ba7dd): the issue page's header with previous
// and next, its title, properties, a list in the sub-issues' manner and the activity feed, without
// the comment box, reactions and the rich description. MIT, Copyright (c) 2025 lndev-ui | Circle
// Template.
// Here: one part ticket, laid out for a narrow panel (properties under the title, not in a side
// column). What it says is shown as plain text. The controls are Mastra's.
import { Ban, Box, ChevronDown, ChevronUp, CircleDot, ClipboardCheck, Ellipsis, PenLine, Pencil, Trash2, Unlock } from 'lucide-react';
import type { ReactNode } from 'react';
import type { Ticket, TicketEvent, TicketsState } from '../host';
import { post } from '../host';
import { editable, moment, ordered, stateWord } from './shared';
import type { Words } from './shared';
import { StateIcon } from './status-icons';
import { Breadcrumb, Crumb } from '@/ds/components/Breadcrumb';
import { Button } from '@/ds/components/Button';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { Notice } from '@/ds/components/Notice';

const EVENT_ICONS: Record<string, ReactNode> = {
  drafted: <PenLine className="size-3.5" />,
  created: <PenLine className="size-3.5" />,
  edited: <Pencil className="size-3.5" />,
  check_asked: <ClipboardCheck className="size-3.5" />,
  check_started: <ClipboardCheck className="size-3.5" />,
  check_ran: <ClipboardCheck className="size-3.5" />,
  check_refused: <ClipboardCheck className="size-3.5" />,
  blocked: <Ban className="size-3.5" />,
  unblocked: <Unlock className="size-3.5" />,
};

const DID = {
  drafted: 'didDrafted',
  created: 'didCreated',
  edited: 'didEdited',
  confirmed: 'didConfirmed',
  started: 'didStarted',
  check_asked: 'didCheckAsked',
  check_started: 'didCheckStarted',
  check_ran: 'didCheckRan',
  check_refused: 'didCheckRefused',
  blocked: 'didBlocked',
  unblocked: 'didUnblocked',
  done: 'didDone',
} as const;

function actorWord(words: Words, actor: TicketEvent['actor']) {
  return actor === 'person' ? words.actorPerson : actor === 'agent' ? words.actorAgent : words.actorApp;
}

function EventRow({ event, words, language }: { event: TicketEvent; words: Words; language: string }) {
  const did = event.action in DID ? words[DID[event.action as keyof typeof DID]] : event.action;
  // An agent's note, a refusal's reason; a bound run is named by its id (the store's counts line is English).
  const extra = event.action === 'check_ran' || event.action === 'check_started' ? event.run : event.note;
  return (
    <li className="flex items-start gap-2.5 py-1.5 text-sm text-muted-foreground">
      <span className="mt-px flex size-5 shrink-0 items-center justify-center rounded-full bg-fill">{EVENT_ICONS[event.action] ?? <CircleDot className="size-3.5" />}</span>
      <span className="min-w-0 flex-1 break-words">
        <span className="font-medium text-foreground/90">{actorWord(words, event.actor)}</span> {did}
        <span className="text-xs whitespace-nowrap">{` · ${moment(event.at, language)}`}</span>
        {extra && <span className="mt-0.5 block text-xs whitespace-pre-wrap">{extra}</span>}
      </span>
    </li>
  );
}

/** The bound check in the run's own words: its id, its outcome word and its counts, as the run gave them. */
function checkText(ticket: Ticket, state: TicketsState) {
  const words = state.strings;
  const check = ticket.check;
  if (!check) return words.noCheck;
  if (check.state === 'asked') return words.checkAsked;
  if (check.state === 'running') return [words.checkRunning, check.run].filter(Boolean).join(' · ');
  if (check.state === 'refused') return [words.checkRefused, check.reason].filter(Boolean).join(' · ');
  const counts = check.counts ?? {};
  const named: [string, string][] = [
    ['failed', words.failed],
    ['advisory', words.advisory],
    ['checked', words.checked],
    ['unsupported', words.notCovered],
  ];
  const parts = named.filter(([key]) => counts[key] !== undefined).map(([key, word]) => `${counts[key]} ${word}`);
  parts.push(...Object.entries(counts).filter(([key]) => !named.some(([known]) => known === key)).map(([key, n]) => `${n} ${key}`));
  const outcome = check.outcome === 'done' ? words.checkDone : check.outcome;
  const now = check.run ? state.freshness[check.run] : undefined;
  return [`${words.checkTitle} ${check.run ?? ''}`.trim(), outcome, ...parts, now && now !== 'current' ? words.checkStale : null].filter(Boolean).join(' · ');
}

/** The one step a person can take on a ticket in its state. A blocked ticket waits for the agent. */
function Step({ ticket, words }: { ticket: Ticket; words: Words }) {
  const step = ticket.state === 'draft' ? 'confirm' : ticket.state === 'confirmed' ? 'start' : ticket.state === 'waiting' ? 'done' : null;
  if (!step) return null;
  return (
    <Button size="sm" variant="primary" className="ml-auto shrink-0" onClick={() => post({ type: `ticket_${step}`, id: ticket.id, revision: ticket.revision })}>
      {words[step]}
    </Button>
  );
}

function Property({ icon, label, children }: { icon: ReactNode; label: string; children: ReactNode }) {
  return (
    <div className="flex min-h-8 items-center gap-2 text-sm">
      <span className="flex size-5 shrink-0 items-center justify-center text-muted-foreground" title={label} aria-label={label} role="img">
        {icon}
      </span>
      <span className="min-w-0 flex-1 break-words">{children}</span>
    </div>
  );
}

export function TicketPage({ state, ticket, onEdit, onDelete }: { state: TicketsState; ticket: Ticket; onEdit: () => void; onDelete: () => void }) {
  const words = state.strings;
  const all = ordered(state.tickets);
  const index = all.findIndex(other => other.id === ticket.id);
  const previous = index > 0 ? all[index - 1] : undefined;
  const next = index >= 0 && index < all.length - 1 ? all[index + 1] : undefined;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-border px-2 sm:px-4">
        <Breadcrumb label={words.title} className="min-w-0">
          <Crumb as="button" onClick={() => post({ type: 'ticket_close' })}>
            {words.title}
          </Crumb>
          <Crumb as="span" isCurrent>
            {ticket.id}
          </Crumb>
        </Breadcrumb>
        <div className="flex shrink-0 items-center gap-1">
          {index >= 0 && <span className="mr-1 text-xs whitespace-nowrap text-muted-foreground tabular-nums">{`${index + 1} / ${all.length}`}</span>}
          <Button variant="ghost" size="icon-sm" aria-label={words.previous} tooltip={words.previous} disabled={!previous} onClick={() => previous && post({ type: 'ticket_open', id: previous.id })}>
            <ChevronUp />
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label={words.next} tooltip={words.next} disabled={!next} onClick={() => next && post({ type: 'ticket_open', id: next.id })}>
            <ChevronDown />
          </Button>
          {editable(ticket) && (
            <DropdownMenu>
              <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={words.more} tooltip={words.more}>
                <Ellipsis />
              </DropdownMenu.Trigger>
              <DropdownMenu.Content align="end">
                <DropdownMenu.Item onClick={onEdit}>
                  <Pencil />
                  {words.edit}
                </DropdownMenu.Item>
                <DropdownMenu.Item onClick={onDelete}>
                  <Trash2 />
                  {words.delete}
                </DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu>
          )}
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <article className="mx-auto max-w-3xl px-4 py-6 sm:px-8 sm:py-10">
          {state.error && (
            <Notice variant="destructive" className="mb-4">
              <Notice.Message>{state.error}</Notice.Message>
            </Notice>
          )}
          <h1 className="text-xl leading-tight font-semibold break-words whitespace-pre-wrap text-balance sm:text-2xl">{ticket.title}</h1>

          <section className="mt-5 flex flex-col" aria-label={words.properties}>
            <h2 className="mb-1 text-xs font-medium text-muted-foreground">{words.properties}</h2>
            <div className="flex min-h-8 items-center gap-2 text-sm">
              <span className="flex size-5 shrink-0 items-center justify-center">
                <StateIcon state={ticket.state} />
              </span>
              <span className="min-w-0">{stateWord(words, ticket.state)}</span>
              <Step ticket={ticket} words={words} />
            </div>
            {ticket.state === 'blocked' && ticket.blocked && (
              <Property icon={<Ban className="size-3.5" />} label={words.stateBlocked}>
                <span className="whitespace-pre-wrap text-muted-foreground">{ticket.blocked}</span>
              </Property>
            )}
            <Property icon={<Box className="size-3.5" />} label={words.part}>
              {ticket.part ? ticket.part.id : <span className="text-muted-foreground">{words.noPart}</span>}
            </Property>
            <Property icon={<ClipboardCheck className="size-3.5" />} label={words.check}>
              <span className={ticket.check?.state === 'ran' ? undefined : 'text-muted-foreground'}>{checkText(ticket, state)}</span>
            </Property>
          </section>

          {ticket.text && <p className="mt-6 text-sm leading-relaxed break-words whitespace-pre-wrap">{ticket.text}</p>}

          {ticket.requirements.length > 0 && (
            <section className="mt-8" aria-label={words.requirements}>
              <h2 className="mb-1 text-sm font-medium">
                {words.requirements} <span className="text-muted-foreground">{ticket.requirements.length}</span>
              </h2>
              <ol className="flex flex-col border-t border-border">
                {ticket.requirements.map((line, at) => (
                  <li key={at} className="flex min-h-10 items-start gap-2.5 border-b border-border px-1 py-2.5 text-sm">
                    <span className="w-4 shrink-0 text-right text-xs leading-5 text-muted-foreground tabular-nums">{at + 1}</span>
                    <span className="min-w-0 flex-1 break-words whitespace-pre-wrap">{line}</span>
                  </li>
                ))}
              </ol>
            </section>
          )}

          <section className="mt-10" aria-label={words.activity}>
            <h2 className="mb-2 text-base font-semibold">{words.activity}</h2>
            <ol className="flex flex-col">
              {state.record.map((event, at) => (
                <EventRow key={at} event={event} words={words} language={state.language} />
              ))}
            </ol>
          </section>
        </article>
      </div>
    </div>
  );
}
