// Derived from Circle, components/common/issues/group-issues.tsx and issue-line.tsx
// (github.com/ln-dev7/circle at b0ba7dd): the grouped issue list, its sticky group bars and its
// rows, without the board view, drag and drop, the context menu and the animation library.
// MIT, Copyright (c) 2025 lndev-ui | Circle Template.
// Here: a project's part tickets, grouped by state; the controls are Mastra's.
import { Box, Plus } from 'lucide-react';
import type { Ticket, TicketsState } from '../host';
import { post } from '../host';
import { day, stateWord } from './shared';
import type { Words } from './shared';
import { STATE_COLOUR, STATE_ORDER, StateIcon } from './status-icons';
import { Button } from '@/ds/components/Button';
import { EmptyState } from '@/ds/components/EmptyState';
import { Notice } from '@/ds/components/Notice';

function Row({ ticket, language }: { ticket: Ticket; language: string }) {
  return (
    <button
      type="button"
      data-ticket={ticket.id}
      onClick={() => post({ type: 'ticket_open', id: ticket.id })}
      className="flex h-11 w-full min-w-0 items-center justify-start gap-2 px-4 text-left hover:bg-fill-subtle focus-visible:bg-fill-subtle focus-visible:outline-none sm:px-6"
    >
      <span className="flex size-7 shrink-0 items-center justify-center">
        <StateIcon state={ticket.state} />
      </span>
      <span className="w-9 shrink-0 text-sm font-medium whitespace-nowrap text-muted-foreground">{ticket.id}</span>
      <span className="min-w-0 flex-1 truncate text-sm font-medium sm:font-semibold">{ticket.title}</span>
      <span className="ml-auto flex shrink-0 items-center justify-end gap-2 pl-3">
        {ticket.part && (
          <span className="inline-flex max-w-20 items-center gap-1 text-xs whitespace-nowrap text-muted-foreground sm:max-w-40 sm:rounded-full sm:border sm:border-border sm:px-2 sm:py-0.5">
            <Box className="hidden size-3 shrink-0 sm:block" aria-hidden />
            <span className="truncate">{ticket.part.id}</span>
          </span>
        )}
        <span className="text-xs whitespace-nowrap text-muted-foreground">{day(ticket.created, language)}</span>
      </span>
    </button>
  );
}

function Group({ state, tickets, words, language }: { state: Ticket['state']; tickets: Ticket[]; words: Words; language: string }) {
  return (
    <section aria-label={stateWord(words, state)}>
      <div className="sticky top-0 z-10 h-10 w-full bg-background">
        <div className="flex h-full w-full items-center gap-2 px-4 sm:px-6" style={{ backgroundColor: `${STATE_COLOUR[state]}08` }}>
          <span className="flex size-7 shrink-0 items-center justify-center">
            <StateIcon state={state} />
          </span>
          <span className="truncate text-sm font-medium">{stateWord(words, state)}</span>
          <span className="text-sm text-muted-foreground">{tickets.length}</span>
        </div>
      </div>
      <div>
        {tickets.map(ticket => (
          <Row key={ticket.id} ticket={ticket} language={language} />
        ))}
      </div>
    </section>
  );
}

export function TicketList({ state, onNew }: { state: TicketsState; onNew: () => void }) {
  const words = state.strings;
  const groups = STATE_ORDER.map(name => ({ name, tickets: state.tickets.filter(ticket => ticket.state === name) })).filter(group => group.tickets.length > 0);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-border px-4 sm:px-6">
        <div className="flex min-w-0 items-center gap-2">
          <h1 className="truncate text-sm font-medium">{words.title}</h1>
          {state.tickets.length > 0 && <span className="text-sm text-muted-foreground">{state.tickets.length}</span>}
        </div>
        <Button size="sm" icon={<Plus />} onClick={onNew} disabled={!state.project}>
          {words.newTicket}
        </Button>
      </header>
      {state.error && (
        <Notice variant="destructive" className="mx-4 mt-3 sm:mx-6">
          <Notice.Message>{state.error}</Notice.Message>
        </Notice>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {groups.length === 0 ? (
          <EmptyState iconSlot={null} titleSlot={state.project ? words.empty : words.noProject} className="py-16" />
        ) : (
          groups.map(group => <Group key={group.name} state={group.name} tickets={group.tickets} words={words} language={state.language} />)
        )}
      </div>
    </div>
  );
}
