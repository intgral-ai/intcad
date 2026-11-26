// Derived from Circle, components/layout/headers/issue/header-nav.tsx,
// components/common/issues/details/issue-details.tsx, issue-properties-panel.tsx and
// activity-feed.tsx, and components/common/issues/status-selector.tsx (github.com/ln-dev7/circle at
// b0ba7dd): the issue page's header with previous and next, its title and description, the
// sub-issues, the activity feed, the properties column and the status menu, without the comment
// box, reactions and the rich description. MIT, Copyright (c) 2025 lndev-ui | Circle Template.
// Here: one part ticket, narrow in the side panel (properties under the title) or large over the
// window (properties in a column at the right). A click on its title, description or requirement
// lines edits them where they stand. What it says is shown as plain text. The controls are Mastra's.
import { Ban, Box, ChevronDown, ChevronUp, CircleDot, ClipboardCheck, Ellipsis, FileText, Maximize2, Minimize2, Paperclip, PenLine, Pencil, Plus, Trash2, Unlock } from 'lucide-react';
import type { ReactNode } from 'react';
import { useRef, useState } from 'react';
import type { TicketEvent, TicketStateName, TicketsState } from '../host';
import { pictureUrl, post } from '../host';
import { childrenOf, fill, lines, MAX_TEXT, MAX_TITLE, moment, names, openWaits, ordered, progress, stateWord, whyNotAssistant, whyNotDone } from './shared';
import type { Full, Words } from './shared';
import { StateIcon } from './status-icons';
import { AttachmentCard } from '@/ds/components/AttachmentCard';
import { Breadcrumb, Crumb } from '@/ds/components/Breadcrumb';
import { Button } from '@/ds/components/Button';
import { Dialog, DialogContent, DialogTitle } from '@/ds/components/Dialog';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { Input } from '@/ds/components/Input';
import { Notice } from '@/ds/components/Notice';
import { Textarea } from '@/ds/components/Textarea';
import { toast } from '@/ds/components/Toaster';

/** A small chip naming a part, as the form and the ticket's properties show it. */
export function PartChip({ id, onClick }: { id: string; onClick?: () => void }) {
  const body = (
    <>
      <Box className="size-3 shrink-0 text-muted-foreground" aria-hidden />
      <span className="truncate">{id}</span>
    </>
  );
  const chip = 'inline-flex h-6 max-w-full min-w-0 items-center gap-1 rounded-full border border-border px-2 text-xs';
  return onClick ? (
    <button type="button" className={`${chip} hover:bg-fill-hover`} onClick={onClick}>
      {body}
    </button>
  ) : (
    <span className={chip}>{body}</span>
  );
}

type Page = { state: TicketsState; words: Words; ticket: Full; tickets: Full[] };

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

/** The states a person may set from the menu, in Linear's order; a draft is accepted, not set. */
const SETTABLE: TicketStateName[] = ['confirmed', 'in_progress', 'blocked', 'waiting', 'done'];

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

/** The check in the run's own words: its id, its outcome word and its counts; 未检查 where none applies or it checked nothing. */
function checkText({ ticket, state, words }: Page) {
  if (!ticket.checks) return `${words.notChecked} · ${words.noApplicableCheck}`;
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
  const nothing = !counts.checked ? words.notChecked : null;
  return [nothing, `${words.checkTitle} ${check.run ?? ''}`.trim(), outcome, ...parts, now && now !== 'current' ? words.checkStale : null].filter(Boolean).join(' · ');
}

/** Text that becomes its editor on a click and saves itself when the person leaves it; Esc leaves it as it was. */
function Editable({ value, multiline, label, empty, onSave, children }: { value: string; multiline?: boolean; label: string; empty?: string; onSave: (next: string) => void; children: ReactNode }) {
  const [draft, setDraft] = useState<string | null>(null);
  if (draft === null) {
    return (
      <div
        role="button"
        tabIndex={0}
        aria-label={label}
        data-edit={label}
        onClick={() => setDraft(value)}
        onKeyDown={event => event.key === 'Enter' && setDraft(value)}
        className="-mx-2 cursor-text rounded-md px-2 py-0.5 hover:bg-fill-subtle focus-visible:bg-fill-subtle focus-visible:outline-none"
      >
        {value ? children : <span className="text-sm text-muted-foreground">{empty}</span>}
      </div>
    );
  }
  const finish = () => {
    if (draft !== value) onSave(draft);
    setDraft(null);
  };
  const keys = (event: React.KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.stopPropagation();
      setDraft(null);
    } else if (!multiline && event.key === 'Enter') finish();
  };
  return multiline ? (
    <Textarea autoFocus aria-label={label} rows={Math.max(3, draft.split('\n').length + 1)} maxLength={MAX_TEXT} value={draft} onChange={event => setDraft(event.target.value)} onBlur={finish} onKeyDown={keys} />
  ) : (
    <Input autoFocus aria-label={label} maxLength={MAX_TITLE} value={draft} onChange={event => setDraft(event.target.value)} onBlur={finish} onKeyDown={keys} className="h-auto py-1 !text-xl !font-semibold" />
  );
}

/** Saves one field and sends the other two as they are; the store records an edit. */
function edit({ ticket }: Page, change: Partial<{ title: string; text: string; requirements: string[] }>) {
  const next = { title: ticket.title, text: ticket.text, requirements: ticket.requirements, ...change };
  if (!next.title.trim()) return;
  post({ type: 'ticket_edit', id: ticket.id, revision: ticket.revision, ...next });
}

/** "It still waits for T-3 and T-5": what a person is reminded of, not stopped by, when they move a waiting ticket (D-88). */
function still({ ticket, tickets, words, state }: Page) {
  const waits = openWaits(ticket, tickets);
  return waits.length ? fill(words.stillWaits, { ids: names(waits, state.language) }) : null;
}

/** The state, and the menu a person changes it with; a step that cannot be taken now says why. */
function StateMenu(page: Page) {
  const { ticket, tickets, words, state } = page;
  const why: Partial<Record<TicketStateName, string | null>> = { done: ticket.state === 'done' ? null : whyNotDone(ticket, tickets, words) };
  const reminder = still(page);
  return (
    <DropdownMenu>
      <DropdownMenu.Trigger variant="ghost" size="sm" aria-label={words.state} className="-ml-2">
        <StateIcon state={ticket.state} />
        {stateWord(words, ticket.state)}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="start" className="min-w-64">
        {reminder && <DropdownMenu.Label className="max-w-72 whitespace-normal">{reminder}</DropdownMenu.Label>}
        <DropdownMenu.RadioGroup value={ticket.state} onValueChange={(to: unknown) => post({ type: 'ticket_set_state', id: ticket.id, revision: ticket.revision, to: to as TicketStateName })}>
          {SETTABLE.map(name => (
            <DropdownMenu.RadioItem key={name} value={name} disabled={Boolean(why[name])} closeOnClick>
              <StateIcon state={name} />
              <span className="shrink-0">{stateWord(words, name)}</span>
              {why[name] && <span className="ml-auto min-w-0 truncate pl-4 text-xs text-muted-foreground">{why[name]}</span>}
            </DropdownMenu.RadioItem>
          ))}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Content>
    </DropdownMenu>
  );
}

function Prop({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-h-8 items-start gap-3 text-sm">
      <span className="w-14 shrink-0 text-xs leading-8 text-muted-foreground">{label}</span>
      <div className="flex min-h-8 min-w-0 flex-1 flex-wrap items-center gap-1.5 py-1">{children}</div>
    </div>
  );
}

function Properties(page: Page) {
  const { ticket, tickets, words, state } = page;
  const selected = Boolean(state.selection?.length);
  const assign = () => post({ type: 'ticket_assign_parts', id: ticket.id, revision: ticket.revision });
  const others = tickets.filter(other => other.id !== ticket.id && other.parent !== ticket.id);
  const toggle = (id: string) => post({ type: 'ticket_waits', id: ticket.id, revision: ticket.revision, waits: ticket.waits.includes(id) ? ticket.waits.filter(other => other !== id) : [...ticket.waits, id] });
  return (
    <section className="flex flex-col" aria-label={words.properties}>
      <Prop label={words.state}>
        <StateMenu {...page} />
      </Prop>
      {ticket.state === 'blocked' && ticket.blocked && (
        <Prop label={words.stateBlocked}>
          <span className="whitespace-pre-wrap text-muted-foreground">{ticket.blocked}</span>
        </Prop>
      )}
      <Prop label={words.part}>
        {ticket.parts.map(part => (
          <PartChip key={`${part.kind}:${part.id}`} id={part.id} onClick={() => post({ type: 'ticket_show_parts', id: ticket.id })} />
        ))}
        {ticket.parts.length === 0 ? (
          <>
            <span className="text-muted-foreground">{words.noPart}</span>
            <Button size="sm" variant="ghost" disabled={!selected} tooltip={selected ? undefined : words.selectFirst} onClick={assign}>
              {words.assign}
            </Button>
          </>
        ) : (
          <Button variant="ghost" size="icon-sm" aria-label={words.assign} tooltip={selected ? words.assign : words.selectFirst} disabled={!selected} onClick={assign}>
            <Plus />
          </Button>
        )}
      </Prop>
      <Prop label={words.waitsFor}>
        {ticket.waits.map(id => {
          const other = tickets.find(candidate => candidate.id === id);
          return (
            <button key={id} type="button" onClick={() => post({ type: 'ticket_open', id })} className="inline-flex h-6 items-center gap-1 rounded-full border border-border px-2 text-xs hover:bg-fill-hover">
              {other && <StateIcon state={other.state} />}
              {id}
            </button>
          );
        })}
        <DropdownMenu>
          <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={words.setWaits} tooltip={words.setWaits}>
            <Plus />
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="start" className="max-w-72">
            {others.map(other => (
              <DropdownMenu.CheckboxItem key={other.id} checked={ticket.waits.includes(other.id)} onCheckedChange={() => toggle(other.id)} closeOnClick={false}>
                <StateIcon state={other.state} />
                <span className="shrink-0 text-muted-foreground">{other.id}</span>
                <span className="min-w-0 truncate">{other.title}</span>
              </DropdownMenu.CheckboxItem>
            ))}
          </DropdownMenu.Content>
        </DropdownMenu>
      </Prop>
      <Prop label={words.check}>
        <span className={ticket.checks && ticket.check?.state === 'ran' ? undefined : 'text-muted-foreground'}>{checkText(page)}</span>
      </Prop>
      <Prop label={words.worker}>
        {ticket.worker ? <span>{ticket.worker === 'assistant' ? words.actorAgent : words.actorPerson}</span> : <span className="text-muted-foreground">{words.workerNone}</span>}
      </Prop>
    </section>
  );
}

/** The buttons a ticket's state offers a person; one that cannot be pressed now says why under it. */
function Actions(page: Page) {
  const { ticket, tickets, words, state } = page;
  const step = (type: 'ticket_confirm' | 'ticket_decline' | 'ticket_start' | 'ticket_work_myself' | 'ticket_ask_check' | 'ticket_done') => () => post({ type, id: ticket.id, revision: ticket.revision });
  let buttons: ReactNode = null;
  let why: string | null = null;
  if (ticket.state === 'draft') {
    buttons = (
      <>
        <Button size="sm" variant="primary" onClick={step('ticket_confirm')}>
          {words.confirm}
        </Button>
        <Button size="sm" variant="ghost" onClick={step('ticket_decline')}>
          {words.decline}
        </Button>
      </>
    );
  } else if (ticket.state === 'confirmed') {
    why = whyNotAssistant(ticket, state, tickets);
    const reminder = still(page);
    buttons = (
      <>
        <Button size="sm" variant="primary" disabled={Boolean(why)} onClick={step('ticket_start')}>
          {words.start}
        </Button>
        <Button
          size="sm"
          onClick={() => {
            step('ticket_work_myself')();
            if (reminder) toast(reminder);
          }}
        >
          {words.workMyself}
        </Button>
      </>
    );
  } else if ((ticket.state === 'in_progress' && ticket.worker === 'person') || ticket.state === 'waiting') {
    if (ticket.state === 'in_progress' && ticket.checks) {
      buttons = (
        <Button size="sm" variant="primary" onClick={step('ticket_ask_check')}>
          {words.askCheck}
        </Button>
      );
    } else {
      why = whyNotDone(ticket, tickets, words);
      buttons = (
        <Button size="sm" variant="primary" disabled={Boolean(why)} onClick={step('ticket_done')}>
          {words.done}
        </Button>
      );
    }
  }
  if (!buttons) return null;
  return (
    <div className="mt-3">
      <div className="flex flex-wrap items-center gap-2">{buttons}</div>
      {why && <p className="mt-1.5 text-xs text-muted-foreground">{why}</p>}
    </div>
  );
}

function Attachments({ ticket, words }: Page) {
  const open = (path: string) => () => post({ type: 'ticket_attachment', path });
  const type = (name: string) => name.split('.').pop()?.toUpperCase() ?? '';
  return (
    <section className="mt-8" aria-label={words.attachments}>
      <h2 className="mb-2 text-sm font-medium">
        {words.attachments} {ticket.attachments.length > 0 && <span className="text-muted-foreground">{ticket.attachments.length}</span>}
      </h2>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {ticket.attachments.map(file =>
          file.picture ? (
            <button key={file.path} type="button" title={file.name} aria-label={file.name} onClick={open(file.path)} className="cursor-pointer overflow-hidden rounded-xl border border-border bg-fill hover:border-border-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-border-focus">
              <img src={pictureUrl(file.picture)} alt={file.name} className="block h-20 w-auto max-w-36 object-cover" draggable={false} />
            </button>
          ) : (
            <div key={file.path} className="w-48 min-w-0">
              <AttachmentCard name={file.name} typeLabel={type(file.name)} icon={<FileText aria-hidden />} onClick={open(file.path)} />
            </div>
          ),
        )}
        <Button size="sm" variant="ghost" icon={<Paperclip />} onClick={() => post({ type: 'ticket_attach', id: ticket.id })}>
          {words.attach}
        </Button>
      </div>
    </section>
  );
}

/** A top ticket's sub-tickets, one row each, and Add sub-ticket; sub-tickets have none of their own. */
function SubTickets({ ticket, tickets, words, onAdd }: Page & { onAdd: () => void }) {
  if (ticket.parent) return null;
  const children = childrenOf(ticket, tickets);
  return (
    <section className="mt-8" aria-label={words.subTickets}>
      <h2 className="mb-1 text-sm font-medium">
        {words.subTickets} {children.length > 0 && <span className="text-muted-foreground">{progress(ticket, tickets)}</span>}
      </h2>
      {children.length > 0 && (
        <div className="flex flex-col border-t border-border">
          {children.map(child => (
            <button key={child.id} type="button" onClick={() => post({ type: 'ticket_open', id: child.id })} className="flex h-10 min-w-0 items-center gap-2.5 border-b border-border px-1 text-left text-sm hover:bg-fill-subtle">
              <StateIcon state={child.state} />
              <span className="shrink-0 text-xs font-medium text-muted-foreground">{child.id}</span>
              <span className="min-w-0 truncate font-medium">{child.title}</span>
              {child.worker && <span className="ml-auto shrink-0 text-xs text-muted-foreground">{child.worker === 'assistant' ? words.actorAgent : words.actorPerson}</span>}
            </button>
          ))}
        </div>
      )}
      <Button size="sm" variant="ghost" icon={<Plus />} className="mt-1 -ml-2" onClick={onAdd}>
        {words.addSub}
      </Button>
    </section>
  );
}

/** Title, the state's buttons, then (narrow) the properties, then description, requirement lines, attachments, sub-tickets and the record. */
function Body({ page, properties, onAddSub }: { page: Page; properties: boolean; onAddSub: () => void }) {
  const { ticket, words, state } = page;
  return (
    <>
      {state.error && (
        <Notice variant="destructive" className="mb-4">
          <Notice.Message>{state.error}</Notice.Message>
        </Notice>
      )}
      <Editable value={ticket.title} label={words.fieldTitle} onSave={title => edit(page, { title })}>
        <h1 className="text-xl leading-tight font-semibold break-words whitespace-pre-wrap text-balance sm:text-2xl">{ticket.title}</h1>
      </Editable>
      <Actions {...page} />
      {properties && (
        <div className="mt-5">
          <Properties {...page} />
        </div>
      )}
      <div className="mt-6">
        <Editable value={ticket.text} multiline label={words.fieldText} empty={words.addText} onSave={text => edit(page, { text })}>
          <p className="text-sm leading-relaxed break-words whitespace-pre-wrap">{ticket.text}</p>
        </Editable>
      </div>
      <section className="mt-8" aria-label={words.requirements}>
        <h2 className="mb-1 text-sm font-medium">
          {words.requirements} {ticket.requirements.length > 0 && <span className="text-muted-foreground">{ticket.requirements.length}</span>}
        </h2>
        <Editable
          value={ticket.requirements.join('\n')}
          multiline
          label={words.requirements}
          empty={words.fieldLines}
          onSave={typed => {
            const requirements = lines(typed);
            if (requirements) edit(page, { requirements });
          }}
        >
          <ol className="flex flex-col border-t border-border">
            {ticket.requirements.map((line, at) => (
              <li key={at} className="flex min-h-10 items-start gap-2.5 border-b border-border px-1 py-2.5 text-sm">
                <span className="w-4 shrink-0 text-right text-xs leading-5 text-muted-foreground tabular-nums">{at + 1}</span>
                <span className="min-w-0 flex-1 break-words whitespace-pre-wrap">{line}</span>
              </li>
            ))}
          </ol>
        </Editable>
      </section>
      <Attachments {...page} />
      <SubTickets {...page} onAdd={onAddSub} />
      <section className="mt-10" aria-label={words.activity}>
        <h2 className="mb-2 text-base font-semibold">{words.activity}</h2>
        <ol className="flex flex-col">
          {state.record.map((event, at) => (
            <EventRow key={at} event={event} words={words} language={state.language} />
          ))}
        </ol>
      </section>
    </>
  );
}

/** Path, place in the list, previous and next, open large or back to the side panel, and the ticket's menu. */
function Header({ page, expanded }: { page: Page; expanded: boolean }) {
  const { ticket, tickets, words } = page;
  const all = ordered(tickets);
  const index = all.findIndex(other => other.id === ticket.id);
  const previous = index > 0 ? all[index - 1] : undefined;
  const next = index >= 0 && index < all.length - 1 ? all[index + 1] : undefined;
  const remove = () => {
    post({ type: 'ticket_delete', id: ticket.id, revision: ticket.revision });
    toast(fill(words.deletedToast, { id: ticket.id }), { action: { label: words.undo, onClick: () => post({ type: 'ticket_restore', id: ticket.id }) } });
  };
  return (
    <header className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-border px-2 sm:px-4">
      <Breadcrumb label={words.title} className="min-w-0">
        <Crumb as="button" onClick={() => post({ type: 'ticket_close' })}>
          {words.title}
        </Crumb>
        {ticket.parent && (
          <Crumb as="button" onClick={() => ticket.parent && post({ type: 'ticket_open', id: ticket.parent })}>
            {ticket.parent}
          </Crumb>
        )}
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
        {expanded ? (
          <Button variant="ghost" size="icon-sm" aria-label={words.collapse} tooltip={words.collapse} onClick={() => post({ type: 'ticket_collapse' })}>
            <Minimize2 />
          </Button>
        ) : (
          <Button variant="ghost" size="icon-sm" aria-label={words.expand} tooltip={words.expand} onClick={() => post({ type: 'ticket_expand', id: ticket.id })}>
            <Maximize2 />
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={words.more} tooltip={words.more}>
            <Ellipsis />
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end">
            <DropdownMenu.Item variant="destructive" onClick={remove}>
              <Trash2 />
              {words.delete}
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu>
      </div>
    </header>
  );
}

export function TicketPage({ state, ticket, tickets, expanded, onAddSub }: { state: TicketsState; ticket: Full; tickets: Full[]; expanded: boolean; onAddSub: () => void }) {
  const page: Page = { state, words: state.strings, ticket, tickets: tickets.filter(other => !other.deleted) };
  // Opened large, the focus goes to the content, not to the first button of the path.
  const content = useRef<HTMLDivElement>(null);
  if (expanded) {
    // Large over the window, as the app's picture viewer: the surround dimmed, Esc or a click
    // outside goes back to the side panel. Content at the left, properties in a column at the right.
    return (
      <Dialog open onOpenChange={open => !open && post({ type: 'ticket_collapse' })}>
        <DialogContent size="xl" showCloseButton={false} initialFocus={content} className="h-[calc(100dvh-4rem)] max-w-5xl overflow-hidden p-0">
          <DialogTitle className="sr-only">{ticket.title}</DialogTitle>
          <Header page={page} expanded />
          <div className="flex min-h-0 flex-1">
            <div ref={content} tabIndex={-1} className="min-h-0 min-w-0 flex-1 overflow-y-auto outline-none">
              <article className="mx-auto max-w-3xl px-10 py-8">
                <Body page={page} properties={false} onAddSub={onAddSub} />
              </article>
            </div>
            <aside className="w-72 shrink-0 overflow-y-auto border-l border-border bg-surface-panel px-5 py-6">
              <h2 className="mb-2 text-xs font-medium text-muted-foreground">{page.words.properties}</h2>
              <Properties {...page} />
            </aside>
          </div>
        </DialogContent>
      </Dialog>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      <Header page={page} expanded={false} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <article className="mx-auto max-w-3xl px-4 py-6 sm:px-8 sm:py-10">
          <Body page={page} properties onAddSub={onAddSub} />
        </article>
      </div>
    </div>
  );
}
