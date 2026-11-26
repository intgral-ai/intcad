// Derived from Circle, components/common/issues/group-issues.tsx and issue-line.tsx
// (github.com/ln-dev7/circle at b0ba7dd): the grouped issue list, its sticky group bars and its
// rows, without the board view, drag and drop, the context menu and the animation library.
// MIT, Copyright (c) 2025 lndev-ui | Circle Template.
// Here: a project's part tickets grouped by state, the drafts the Assistant proposed on top with
// Accept and Decline, sub-tickets folded under their parent, a part's filter and the deleted
// tickets; the controls are Mastra's.
import { Box, Ellipsis, Lock, Paperclip, Plus, Sparkles, Trash2, X } from 'lucide-react';
import { useState } from 'react';
import type { TicketsState } from '../host';
import { post } from '../host';
import { childrenOf, day, fill, names, progress, stateWord } from './shared';
import type { Full, Words } from './shared';
import { STATE_COLOUR, STATE_ORDER, StateIcon } from './status-icons';
import { Breadcrumb, Crumb } from '@/ds/components/Breadcrumb';
import { Button } from '@/ds/components/Button';
import { DisclosureChevron } from '@/ds/components/DisclosureChevron';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { EmptyState } from '@/ds/components/EmptyState';
import { Notice } from '@/ds/components/Notice';

/** `tickets`: all live tickets (a parent's count); `shown`: those the part's filter lets through. */
type List = { state: TicketsState; words: Words; tickets: Full[]; shown: Full[] };

/** What a row says on its right: the first part and how many more, attachments, a lock while it waits, the day. */
function Meta({ ticket, list }: { ticket: Full; list: List }) {
  const [first, ...more] = ticket.parts;
  return (
    <span className="ml-auto flex shrink-0 items-center justify-end gap-2 pl-2 text-xs whitespace-nowrap text-muted-foreground">
      {first && (
        <span className="inline-flex items-center gap-1">
          <span className="max-w-16 truncate sm:max-w-32">{first.id}</span>
          {more.length > 0 && <span>{`+${more.length}`}</span>}
        </span>
      )}
      {ticket.attachments.length > 0 && (
        <span className="inline-flex items-center gap-0.5">
          <Paperclip className="size-3" aria-hidden />
          {ticket.attachments.length}
        </span>
      )}
      {ticket.waits.length > 0 && (
        <span title={`${list.words.waitsFor} ${names(ticket.waits, list.state.language)}`} aria-label={list.words.waitsFor}>
          <Lock className="size-3" aria-hidden />
        </span>
      )}
      <span>{day(ticket.created, list.state.language)}</span>
    </span>
  );
}

function Row({ ticket, list, depth = 0, open, onToggle }: { ticket: Full; list: List; depth?: number; open?: boolean; onToggle?: () => void }) {
  const counted = progress(ticket, list.tickets);
  const proposed = ticket.state === 'draft' && depth === 0;
  return (
    // The fold sits in the row's left margin, so every state icon lines up with its group's.
    <div className="flex h-11 w-full min-w-0 items-center pr-4 hover:bg-fill-subtle sm:pr-6 sm:pl-2">
      <span className="flex w-4 shrink-0 justify-center" style={{ marginRight: depth * 20 }}>
        {onToggle && (
          <button type="button" aria-expanded={open} aria-label={list.words.subTickets} onClick={onToggle} className="flex size-4 items-center justify-center rounded-sm text-muted-foreground hover:text-foreground">
            <DisclosureChevron direction="right" open={open} className="size-3.5" />
          </button>
        )}
      </span>
      <button
        type="button"
        data-ticket={ticket.id}
        onClick={() => post({ type: 'ticket_open', id: ticket.id })}
        className="flex h-full min-w-0 flex-1 items-center gap-2 text-left focus-visible:outline-none"
      >
        <span className="flex size-5 shrink-0 items-center justify-center">
          <StateIcon state={ticket.state} />
        </span>
        <span className="shrink-0 text-sm font-medium whitespace-nowrap text-muted-foreground">{ticket.id}</span>
        <span className="min-w-0 truncate text-sm font-medium sm:font-semibold">{ticket.title}</span>
        {counted && <span className="shrink-0 rounded-full border border-border px-1.5 text-xs text-muted-foreground tabular-nums">{counted}</span>}
        {!proposed && <Meta ticket={ticket} list={list} />}
      </button>
      {proposed && (
        <span className="ml-1 flex shrink-0 items-center gap-1">
          <Button size="sm" variant="primary" onClick={() => post({ type: 'ticket_confirm', id: ticket.id, revision: ticket.revision })}>
            {list.words.confirm}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => post({ type: 'ticket_decline', id: ticket.id, revision: ticket.revision })}>
            {list.words.decline}
          </Button>
        </span>
      )}
    </div>
  );
}

/** A ticket and, when unfolded, its sub-tickets under it, whatever their state. */
function Family({ ticket, list }: { ticket: Full; list: List }) {
  const [open, setOpen] = useState(true);
  const children = childrenOf(ticket, list.shown);
  return (
    <>
      <Row ticket={ticket} list={list} open={open} onToggle={children.length ? () => setOpen(!open) : undefined} />
      {open && children.map(child => <Row key={child.id} ticket={child} list={list} depth={1} />)}
    </>
  );
}

function Group({ state, tickets, list }: { state: Full['state']; tickets: Full[]; list: List }) {
  const label = state === 'draft' ? list.words.proposed : stateWord(list.words, state);
  return (
    <section aria-label={label}>
      <div className="sticky top-0 z-10 h-10 w-full bg-background">
        <div className="flex h-full w-full items-center gap-2 px-4 sm:px-6" style={{ backgroundColor: `${STATE_COLOUR[state]}08` }}>
          <span className="flex size-5 shrink-0 items-center justify-center">
            <StateIcon state={state} />
          </span>
          <span className="truncate text-sm font-medium">{label}</span>
          <span className="text-sm text-muted-foreground">{tickets.length}</span>
        </div>
      </div>
      {tickets.map(ticket => (
        <Family key={ticket.id} ticket={ticket} list={list} />
      ))}
    </section>
  );
}

/** The deleted tickets, each with Restore; reached from the list's menu. */
function Deleted({ words, deleted, onBack }: { words: Words; deleted: Full[]; onBack: () => void }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-2 sm:px-4">
        <Breadcrumb label={words.title} className="min-w-0">
          <Crumb as="button" onClick={onBack}>
            {words.title}
          </Crumb>
          <Crumb as="span" isCurrent>
            {words.deleted}
          </Crumb>
        </Breadcrumb>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {deleted.length === 0 ? (
          <EmptyState iconSlot={null} titleSlot={words.noDeleted} className="py-16" />
        ) : (
          deleted.map(ticket => (
            <div key={ticket.id} className="flex h-11 min-w-0 items-center gap-2 px-4 text-muted-foreground sm:px-6">
              <span className="flex size-5 shrink-0 items-center justify-center opacity-60">
                <StateIcon state={ticket.state} />
              </span>
              <span className="shrink-0 text-sm font-medium">{ticket.id}</span>
              <span className="min-w-0 flex-1 truncate text-sm line-through decoration-muted-foreground/50">{ticket.title}</span>
              <Button size="sm" className="shrink-0" onClick={() => post({ type: 'ticket_restore', id: ticket.id })}>
                {words.restore}
              </Button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

export function TicketList({ state, tickets, onNew }: { state: TicketsState; tickets: Full[]; onNew: () => void }) {
  const [showDeleted, setShowDeleted] = useState(false);
  const words = state.strings;
  const live = tickets.filter(ticket => !ticket.deleted);
  if (showDeleted) return <Deleted words={words} deleted={tickets.filter(ticket => ticket.deleted)} onBack={() => setShowDeleted(false)} />;
  const filter = state.filter;
  const shown = filter ? live.filter(ticket => ticket.parts.some(part => part.kind === filter.kind && part.id === filter.id)) : live;
  const list: List = { state, words, tickets: live, shown };
  // A sub-ticket follows its parent; one whose parent is not shown stands on its own.
  const top = shown.filter(ticket => !ticket.parent || !shown.some(other => other.id === ticket.parent));
  const groups = STATE_ORDER.map(name => ({ name, tickets: top.filter(ticket => ticket.state === name) })).filter(group => group.tickets.length > 0);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-border px-4 sm:px-6">
        <div className="flex min-w-0 items-center gap-2">
          <h1 className="truncate text-sm font-medium">{words.title}</h1>
          {live.length > 0 && <span className="text-sm text-muted-foreground">{live.length}</span>}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <DropdownMenu>
            <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={words.more} tooltip={words.more}>
              <Ellipsis />
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="end">
              <DropdownMenu.Item onClick={() => setShowDeleted(true)}>
                <Trash2 />
                {words.deleted}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu>
          <Button size="sm" icon={<Plus />} onClick={onNew} disabled={!state.project}>
            {words.newTicket}
          </Button>
        </div>
      </header>
      {filter && (
        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border pr-2 pl-4 text-sm sm:pl-6">
          <Box className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="min-w-0 flex-1 truncate">{fill(words.filterPart, { part: filter.id })}</span>
          <Button variant="ghost" size="icon-sm" aria-label={words.clear} tooltip={words.clear} onClick={() => post({ type: 'ticket_filter_clear' })}>
            <X />
          </Button>
        </div>
      )}
      {state.error && (
        <Notice variant="destructive" className="mx-4 mt-3 sm:mx-6">
          <Notice.Message>{state.error}</Notice.Message>
        </Notice>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {groups.length === 0 ? (
          <EmptyState
            iconSlot={null}
            titleSlot={state.project ? words.empty : words.noProject}
            className="px-6 py-16"
            actionSlot={
              state.project && (
                <div className="flex flex-wrap justify-center gap-2">
                  <Button size="sm" icon={<Sparkles />} onClick={() => post({ type: 'ticket_ask_draft' })}>
                    {words.askDraft}
                  </Button>
                  <Button size="sm" icon={<Plus />} onClick={onNew}>
                    {words.newTicket}
                  </Button>
                </div>
              )
            }
          />
        ) : (
          groups.map(group => <Group key={group.name} state={group.name} tickets={group.tickets} list={list} />)
        )}
      </div>
    </div>
  );
}
