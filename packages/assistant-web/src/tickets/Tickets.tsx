// The tickets view: a project's part tickets as a list, or one ticket's page, and the dialogs a
// person creates, edits and deletes a ticket with. The host decides which ticket is open
// (`selected`) and checks every step against the ticket's state and the revision the person saw.
import { useState } from 'react';
import type { Ticket, TicketsState } from '../host';
import { post } from '../host';
import { fill } from './shared';
import type { Words } from './shared';
import { TicketList } from './TicketList';
import { TicketPage } from './TicketPage';
import { Dialog, DialogAction, DialogBody, DialogCancel, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/ds/components/Dialog';
import { Field, FieldLabel } from '@/ds/components/Field';
import { Input } from '@/ds/components/Input';
import { Textarea } from '@/ds/components/Textarea';

/** The same limits as the store (crates/engine/src/project/tickets.rs). */
const MAX_TITLE = 200;
const MAX_TEXT = 8000;
const MAX_LINES = 40;
const MAX_LINE = 400;

/** A dialog open on the page; `revision` is the ticket's as the person saw it when the dialog opened. */
type Open = { kind: 'create' } | { kind: 'edit'; ticket: Ticket } | { kind: 'delete'; ticket: Ticket } | null;

function TicketForm({ words, ticket, onClose }: { words: Words; ticket?: Ticket; onClose: () => void }) {
  const [title, setTitle] = useState(ticket?.title ?? '');
  const [text, setText] = useState(ticket?.text ?? '');
  const [lines, setLines] = useState(ticket?.requirements.join('\n') ?? '');
  const requirements = lines
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
  const fits = title.trim().length > 0 && requirements.length <= MAX_LINES && requirements.every(line => line.length <= MAX_LINE);
  function save() {
    if (!fits) return;
    const body = { title: title.trim(), text: text.trim(), requirements };
    post(ticket ? { type: 'ticket_edit', id: ticket.id, revision: ticket.revision, ...body } : { type: 'ticket_create', ...body });
    onClose();
  }
  const heading = ticket ? fill(words.editTitle, { id: ticket.id }) : words.newTicket;
  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{heading}</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          <Field className="gap-1">
            <FieldLabel>{words.fieldTitle}</FieldLabel>
            <Input autoFocus maxLength={MAX_TITLE} value={title} onChange={event => setTitle(event.target.value)} />
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.fieldText}</FieldLabel>
            <Textarea rows={4} maxLength={MAX_TEXT} value={text} onChange={event => setText(event.target.value)} />
          </Field>
          <Field className="gap-1">
            <FieldLabel>{words.fieldLines}</FieldLabel>
            <Textarea rows={4} value={lines} onChange={event => setLines(event.target.value)} />
          </Field>
        </DialogBody>
        <DialogFooter>
          <DialogCancel>{words.cancel}</DialogCancel>
          <DialogAction disabled={!fits} onConfirm={save}>
            {ticket ? words.save : words.create}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function DeleteDialog({ words, ticket, onClose }: { words: Words; ticket: Ticket; onClose: () => void }) {
  return (
    <Dialog open intent="destructive" onOpenChange={open => !open && onClose()}>
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{fill(words.deleteAsk, { id: ticket.id })}</DialogTitle>
          <DialogDescription className="break-words">{ticket.title}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <DialogCancel>{words.cancel}</DialogCancel>
          <DialogAction
            onConfirm={() => {
              post({ type: 'ticket_delete', id: ticket.id, revision: ticket.revision });
              onClose();
            }}
          >
            {words.delete}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function Tickets({ state }: { state: TicketsState }) {
  const [open, setOpen] = useState<Open>(null);
  const words = state.strings;
  const ticket = state.selected ? state.tickets.find(candidate => candidate.id === state.selected) : undefined;
  const close = () => setOpen(null);
  return (
    <div className="h-dvh overflow-hidden bg-background text-foreground">
      {ticket ? (
        <TicketPage state={state} ticket={ticket} onEdit={() => setOpen({ kind: 'edit', ticket })} onDelete={() => setOpen({ kind: 'delete', ticket })} />
      ) : (
        <TicketList state={state} onNew={() => setOpen({ kind: 'create' })} />
      )}
      {open?.kind === 'create' && <TicketForm words={words} onClose={close} />}
      {open?.kind === 'edit' && <TicketForm words={words} ticket={open.ticket} onClose={close} />}
      {open?.kind === 'delete' && <DeleteDialog words={words} ticket={open.ticket} onClose={close} />}
    </div>
  );
}
