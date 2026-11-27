// Derived from Circle, components/layout/headers/issue/header-nav.tsx,
// components/common/issues/details/issue-details.tsx, issue-properties-panel.tsx and
// activity-feed.tsx, and components/common/issues/status-selector.tsx (github.com/ln-dev7/circle at
// b0ba7dd): the issue page's header with previous and next, its title and description, the
// sub-issues, the activity feed with its comment cards and comment box, the properties column and
// the status menu, without reactions, replies and the rich description. MIT, Copyright (c) 2025
// lndev-ui | Circle Template.
// Here: one part ticket, narrow in the side panel (properties under the title) or large over the
// window (properties in a column at the right). A click on its title, description or requirement
// lines edits them where they stand. What it says is shown as plain text. The controls are Mastra's.
import { Ban, Box, Camera, ChevronDown, ChevronUp, CircleDot, ClipboardCheck, Ellipsis, FileText, Maximize2, Minimize2, Paperclip, PenLine, Pencil, Plus, Trash2, Unlock, X } from 'lucide-react';
import type { ReactNode } from 'react';
import { useRef, useState } from 'react';
import type { CheckItem, Ticket, TicketAction, TicketAttachment, TicketEvent, TicketRounds, TicketStateName, TicketsState } from '../host';
import { ask, lastTickets, pictureUrl, post } from '../host';
import { childrenOf, fill, lines, MAX_COMMENT, MAX_REASON, MAX_TEXT, MAX_TITLE, moment, names, ordered, progress, refusalText, stateWord, whyNotAssistant, whyNotDone } from './shared';
import type { Full, Words } from './shared';
import { StateIcon } from './status-icons';
import { AttachmentCard } from '@/ds/components/AttachmentCard';
import { Avatar } from '@/ds/components/Avatar';
import { Badge } from '@/ds/components/Badge';
import { Breadcrumb, Crumb } from '@/ds/components/Breadcrumb';
import { Button } from '@/ds/components/Button';
import { Dialog, DialogAction, DialogBody, DialogCancel, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/ds/components/Dialog';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { Input } from '@/ds/components/Input';
import { Notice } from '@/ds/components/Notice';
import { Switch } from '@/ds/components/Switch';
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
  declined: 'didDeclined',
  deleted: 'didDeleted',
  restored: 'didRestored',
  taken: 'didTaken',
  moved: 'didMoved',
  finished: 'didFinished',
  check_off: 'didCheckOff',
  check_on: 'didCheckOn',
  recheck: 'didRecheck',
  view_attached: 'didViewAttached',
} as const;

/** The states a person may set from the menu, in Linear's order; a draft is accepted, not set. */
const SETTABLE: TicketStateName[] = ['confirmed', 'in_progress', 'blocked', 'waiting', 'done'];

function actorWord(words: Words, actor: TicketEvent['actor']) {
  return actor === 'person' ? words.actorPerson : actor === 'agent' ? words.actorAgent : words.actorApp;
}

/** Why the Assistant's rounds stopped, in words. */
const STOPPED = {
  clean: 'stopClean',
  advisory: 'stopAdvisory',
  failed: 'stopFailed',
  refused: 'stopRefused',
  person: 'stopPerson',
  assistant: 'stopAssistant',
} as const;

/** The Assistant's rounds as a ticket or a line says them: 「第 2 轮，共 3 轮 · 等助手改」, 「首次检查 · 检查中」, 「已停：只剩建议 · 第 3 轮，共 3 轮」. */
function roundsText(rounds: TicketRounds | null | undefined, checking: boolean, words: Words) {
  if (!rounds) return null;
  const where = roundWhere(rounds, words);
  if (rounds.stopped) return `${words.roundsStopped}: ${words[STOPPED[rounds.stopped]]} · ${where}`;
  return `${where} · ${checking ? words.roundsChecking : words.roundsFixing}`;
}

/** Which round it is: 「首次检查」 or 「第 2 轮，共 3 轮」. */
function roundWhere(rounds: TicketRounds, words: Words) {
  return rounds.round > 0 ? fill(words.roundOf, { round: String(rounds.round), of: String(rounds.of) }) : words.roundFirst;
}

/** Whether a check of the ticket is asked for or running. */
function checking(ticket: Ticket) {
  return ticket.check?.state === 'asked' || ticket.check?.state === 'running';
}

/** A check's own word by its kind (「DFM 检查」), as the host names it. */
function kindWord(state: TicketsState, words: Words, kind?: string) {
  return state.kinds?.[kind ?? 'dfm'] ?? words.checkTitle;
}

/** A run's rules, the ones that found something first. */
const ITEM_ORDER = ['failed', 'advisory', 'passed', 'checked', 'unsupported'];

function itemWord(words: Words, state: string) {
  return state === 'failed' ? words.failed : state === 'advisory' ? words.advisory : state === 'unsupported' ? words.notCovered : words.checked;
}

const rank = (state: string) => (ITEM_ORDER.includes(state) ? ITEM_ORDER.indexOf(state) : 3);

/** What a run came out with, in the person's words: `2 建议 · 8 已检查 · 3 未覆盖`, failed first, none left out but zeros. */
function outcome(counts: Record<string, number>, words: Words) {
  const checked = (counts.checked ?? 0) + (counts.passed ?? 0);
  const named: [number, string][] = [
    [counts.failed ?? 0, words.failed],
    [counts.advisory ?? 0, words.advisory],
    [checked, words.checked],
    [counts.unsupported ?? 0, words.notCovered],
  ];
  return named.filter(([n]) => n > 0).map(([n, word]) => `${n} ${word}`);
}

/** A run's counts from its items. */
function counted(items: CheckItem[]) {
  const counts: Record<string, number> = {};
  for (const item of items) counts[item.state] = (counts[item.state] ?? 0) + 1;
  return counts;
}

/** The items a check's run reported, under its line in the record: state, parts, what was found against what is required, the item's own name small at the end, and the run at the foot. */
function CheckItems({ items, run, words }: { items: CheckItem[]; run?: string; words: Words }) {
  const sorted = [...items].sort((a, b) => rank(a.state) - rank(b.state));
  return (
    <div className="mt-1.5">
      <ul className="flex flex-col border-t border-border" aria-label={fill(words.checkItems, { n: String(items.length) })}>
        {sorted.map(item => (
          <li key={item.rule} className="flex min-h-8 min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-border py-1 text-xs">
            <Badge size="sm" emphasis="subtle" variant={item.state === 'failed' ? 'destructive' : item.state === 'advisory' ? 'warning' : 'neutral'}>
              {itemWord(words, item.state)}
            </Badge>
            {item.parts.length > 0 && <span className="min-w-0 text-foreground/90">{item.parts.join('、')}</span>}
            {(item.values ?? []).length > 0 && <span className="min-w-0">{(item.values ?? []).join('；')}</span>}
            {item.count > 1 && <span className="tabular-nums">×{item.count}</span>}
            <span className="ml-auto min-w-0 font-mono break-all text-muted-foreground/70">{item.rule}</span>
          </li>
        ))}
      </ul>
      {run && <p className="pt-1 font-mono text-xs text-muted-foreground/70">{run}</p>}
    </div>
  );
}

/** What a person or the Assistant wrote with a step: what it changed, why it stopped, why it went back. */
function said(event: TicketEvent) {
  return ['finished', 'blocked', 'unblocked', 'moved'].includes(event.action) && event.actor !== 'app' ? event.note : undefined;
}

function EventRow({ event, words, language, items, counts, kind }: { event: TicketEvent; words: Words; language: string; items?: CheckItem[]; counts?: Record<string, number>; kind: string }) {
  const [open, setOpen] = useState(false);
  // The Assistant's 「做完了」 asked for this check: its round goes with the line.
  const auto = event.action === 'check_asked' && event.actor === 'agent';
  const named = auto ? words.didCheckAskedAuto : event.action in DID ? words[DID[event.action as keyof typeof DID]] : event.action;
  const did = fill(named, { state: event.to ? stateWord(words, event.to) : '' });
  // A step taken with another ticket. A run is not named on its line (its id is at the foot of its items).
  const together = event.note?.match(/^with (T-\d+)$/);
  const ran = event.action === 'check_ran' || event.action === 'check_started';
  // Where the rounds stood: a check asked for names its round, a result says what came of it.
  const rounds = !event.rounds ? null : auto ? roundWhere(event.rounds, words) : event.action === 'check_ran' || event.action === 'check_refused' ? roundsText(event.rounds, false, words) : null;
  const extra = rounds ?? (ran ? null : together ? fill(words.withTicket, { id: together[1] ?? '' }) : said(event) || event.action === 'comment_view' ? null : event.note);
  // The check's one result line: the kind of check, how its run ended (the run's own word when it
  // is not `done`: superseded, cancelled, failed), and what came out.
  const ended = event.note?.split(' · ')[0];
  const result =
    event.action === 'check_ran' ? [`${kind} ${!ended || ended === 'done' ? words.checkRan : ended}`, ...outcome(items ? counted(items) : (counts ?? {}), words)].join(' · ') : null;
  return (
    <>
      <div className="flex items-center gap-2.5 py-1.5 text-sm text-muted-foreground">
        <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-fill">{EVENT_ICONS[event.action] ?? <CircleDot className="size-3.5" />}</span>
        {result ? (
          // What came out is read whole: the line wraps rather than cut its counts.
          <span className="min-w-0 flex-1 break-words">
            <span className="text-foreground/90">{result}</span> <span className="text-xs whitespace-nowrap">· {moment(event.at, language)}</span>
          </span>
        ) : (
          <>
            <span className="min-w-0 truncate">
              <span className="font-medium text-foreground/90">{actorWord(words, event.actor)}</span> {did}
            </span>
            <span className="shrink-0 text-xs">· {moment(event.at, language)}</span>
          </>
        )}
        {event.action === 'check_ran' && items && items.length > 0 && (
          <Button size="sm" variant="ghost" aria-expanded={open} className="-my-1 ml-auto shrink-0" onClick={() => setOpen(!open)}>
            {fill(words.checkItems, { n: String(items.length) })}
            {open ? <ChevronUp /> : <ChevronDown />}
          </Button>
        )}
      </div>
      {extra && <p className="-mt-1 pb-1.5 pl-7.5 text-xs break-words whitespace-pre-wrap text-muted-foreground">{extra}</p>}
      {open && items && (
        <div className="pb-1.5 pl-7.5">
          <CheckItems items={items} run={event.run} words={words} />
        </div>
      )}
      {(said(event) || (event.attachments && event.attachments.length > 0)) && <CommentCard event={event} words={words} language={language} />}
    </>
  );
}

/** A comment as a card: who wrote it, when, what, and its files. A step's own text shows the same way. */
function CommentCard({ event, words, language }: { event: TicketEvent; words: Words; language: string }) {
  const who = actorWord(words, event.actor);
  const text = event.action === 'comment' ? event.note : said(event);
  return (
    <div className="my-2 rounded-lg border border-border/60 bg-card p-3.5" data-comment={event.at}>
      <div className="mb-1.5 flex items-center gap-2">
        <Avatar name={who} size="inline" />
        <span className="text-sm font-medium">{who}</span>
        <span className="text-xs text-muted-foreground">{moment(event.at, language)}</span>
      </div>
      {text && <p className="my-1.5 text-sm break-words whitespace-pre-wrap">{text}</p>}
      {event.attachments && event.attachments.length > 0 && <Files files={event.attachments} className="mt-2" />}
    </div>
  );
}

/** The record, oldest first: steps and comments in one feed, and the box a person writes a comment in. */
function Activity({ ticket, state, words }: Page) {
  const [draft, setDraft] = useState('');
  // Files taken in for this comment that the person took off again before sending.
  const [removed, setRemoved] = useState<string[]>([]);
  const files = (state.commenting ?? []).filter(file => !removed.includes(file.path));
  const [sending, setSending] = useState(false);
  const submitComment = () => {
    if ((!draft.trim() && files.length === 0) || sending) return;
    setSending(true);
    ask({ type: 'ticket_comment', id: ticket.id, text: draft.trim(), attachments: files.map(file => file.path) }, ok => {
      setSending(false);
      if (ok) {
        setDraft('');
        setRemoved([]);
      }
    });
  };
  // A check that has run takes one line: its start is folded into the result.
  const record = state.record.filter((event, at) => event.action !== 'check_started' || !state.record.slice(at + 1).some(later => later.action === 'check_ran' || later.action === 'check_refused'));
  const counts = (run?: string) => (run && ticket.check?.run === run ? ticket.check.counts : undefined);
  return (
    <section className="mt-10" aria-label={words.activity}>
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-base font-semibold">{words.activity}</h2>
      </div>

      <div className="flex flex-col">
        {record.map((event, at) =>
          event.action === 'comment' ? (
            <CommentCard key={at} event={event} words={words} language={state.language} />
          ) : (
            <EventRow
              key={at}
              event={event}
              words={words}
              language={state.language}
              items={event.run ? state.items?.[event.run] : undefined}
              counts={counts(event.run)}
              kind={kindWord(state, words, event.kind)}
            />
          ),
        )}
      </div>

      {/* Composer */}
      <div className="mt-3 flex flex-col gap-2 rounded-lg border border-border/60 bg-card p-3" data-ticket={ticket.id}>
        {files.length > 0 && (
          <div className="flex flex-wrap gap-2" aria-label={words.attachments}>
            {files.map(file => (
              <span key={file.path} className="flex min-w-0 items-center gap-1">
                <Badge variant="neutral" emphasis="subtle" size="sm" icon={file.picture ? <img src={pictureUrl(file.picture)} alt="" className="size-4 rounded-sm object-cover" draggable={false} /> : <FileText aria-hidden />}>
                  <span className="truncate">{file.name}</span>
                </Badge>
                <Button type="button" size="icon-sm" variant="ghost" aria-label={`${words.removeFile} ${file.name}`} onClick={() => setRemoved([...removed, file.path])}>
                  <X />
                </Button>
              </span>
            ))}
          </div>
        )}
        <textarea
          value={draft}
          onChange={event => setDraft(event.target.value)}
          onKeyDown={event => {
            if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
              submitComment();
            }
          }}
          aria-label={words.comment}
          placeholder={words.commentPlaceholder}
          maxLength={MAX_COMMENT}
          rows={2}
          className="w-full resize-none bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        <div className="flex items-center justify-between">
          <DropdownMenu>
            <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={words.attach} tooltip={words.attach} className="-ml-1.5">
              <Plus />
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="start">
              <DropdownMenu.Item onClick={() => ask({ type: 'ticket_comment_attach', id: ticket.id, view: false })}>
                <FileText />
                {words.attachFile}
              </DropdownMenu.Item>
              <DropdownMenu.Item onClick={() => ask({ type: 'ticket_comment_attach', id: ticket.id, view: true })}>
                <Camera />
                {words.attachViewNow}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu>
          <Button size="sm" onClick={submitComment} disabled={sending || (!draft.trim() && files.length === 0)}>
            {words.comment}
          </Button>
        </div>
      </div>
    </section>
  );
}

/**
 * The check in the run's own words: a run that ran shows its id, outcome word and counts first;
 * 未检查 where it checked nothing (the store's `unchecked`) or where no check applies.
 */
function checkText({ ticket, state, words }: Page) {
  const check = ticket.check;
  if (check?.state === 'ran') {
    const counts = check.counts ?? {};
    // A passed rule was checked: the run's own `checked` and `passed` are one count here.
    const checked = (counts.checked ?? 0) + (counts.passed ?? 0);
    const named: [string, number | undefined, string][] = [
      ['failed', counts.failed, words.failed],
      ['advisory', counts.advisory, words.advisory],
      ['checked', counts.checked !== undefined || counts.passed !== undefined ? checked : undefined, words.checked],
      ['unsupported', counts.unsupported, words.notCovered],
    ];
    const parts = named.filter(([, n]) => n !== undefined).map(([, n, word]) => `${n} ${word}`);
    parts.push(...Object.entries(counts).filter(([key]) => key !== 'passed' && !named.some(([known]) => known === key)).map(([key, n]) => `${n} ${key}`));
    const outcome = check.outcome === 'done' ? words.checkDone : check.outcome;
    const now = check.run ? state.freshness[check.run] : undefined;
    const nothing = ticket.checks && ticket.unchecked ? words.notChecked : null;
    // The run's id is at the foot of its opened items in 「动态」, not here.
    return [nothing, kindWord(state, words, check.kind), outcome, ...parts, now && now !== 'current' ? words.checkStale : null].filter(Boolean).join(' · ');
  }
  if (ticket.checksOff && ticket.parts.length > 0) return words.checkOff;
  if (!ticket.checks) return `${words.notChecked} · ${words.noApplicableCheck}`;
  if (!check) return words.noCheck;
  if (check.state === 'asked') return words.checkAsked;
  if (check.state === 'running') return words.checkRunning;
  return [words.checkRefused, check.reason].filter(Boolean).join(' · ');
}

/**
 * Text that becomes its editor on a click and saves itself when the person leaves it; Esc leaves
 * it as it was. What was typed stays shown until the host has taken the save, and a refused save
 * opens the editor again with it, so nothing typed is lost.
 */
function Editable({ value, multiline, label, empty, onSave, children }: { value: string; multiline?: boolean; label: string; empty?: string; onSave: (next: string, then: (ok: boolean) => void) => void; children: ReactNode }) {
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  if (draft === null) {
    return (
      <div
        role="button"
        tabIndex={0}
        aria-label={label}
        data-edit={label}
        onClick={() => setDraft(saving ?? value)}
        onKeyDown={event => event.key === 'Enter' && setDraft(saving ?? value)}
        className="-mx-2 cursor-text rounded-md px-2 py-0.5 hover:bg-fill-subtle focus-visible:bg-fill-subtle focus-visible:outline-none"
      >
        {saving !== null ? <p className="text-sm break-words whitespace-pre-wrap text-muted-foreground">{saving}</p> : value ? children : <span className="text-sm text-muted-foreground">{empty}</span>}
      </div>
    );
  }
  const finish = () => {
    const typed = draft;
    setDraft(null);
    if (typed === value) return;
    setSaving(typed);
    onSave(typed, ok => {
      setSaving(null);
      if (!ok) setDraft(typed);
    });
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

// A ticket's saves go one at a time: the next waits for the host's answer to the one before, and is
// then made on the ticket as the host has it now (its new revision), so two saves made in quick
// succession are not refused as made on an old one.
let saving = false;
const queued: (() => void)[] = [];

/** Save a change of ticket `id`, built on the ticket as the host last told it when its turn comes. */
function save(id: string, build: (ticket: Ticket) => TicketAction | null, then?: (ok: boolean) => void) {
  const run = () => {
    const ticket = lastTickets()?.tickets.find(other => other.id === id);
    const action = ticket ? build(ticket) : null;
    if (!action) {
      then?.(false);
      queued.shift()?.();
      return;
    }
    saving = true;
    ask(action, ok => {
      saving = false;
      then?.(ok);
      queued.shift()?.();
    });
  };
  if (saving) queued.push(run);
  else run();
}

/** Save one field of the ticket's text; the others are not sent and stay as they are. */
function edit({ ticket }: Page, change: { title: string } | { text: string } | { requirements: string[] }, then: (ok: boolean) => void) {
  if ('title' in change && !change.title.trim()) return then(false);
  save(ticket.id, latest => ({ type: 'ticket_edit', id: latest.id, revision: latest.revision, ...change }), then);
}

/** "It still waits for T-3 and T-5": what a person is reminded of, not stopped by, when they move a waiting ticket. */
function still({ ticket, words, state }: Page) {
  const waits = ticket.openWaits;
  return waits.length ? fill(words.stillWaits, { ids: names(waits, state.language) }) : null;
}

/** A question before a step that takes other tickets with it: the primary answer, another (optional), or Cancel. */
export function Choice({ title, primary, secondary, destructive, cancel, onPrimary, onSecondary, onClose }: { title: string; primary: string; secondary?: string; destructive?: boolean; cancel: string; onPrimary: () => void; onSecondary?: () => void; onClose: () => void }) {
  return (
    <Dialog open intent={destructive ? 'destructive' : 'default'} onOpenChange={open => !open && onClose()}>
      <DialogContent showCloseButton={false} size="sm">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogFooter>
          <DialogCancel>{cancel}</DialogCancel>
          {secondary && onSecondary && (
            <Button
              size="sm"
              onClick={() => {
                onSecondary();
                onClose();
              }}
            >
              {secondary}
            </Button>
          )}
          <DialogAction
            onConfirm={() => {
              onPrimary();
              onClose();
            }}
          >
            {primary}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Accept a draft: with its draft sub-tickets, or alone; asked first when it has any. */
export function useAccept(ticket: Full, tickets: Full[], words: Words) {
  const [asking, setAsking] = useState(false);
  const subs = childrenOf(ticket, tickets).filter(child => child.state === 'draft' && !child.deleted).length;
  const send = (withSubs: boolean) => ask({ type: 'ticket_confirm', id: ticket.id, revision: ticket.revision, with_subs: withSubs });
  const dialog = asking ? (
    <Choice title={fill(words.confirmWithSubs, { n: String(subs) })} primary={words.together} secondary={words.onlyThis} cancel={words.cancel} onPrimary={() => send(true)} onSecondary={() => send(false)} onClose={() => setAsking(false)} />
  ) : null;
  return { accept: () => (subs ? setAsking(true) : send(false)), dialog };
}

/** Delete (or decline a draft): its live sub-tickets go with it, asked first. A success says so with Undo; a refusal says why on the page and nothing else. */
export function useRemove(ticket: Full, tickets: Full[], words: Words, kind: 'ticket_delete' | 'ticket_decline') {
  const [asking, setAsking] = useState(false);
  const subs = childrenOf(ticket, tickets).filter(child => !child.deleted).length;
  const send = () =>
    ask({ type: kind, id: ticket.id, revision: ticket.revision }, ok => {
      if (ok) toast(fill(words.deletedToast, { id: ticket.id }), { action: { label: words.undo, onClick: () => ask({ type: 'ticket_restore', id: ticket.id }) } });
    });
  const dialog = asking ? <Choice title={fill(words.deleteWithSubs, { n: String(subs) })} primary={kind === 'ticket_delete' ? words.delete : words.decline} destructive cancel={words.cancel} onPrimary={send} onClose={() => setAsking(false)} /> : null;
  return { remove: () => (subs ? setAsking(true) : send()), dialog };
}

/** How far along a state is: a move to a smaller one is a move back. */
const RANK: Record<TicketStateName, number> = { draft: 0, confirmed: 1, in_progress: 2, blocked: 2, waiting: 3, done: 4 };

/** Moving a ticket back: why, asked once and optional, kept on the move's line. */
function WhyBack({ to, words, onMove, onClose }: { to: TicketStateName; words: Words; onMove: (reason: string) => void; onClose: () => void }) {
  const [reason, setReason] = useState('');
  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent showCloseButton={false} size="sm">
        <DialogHeader>
          <DialogTitle>{fill(words.backTitle, { state: stateWord(words, to) })}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <Input autoFocus aria-label={words.backWhy} placeholder={words.backWhy} maxLength={MAX_REASON} value={reason} onChange={event => setReason(event.target.value)} />
        </DialogBody>
        <DialogFooter>
          <DialogCancel>{words.cancel}</DialogCancel>
          <DialogAction
            onConfirm={() => {
              onMove(reason.trim());
              onClose();
            }}
          >
            {words.backConfirm}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** The state, and the menu a person changes it with; a step that cannot be taken now says why. */
function StateMenu(page: Page) {
  const { ticket, words } = page;
  const [back, setBack] = useState<TicketStateName | null>(null);
  const why: Partial<Record<TicketStateName, string | null>> = { done: ticket.state === 'done' ? null : whyNotDone(ticket, words) };
  const reminder = still(page);
  const move = (to: TicketStateName, reason?: string) => ask({ type: 'ticket_set_state', id: ticket.id, revision: ticket.revision, to, ...(reason ? { reason } : {}) });
  return (
    <>
    {back && <WhyBack to={back} words={words} onMove={reason => move(back, reason)} onClose={() => setBack(null)} />}
    <DropdownMenu>
      <DropdownMenu.Trigger variant="ghost" size="sm" aria-label={words.state} className="-ml-2">
        <StateIcon state={ticket.state} />
        {stateWord(words, ticket.state)}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="start" className="min-w-64">
        {reminder && <DropdownMenu.Label className="max-w-72 whitespace-normal">{reminder}</DropdownMenu.Label>}
        <DropdownMenu.RadioGroup
          value={ticket.state}
          onValueChange={(picked: unknown) => {
            const to = picked as TicketStateName;
            if (RANK[to] < RANK[ticket.state]) setBack(to);
            else move(to);
          }}
        >
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
    </>
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
  const assign = () => save(ticket.id, latest => ({ type: 'ticket_assign_parts', id: latest.id, revision: latest.revision }));
  const others = tickets.filter(other => other.id !== ticket.id && other.parent !== ticket.id);
  // Built on the ticket as the host has it when its turn comes: two quick ticks both count.
  const toggle = (id: string) =>
    save(ticket.id, latest => {
      const waits = latest.waits ?? [];
      return { type: 'ticket_waits', id: latest.id, revision: latest.revision, waits: waits.includes(id) ? waits.filter(other => other !== id) : [...waits, id] };
    });
  const switchCheck = (on: boolean) => save(ticket.id, latest => ({ type: 'ticket_edit', id: latest.id, revision: latest.revision, checks: on }));
  return (
    <section className="flex flex-col" aria-label={words.properties}>
      <Prop label={words.state}>
        <StateMenu {...page} />
      </Prop>
      {ticket.state === 'blocked' && ticket.blocked && (
        <Prop label={words.stateBlocked}>
          {/* Rounds that stopped it are said in the page's words, with the check's own reason where it could not run. */}
          <span className="whitespace-pre-wrap text-muted-foreground">
            {ticket.rounds?.stopped === 'failed' || ticket.rounds?.stopped === 'refused'
              ? [roundsText(ticket.rounds, false, words), ticket.rounds.stopped === 'refused' ? ticket.check?.reason : null].filter(Boolean).join(' · ')
              : ticket.blocked}
          </span>
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
        <span className={ticket.check?.state === 'ran' ? undefined : 'text-muted-foreground'}>{checkText(page)}</span>
        {/* The rounds while they go on; once they stopped a ticket, its 受阻 row says why. */}
        {ticket.rounds && !ticket.rounds.stopped && <span className="text-xs text-muted-foreground">{roundsText(ticket.rounds, checking(ticket), words)}</span>}
        {ticket.recheck && (
          <span className="flex min-w-0 items-center gap-1.5">
            <Badge variant="warning" emphasis="subtle" size="sm">
              {words.recheck}
            </Badge>
            <span className="min-w-0 text-xs break-words text-muted-foreground">{ticket.recheck}</span>
          </span>
        )}
        {ticket.parts.length > 0 && (
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <Switch checked={!ticket.checksOff} onCheckedChange={(on: boolean) => switchCheck(on)} aria-label={words.needsCheck} />
            {words.needsCheck}
          </label>
        )}
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
  const step = (type: 'ticket_start' | 'ticket_ask_check' | 'ticket_done', then?: (ok: boolean) => void) => () => ask({ type, id: ticket.id, revision: ticket.revision }, then);
  const { accept, dialog: accepting } = useAccept(ticket, tickets, words);
  const { remove: decline, dialog: declining } = useRemove(ticket, tickets, words, 'ticket_decline');
  let buttons: ReactNode = null;
  let why: string | null = null;
  let note: string | null = null;
  if (ticket.state === 'draft') {
    buttons = (
      <>
        <Button size="sm" variant="primary" onClick={accept}>
          {words.confirm}
        </Button>
        <Button size="sm" variant="ghost" onClick={decline}>
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
        <Button size="sm" onClick={() => ask({ type: 'ticket_work_myself', id: ticket.id, revision: ticket.revision }, ok => ok && reminder && toast(reminder))}>
          {words.workMyself}
        </Button>
      </>
    );
  } else if (ticket.state === 'in_progress' && ticket.worker === 'assistant' && ticket.checks && (ticket.finished || (ticket.rounds && !ticket.rounds.stopped))) {
    // The Assistant says it is finished, or works its rounds of check and fix: a person may run the
    // check at any time (which ends the rounds), except while one runs.
    const going = ticket.rounds && !ticket.rounds.stopped;
    note = going ? roundsText(ticket.rounds, checking(ticket), words) : words.finishedNote;
    buttons = checking(ticket) ? null : (
      <Button size="sm" variant="primary" onClick={step('ticket_ask_check')}>
        {words.runCheck}
      </Button>
    );
    if (!buttons && note) return <p className="mt-3 text-sm text-muted-foreground">{note}</p>;
  } else if ((ticket.state === 'in_progress' && ticket.worker === 'person') || ticket.state === 'waiting') {
    if (ticket.state === 'in_progress' && ticket.checks) {
      buttons = (
        <Button size="sm" variant="primary" onClick={step('ticket_ask_check')}>
          {words.askCheck}
        </Button>
      );
    } else {
      why = whyNotDone(ticket, words);
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
      {note && <p className="mb-2 text-sm text-muted-foreground">{note}</p>}
      <div className="flex flex-wrap items-center gap-2">{buttons}</div>
      {why && <p className="mt-1.5 text-xs text-muted-foreground">{why}</p>}
      {accepting}
      {declining}
    </div>
  );
}

/** Files as a ticket or a comment shows them: a picture as its thumbnail, any other file as a card; a click opens it. */
function Files({ files, className, children }: { files: TicketAttachment[]; className?: string; children?: ReactNode }) {
  const open = (path: string) => () => ask({ type: 'ticket_attachment', path });
  const type = (name: string) => name.split('.').pop()?.toUpperCase() ?? '';
  return (
    <div className={`flex min-w-0 flex-wrap items-center gap-2 ${className ?? ''}`}>
      {files.map(file =>
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
      {children}
    </div>
  );
}

function Attachments({ ticket, words }: Page) {
  return (
    <section className="mt-8" aria-label={words.attachments}>
      <h2 className="mb-2 text-sm font-medium">
        {words.attachments} {ticket.attachments.length > 0 && <span className="text-muted-foreground">{ticket.attachments.length}</span>}
      </h2>
      <Files files={ticket.attachments}>
        <Button size="sm" variant="ghost" icon={<Paperclip />} onClick={() => ask({ type: 'ticket_attach', id: ticket.id })}>
          {words.attach}
        </Button>
      </Files>
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
          <Notice.Message>{refusalText(state)}</Notice.Message>
        </Notice>
      )}
      <Editable value={ticket.title} label={words.fieldTitle} onSave={(title, then) => edit(page, { title }, then)}>
        <h1 className="text-xl leading-tight font-semibold break-words whitespace-pre-wrap text-balance sm:text-2xl">{ticket.title}</h1>
      </Editable>
      <Actions {...page} />
      {properties && (
        <div className="mt-5">
          <Properties {...page} />
        </div>
      )}
      <div className="mt-6">
        <Editable value={ticket.text} multiline label={words.fieldText} empty={words.addText} onSave={(text, then) => edit(page, { text }, then)}>
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
          onSave={(typed, then) => {
            const requirements = lines(typed);
            if (requirements) edit(page, { requirements }, then);
            else then(false);
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
      {/* One comment box per ticket: what was typed on one never goes to the next. */}
      <Activity key={ticket.id} {...page} />
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
  const { remove, dialog } = useRemove(ticket, tickets, words, 'ticket_delete');
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
      {dialog}
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
