// Derived in part from Circle, components/layout/sidebar/create-new-issue/index.tsx
// (github.com/ln-dev7/circle at b0ba7dd): the create dialog's title, description and the row of
// properties under them. MIT, Copyright (c) 2025 lndev-ui | Circle Template.
// The tickets view: a project's part tickets as a list, or one ticket's page (in the side panel, or
// large over the window), and the dialog a person creates a ticket with. The host decides which
// ticket is open (`selected`) and whether it is large (`expanded`), and checks every step against
// the ticket's state and the revision the person saw.
import { FileText, Paperclip, Plus } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { TicketsState } from '../host';
import { ask, hear, pictureUrl } from '../host';
import { full, lines, MAX_TEXT, MAX_TITLE, refusalText } from './shared';
import type { Full, Words } from './shared';
import { StateIcon } from './status-icons';
import { TicketList } from './TicketList';
import { PartChip, TicketPage } from './TicketPage';
import { Button } from '@/ds/components/Button';
import { Dialog, DialogAction, DialogBody, DialogCancel, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/ds/components/Dialog';
import { Field, FieldLabel } from '@/ds/components/Field';
import { Input } from '@/ds/components/Input';
import { Notice } from '@/ds/components/Notice';
import { Textarea } from '@/ds/components/Textarea';
import { Toaster } from '@/ds/components/Toaster';

/** A new ticket: title, description, requirement lines, the parts (the model's selection to start with) and files. */
function NewTicket({ state, words, parent, tickets, onClose }: { state: TicketsState; words: Words; parent: string | null; tickets: Full[]; onClose: () => void }) {
  const [title, setTitle] = useState('');
  const [text, setText] = useState('');
  const [typed, setTyped] = useState('');
  const [parts, setParts] = useState(state.selection ?? []);
  // A refused ticket keeps the form open with what was typed, and says why.
  const [refused, setRefused] = useState(false);
  const requirements = lines(typed);
  const fits = title.trim().length > 0 && requirements !== null;
  const adding = state.adding ?? [];
  const above = parent ? tickets.find(ticket => ticket.id === parent) : undefined;
  function save() {
    if (!fits) return;
    setRefused(false);
    ask({ type: 'ticket_create', title: title.trim(), text: text.trim(), requirements, parts, attachments: adding.map(file => file.path), parent }, ok => (ok ? onClose() : setRefused(true)));
  }
  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent showCloseButton={false} size="lg">
        <DialogHeader>
          <DialogTitle>{words.newTicket}</DialogTitle>
          {above && (
            <DialogDescription className="flex min-w-0 items-center gap-1.5">
              <span className="shrink-0">{words.parent}</span>
              <StateIcon state={above.state} />
              <span className="shrink-0 font-medium">{above.id}</span>
              <span className="truncate">{above.title}</span>
            </DialogDescription>
          )}
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          {refused && state.error && (
            <Notice variant="destructive">
              <Notice.Message>{refusalText(state)}</Notice.Message>
            </Notice>
          )}
          <Field className="gap-1">
            <FieldLabel>{words.fieldTitle}</FieldLabel>
            <Input autoFocus maxLength={MAX_TITLE} value={title} onChange={event => setTitle(event.target.value)} />
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.fieldText}</FieldLabel>
            <Textarea rows={3} maxLength={MAX_TEXT} value={text} onChange={event => setText(event.target.value)} />
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.fieldLines}</FieldLabel>
            <Textarea rows={3} value={typed} onChange={event => setTyped(event.target.value)} />
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.part}</FieldLabel>
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              {parts.map(part => (
                <PartChip key={`${part.kind}:${part.id}`} id={part.id} />
              ))}
              <Button size="sm" variant="ghost" icon={<Plus />} disabled={!state.selection?.length} tooltip={state.selection?.length ? undefined : words.selectFirst} onClick={() => setParts(state.selection ?? [])}>
                {words.useSelection}
              </Button>
            </div>
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.attachments}</FieldLabel>
            <div className="flex min-w-0 flex-wrap items-center gap-1.5">
              {adding.map(file => (
                <span key={file.path} className="inline-flex h-7 max-w-full min-w-0 items-center gap-1.5 rounded-md border border-border pr-2 pl-1 text-xs">
                  {file.picture ? <img src={pictureUrl(file.picture)} alt="" className="size-5 rounded-sm object-cover" draggable={false} /> : <FileText className="size-4 text-muted-foreground" aria-hidden />}
                  <span className="truncate">{file.name}</span>
                </span>
              ))}
              <Button size="sm" variant="ghost" icon={<Paperclip />} onClick={() => ask({ type: 'ticket_attach', id: null })}>
                {words.attach}
              </Button>
            </div>
          </Field>
        </DialogBody>
        <DialogFooter>
          <DialogCancel>{words.cancel}</DialogCancel>
          <DialogAction disabled={!fits} onConfirm={save}>
            {words.create}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function Tickets({ state }: { state: TicketsState }) {
  // `undefined`: no form; `null`: a new top ticket; an id: a new sub-ticket of that ticket.
  const [creating, setCreating] = useState<string | null | undefined>(undefined);
  const tickets = state.tickets.map(full);
  const ticket = state.selected ? tickets.find(candidate => candidate.id === state.selected) : undefined;
  const expanded = Boolean(ticket && state.expanded);
  // Each state the host pushes answers the request it was pushed for.
  useEffect(() => hear(state), [state]);
  return (
    // Large, the ticket lies over the app's own window, which the dimmed surround lets through.
    <div className={`h-dvh overflow-hidden text-foreground ${expanded ? '' : 'bg-background'}`}>
      {ticket ? (
        <TicketPage state={state} ticket={ticket} tickets={tickets} expanded={expanded} onAddSub={() => setCreating(ticket.id)} />
      ) : (
        <TicketList state={state} tickets={tickets} onNew={() => setCreating(null)} />
      )}
      {creating !== undefined && <NewTicket state={state} words={state.strings} parent={creating} tickets={tickets} onClose={() => setCreating(undefined)} />}
      <Toaster theme={state.dark ? 'dark' : 'light'} position="bottom-center" />
    </div>
  );
}
