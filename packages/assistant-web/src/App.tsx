// The Assistant's conversation and composer, drawn with Mastra's playground-ui components and
// fed with the desktop app's own transcript. Everything visual is imported from the Mastra
// checkout; this file only maps our items onto those components.
import {
  ArrowUp,
  Camera,
  ChevronDown,
  Ellipsis,
  FileText,
  Image,
  MessageSquare,
  MousePointerClick,
  Pencil,
  Plus,
  ShieldCheck,
  Square,
  SquarePen,
  Target,
  Undo2,
  X,
} from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react';
import type { AssistantState, ContextChip, ContextGroups, Item, PermissionOption, PictureRef, QueuedMessage, Strings } from './host';
import { pictureUrl, post } from './host';
import { Activity, ActivityContent, ActivityHeadline, ActivityTrigger } from '@/ds/components/ai/activity';
import type { TaskListItem } from '@/ds/components/ai/task-list';
import { TaskList, TaskListContainer } from '@/ds/components/ai/task-list';
import { ToolCallGroup, ToolCallOutput, presentTool } from '@/ds/components/ai/tool-call';
import { AttachmentCard } from '@/ds/components/AttachmentCard';
import { Badge } from '@/ds/components/Badge';
import { Button } from '@/ds/components/Button';
import { ChatShell } from '@/ds/components/ChatShell';
import { Combobox } from '@/ds/components/Combobox';
import type { ComboboxOption } from '@/ds/components/Combobox';
import {
  Composer,
  ComposerActions,
  ComposerActionsEnd,
  ComposerActionsStart,
  ComposerAttachments,
  ComposerBox,
  ComposerInput,
  ComposerRing,
  ComposerStatus,
} from '@/ds/components/Composer';
import { CopyButton } from '@/ds/components/CopyButton';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { EmptyState } from '@/ds/components/EmptyState';
import { MarkdownRenderer } from '@/ds/components/MarkdownRenderer';
import { Message } from '@/ds/components/Message';
import { MessageScrollerItem } from '@/ds/components/MessageScroller';
import { Notice } from '@/ds/components/Notice';
import { PendingIndicator } from '@/ds/components/PendingIndicator';
import { Textarea } from '@/ds/components/Textarea';
import { Txt } from '@/ds/components/Txt';
import { cn } from '@/lib/utils';

type Of<K extends Item['kind']> = Extract<Item, { kind: K }>;
type Tool = Of<'tool'>;
type Approval = Of<'approval'>;

type Part =
  | { kind: 'text'; index: number; text: string; thought: boolean; objects: string[] }
  | { kind: 'tools'; tools: { tool: Tool; approval?: Approval }[] }
  | { kind: 'permission'; item: Of<'permission'> }
  | { kind: 'approval'; item: Approval }
  | { kind: 'changes'; item: Of<'changes'> }
  | { kind: 'notice'; item: Of<'notice'> }
  | { kind: 'skill'; item: Of<'skill'> };

interface Turn {
  id: string;
  prompt?: Of<'user'>;
  parts: Part[];
  plan?: Of<'plan'>['steps'];
}

/** The flat transcript as turns: a prompt, then what the assistant did, each approval beside its tool. */
function turnsOf(items: Item[]): Turn[] {
  const turns: Turn[] = [];
  let turn: Turn | undefined;
  items.forEach((item, index) => {
    if (item.kind === 'user') {
      turn = { id: `turn-${index}`, prompt: item, parts: [] };
      turns.push(turn);
      return;
    }
    if (!turn) {
      turn = { id: `turn-${index}`, parts: [] };
      turns.push(turn);
    }
    const previous = turn.parts.at(-1);
    switch (item.kind) {
      case 'plan':
        turn.plan = item.steps;
        break;
      case 'tool':
        if (previous?.kind === 'tools') previous.tools.push({ tool: item });
        else turn.parts.push({ kind: 'tools', tools: [{ tool: item }] });
        break;
      case 'approval': {
        const held = previous?.kind === 'tools' ? previous.tools.at(-1) : undefined;
        if (held && !held.approval) held.approval = item;
        else turn.parts.push({ kind: 'approval', item });
        break;
      }
      case 'text':
        if (item.text.trim()) {
          turn.parts.push({ kind: 'text', index: item.index, text: item.text.trim(), thought: item.thought, objects: item.objects });
        }
        break;
      default:
        turn.parts.push({ kind: item.kind, item } as Part);
    }
  });
  return turns;
}

/** "batch: solid.box 40 × 30 × 20" as a tool name and what it is doing. */
function nameOf(title: string): { name: string; detail?: string } {
  const at = title.indexOf(': ');
  return at > 0 ? { name: title.slice(0, at), detail: title.slice(at + 2) } : { name: title };
}

function activityStatus(tool: Tool, waiting: boolean) {
  if (tool.status === 'failed') return 'error' as const;
  return tool.status === 'running' && !waiting ? ('running' as const) : ('idle' as const);
}

/** The app's three answers to its own approval request, as Mastra buttons. */
function ApprovalChoices({ approval, strings }: { approval: Approval; strings: Strings }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'approve', id: approval.id, allow: true })}>
        {strings.allowOnce}
      </Button>
      <Button type="button" size="sm" onClick={() => post({ type: 'approve', id: approval.id, allow: 'always' })}>
        {strings.alwaysAllow}
      </Button>
      <Button type="button" size="sm" variant="ghost" onClick={() => post({ type: 'approve', id: approval.id, allow: false })}>
        {strings.deny}
      </Button>
    </div>
  );
}

function approvalBadge(approval: Approval, strings: Strings) {
  if (approval.answer === null) {
    return (
      <Badge variant="warning" emphasis="subtle" size="sm" icon={<ShieldCheck aria-hidden />}>
        {strings.approval}
      </Badge>
    );
  }
  return (
    <Badge variant={approval.answer ? 'success' : 'neutral'} emphasis="subtle" size="sm">
      {approval.answer ? strings.allowed : strings.denied}
    </Badge>
  );
}

function ToolRow({ tool, approval, strings }: { tool: Tool; approval?: Approval; strings: Strings }) {
  const { name, detail } = nameOf(tool.title);
  const waiting = approval?.answer === null;
  const { icon: ToolIcon, label } = presentTool(name, {});
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Activity status={activityStatus(tool, waiting)} aria-label={tool.title}>
        <ActivityTrigger>
          <ActivityHeadline
            icon={<ToolIcon aria-hidden />}
            label={tool.words ?? label}
            detail={tool.words ? undefined : detail}
            badges={approval ? approvalBadge(approval, strings) : undefined}
          />
        </ActivityTrigger>
        <ActivityContent>
          {tool.input && <ToolCallOutput text={tool.input} />}
          {tool.output && <ToolCallOutput text={tool.output} error={tool.status === 'failed'} />}
        </ActivityContent>
      </Activity>
      {/* Outside the row's fold: a request that arrives after the row is drawn must still be seen. */}
      {waiting && approval && (
        <div className="pl-7">
          <ApprovalChoices approval={approval} strings={strings} />
        </div>
      )}
    </div>
  );
}

function Tools({ part, strings }: { part: Extract<Part, { kind: 'tools' }>; strings: Strings }) {
  const rows = part.tools.map(({ tool, approval }) => <ToolRow key={tool.id} tool={tool} approval={approval} strings={strings} />);
  // A finished run of several calls folds into one line; a call that waits for the user stays in view.
  const settled = part.tools.every(({ tool, approval }) => tool.status === 'completed' && approval?.answer !== null);
  if (part.tools.length < 3 || !settled) return <div className="flex min-w-0 flex-col gap-1">{rows}</div>;
  return (
    <ToolCallGroup
      steps={part.tools.map(({ tool }) => ({ toolName: nameOf(tool.title).name, args: {}, status: 'idle' as const, hasResult: true }))}
    >
      {rows}
    </ToolCallGroup>
  );
}

function PermissionCard({ item }: { item: Of<'permission'> }) {
  const strong = (option: PermissionOption) => option.kind === 'allow_once';
  return (
    <Notice variant={item.answered === null ? 'warning' : 'note'} title={item.heading}>
      {item.answered === null ? (
        <div className="flex flex-wrap items-center gap-2 pt-2">
          {item.options.map(option => (
            <Button
              key={option.id}
              type="button"
              size="sm"
              variant={strong(option) ? 'primary' : 'default'}
              onClick={() => post({ type: 'permission', index: item.index, option: option.id })}
            >
              {option.name}
            </Button>
          ))}
        </div>
      ) : (
        <Notice.Message>{item.answered}</Notice.Message>
      )}
    </Notice>
  );
}

const OBJECT_NAME = ['assistant-object-name'];
const WORD = '[\\p{L}\\p{N}_]';

/** A node of the answer's Markdown tree, as far as marking names needs to know it. */
interface Node {
  type: string;
  value?: string;
  children?: Node[];
  data?: unknown;
}

function objectName(label: string, at: number): Node {
  return {
    type: 'objectName',
    data: { hName: 'button', hProperties: { type: 'button', className: OBJECT_NAME, 'data-object': String(at) } },
    children: [{ type: 'text', value: label }],
  };
}

/**
 * A Markdown plugin that makes buttons of the names the app listed for an answer, where they
 * stand as whole words in its text. Only the app's list decides: nothing the answer writes
 * (a link, markup) becomes one, and the app checks each click against the same list.
 */
function objectNames(labels: string[]) {
  const listed = labels.filter(label => label.length > 0);
  if (listed.length === 0) return null;
  const escaped = listed.map(label => label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  let names: RegExp;
  try {
    names = new RegExp(`(^|[^\\p{L}\\p{N}_])(${escaped.join('|')})(?!${WORD})`, 'gu');
  } catch {
    return null;
  }
  const split = (value: string): Node[] | null => {
    const out: Node[] = [];
    let from = 0;
    names.lastIndex = 0;
    for (let found = names.exec(value); found; found = names.exec(value)) {
      const name = found[2] ?? '';
      const start = found.index + (found[1] ?? '').length;
      if (start > from) out.push({ type: 'text', value: value.slice(from, start) });
      out.push(objectName(name, labels.indexOf(name)));
      from = start + name.length;
    }
    if (out.length === 0) return null;
    if (from < value.length) out.push({ type: 'text', value: value.slice(from) });
    return out;
  };
  const mark = (node: Node) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return;
    node.children = node.children.flatMap(child => {
      if (child.type === 'text' && child.value) return split(child.value) ?? [child];
      // A name written as code is still the name.
      if (child.type === 'inlineCode' && child.value && labels.includes(child.value)) return [objectName(child.value, labels.indexOf(child.value))];
      mark(child);
      return [child];
    });
  };
  return () => (tree: Node) => mark(tree);
}

/** An answer's text. The names of things in the design that the app listed select them when clicked. */
function Answer({ part, streaming }: { part: Extract<Part, { kind: 'text' }>; streaming: boolean }) {
  const listed = part.objects.join('\n');
  const plugins = useMemo(() => {
    const plugin = objectNames(listed ? listed.split('\n') : []);
    return plugin ? [plugin] : undefined;
  }, [listed]);
  function onClick(event: MouseEvent<HTMLDivElement>) {
    const name = event.target instanceof Element ? event.target.closest('[data-object]') : null;
    if (!name) return;
    post({ type: 'object', index: part.index, ref: Number(name.getAttribute('data-object')), label: name.textContent ?? '' });
  }
  if (!plugins) return <MarkdownRenderer streaming={streaming}>{part.text}</MarkdownRenderer>;
  return (
    <div className="min-w-0" onClick={onClick}>
      <MarkdownRenderer streaming={streaming} remarkPlugins={plugins}>
        {part.text}
      </MarkdownRenderer>
    </div>
  );
}

function Response({ turn, busy, strings }: { turn: Turn; busy: boolean; strings: Strings }) {
  const lastText = turn.parts.reduce((found, part, index) => (part.kind === 'text' && !part.thought ? index : found), -1);
  return (
    <div className="flex min-w-0 flex-col gap-4">
      {turn.parts.map((part, index) => {
        const key = `${turn.id}-${index}`;
        switch (part.kind) {
          case 'text':
            if (part.thought) {
              return (
                <Txt key={key} variant="caption" tone="muted" className="italic">
                  {part.text}
                </Txt>
              );
            }
            return (
              <div key={key} className="flex min-w-0 flex-col gap-2">
                <Answer part={part} streaming={busy && index === turn.parts.length - 1} />
                {index === lastText && !busy && (
                  <div>
                    <CopyButton content={part.text} />
                  </div>
                )}
              </div>
            );
          case 'tools':
            return <Tools key={key} part={part} strings={strings} />;
          case 'permission':
            return <PermissionCard key={key} item={part.item} />;
          case 'approval':
            return (
              <Notice key={key} variant={part.item.answer === null ? 'warning' : 'note'} title={part.item.title}>
                {part.item.answer === null ? (
                  <div className="pt-2">
                    <ApprovalChoices approval={part.item} strings={strings} />
                  </div>
                ) : (
                  <Notice.Message>{part.item.answer ? strings.allowed : strings.denied}</Notice.Message>
                )}
              </Notice>
            );
          case 'changes':
            return (
              <div key={key} className="flex flex-wrap items-center gap-2">
                <Txt variant="caption">
                  {part.item.label}
                  {part.item.undone ? ` ${strings.undone}` : ''}
                </Txt>
                {part.item.undoable && !busy && (
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    icon={<Undo2 aria-hidden />}
                    onClick={() => post({ type: 'undo', index: part.item.index })}
                  >
                    {strings.undo}
                  </Button>
                )}
              </div>
            );
          case 'notice':
            return (
              <Notice key={key} variant={part.item.error ? 'destructive' : 'note'}>
                <Notice.Message>{part.item.text}</Notice.Message>
              </Notice>
            );
          case 'skill':
            return (
              <div key={key} className="flex min-w-0 flex-col gap-2">
                <Txt variant="caption">{part.item.line}</Txt>
                {part.item.warnings.map((warning, at) => (
                  <Notice key={at} variant="warning">
                    <Notice.Message>{warning}</Notice.Message>
                  </Notice>
                ))}
                {part.item.terminal && <Txt variant="caption">{part.item.terminal}</Txt>}
                {part.item.revalidate && (
                  <div>
                    <Button type="button" size="sm" onClick={() => post({ type: 'revalidate' })}>
                      {strings.revalidate}
                    </Button>
                  </div>
                )}
              </div>
            );
        }
      })}
      {busy && turn.parts.length === 0 && (
        <div role="status" aria-label={strings.working}>
          <PendingIndicator className="h-6" />
        </div>
      )}
    </div>
  );
}

/** Something that goes with a message; with `onRemove`, a button that takes it off the next one. */
function Chip({ icon, label, title, remove, onRemove }: { icon?: ReactNode; label: string; title?: string; remove?: string; onRemove?: () => void }) {
  return (
    <span className="flex min-w-0 items-center gap-1" title={title || undefined}>
      <Badge variant="neutral" emphasis="subtle" size="sm" icon={icon}>
        <span className="truncate">{label}</span>
      </Badge>
      {onRemove && (
        <Button type="button" size="icon-sm" variant="ghost" aria-label={`${remove ?? ''} ${label}`.trim()} onClick={onRemove}>
          <X />
        </Button>
      )}
    </span>
  );
}

/** What a message was sent with, a row for each kind: its count, a chip for each thing, and how many more. */
function ContextChips({
  context,
  end = false,
  remove,
  onRemove,
}: {
  context: ContextGroups;
  end?: boolean;
  remove?: string;
  onRemove?: (chip: ContextChip) => void;
}) {
  if (context.groups.length === 0 && !context.omitted) return null;
  return (
    <div className={cn('flex min-w-0 flex-col gap-1', end && 'items-end')}>
      {context.groups.map(group => (
        <div key={group.heading} className={cn('flex min-w-0 flex-wrap items-center gap-1', end && 'justify-end')}>
          <Txt variant="caption">{group.heading}</Txt>
          {group.items.map(chip => (
            <Chip key={chip.index} label={chip.label} title={chip.detail} remove={remove} onRemove={onRemove && (() => onRemove(chip))} />
          ))}
          {group.more && <Txt variant="caption">{group.more}</Txt>}
        </div>
      ))}
      {context.omitted && <Txt variant="caption">{context.omitted}</Txt>}
    </div>
  );
}

/** "plate.step" as "STEP", for a file card's second line. */
function typeOf(name: string) {
  const at = name.lastIndexOf('.');
  return at > 0 && at < name.length - 1 ? name.slice(at + 1).toUpperCase() : '';
}

/** A sent message's pictures as thumbnails (one opens in the app's viewer) and its files as Mastra's cards. */
function SentFiles({ prompt, strings }: { prompt: Of<'user'>; strings: Strings }) {
  const naming = (picture: PictureRef) => picture.note || strings.viewAttached;
  return (
    <>
      {prompt.pictures.length > 0 && (
        <div className="flex max-w-full min-w-0 flex-wrap justify-end gap-2">
          {prompt.pictures.map((picture, at) => (
            <button
              key={at}
              type="button"
              title={naming(picture)}
              aria-label={`${strings.openPicture}: ${naming(picture)}`}
              className="cursor-pointer overflow-hidden rounded-xl border border-border bg-fill hover:border-border-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-border-focus motion-safe:transition-colors"
              onClick={() => post({ type: 'picture', index: prompt.index, at })}
            >
              <img src={pictureUrl(picture.key)} alt={naming(picture)} className="block h-24 w-auto max-w-44 object-cover" draggable={false} />
            </button>
          ))}
        </div>
      )}
      {prompt.files.map((name, at) => (
        <AttachmentCard key={at} name={name} typeLabel={typeOf(name)} icon={<FileText aria-hidden />} />
      ))}
    </>
  );
}

/**
 * The messages waiting behind the running one, or left waiting by Stop, docked over the composer
 * so they stay in view. Each can be reworded or taken out; Send (or Send now, which stops the
 * running step) sends them all as one message.
 */
function Queue({ state }: { state: AssistantState }) {
  const { strings } = state;
  const [draft, setDraft] = useState<{ index: number; was: string; text: string } | null>(null);
  if (state.pending.length === 0) return null;
  const busy = state.state === 'busy';
  // The queue moved on under an edit: the edit is over.
  const editing = draft && state.pending[draft.index]?.text === draft.was ? draft : null;
  const label = state.stopped && !busy ? strings.queuedStopped : strings.queued;
  function save() {
    if (!editing || !editing.text.trim()) return;
    post({ type: 'edit_pending', index: editing.index, was: editing.was, text: editing.text });
    setDraft(null);
  }
  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Escape') setDraft(null);
    else if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) save();
  }
  const row = (message: QueuedMessage, index: number) =>
    editing?.index === index ? (
      <div key={index} className="flex min-w-0 flex-col items-end gap-2 py-1">
        <Textarea
          autoFocus
          size="md"
          aria-label={strings.edit}
          value={editing.text}
          onChange={event => setDraft({ ...editing, text: event.target.value })}
          onKeyDown={onKeyDown}
        />
        <div className="flex items-center gap-2">
          <Button type="button" size="sm" variant="primary" disabled={!editing.text.trim()} onClick={save}>
            {strings.save}
          </Button>
          <Button type="button" size="sm" onClick={() => setDraft(null)}>
            {strings.cancel}
          </Button>
        </div>
      </div>
    ) : (
      <div key={index} className="flex min-w-0 items-start gap-1">
        <div className="flex min-w-0 flex-1 flex-col gap-1 py-1">
          {message.text && (
            <Txt variant="body-sm" className="line-clamp-2 break-words whitespace-pre-wrap" title={message.text}>
              {message.text}
            </Txt>
          )}
          {message.image && <Txt variant="caption">{strings.viewAttached}</Txt>}
          <ContextChips context={message.context} />
          {message.files.length > 0 && <Txt variant="caption">{message.files.join(', ')}</Txt>}
        </div>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={strings.edit}
          tooltip={strings.edit}
          onClick={() => setDraft({ index, was: message.text, text: message.text })}
        >
          <Pencil />
        </Button>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={strings.remove}
          tooltip={strings.remove}
          onClick={() => post({ type: 'remove_pending', index, text: message.text })}
        >
          <X />
        </Button>
      </div>
    );
  return (
    <TaskListContainer aria-label={label} className="flex max-h-64 min-w-0 flex-col gap-1 overflow-y-auto px-3 py-2.5">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <Txt variant="caption" className="min-w-0">
          {state.pending.length > 1 ? `${label} · ${state.pending.length}` : label}
        </Txt>
        {!editing &&
          (busy ? (
            <Button type="button" size="sm" tooltip={strings.sendNowTip} onClick={() => post({ type: 'send_now' })}>
              {strings.sendNow}
            </Button>
          ) : (
            <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'send_now' })}>
              {strings.send}
            </Button>
          ))}
      </div>
      {state.pending.map(row)}
    </TaskListContainer>
  );
}

/**
 * The agent's model, searched by typing; an agent that takes a model it did not list is sent what
 * was typed. The list is the agent's own models, or its model option when it describes itself by
 * options. Mastra's trigger is as wide as its row by default; `w-auto` keeps it to its text, so it
 * takes no room from the controls beside it and is the one that is cut short in a narrow panel.
 */
function ModelPicker({ state }: { state: AssistantState }) {
  const { strings } = state;
  const [typed, setTyped] = useState('');
  const option = state.config?.find(entry => entry.category === 'model');
  const listed: ComboboxOption[] = option
    ? option.options.map(choice => ({
        value: choice.value,
        label: choice.name || choice.value,
        description: choice.group ?? undefined,
        keywords: [choice.value, choice.group ?? ''],
      }))
    : state.models.map(model => ({ value: model.id, label: model.name || model.id, description: model.group ?? undefined, keywords: [model.id, model.group ?? ''] }));
  const current = option ? option.currentValue : state.model;
  if (listed.length === 0 && !(option && state.customModels)) return null;
  const wanted = typed.trim();
  const known = listed.some(entry => entry.value === wanted || entry.label === wanted);
  // Our own row for a typed model, in the app's language.
  const options = state.customModels && wanted && !known ? [{ value: wanted, label: strings.modelCustom.replace('{model}', wanted) }, ...listed] : listed;
  function choose(value: string) {
    if (value === current) return;
    if (!listed.some(entry => entry.value === value)) post({ type: 'custom_model', text: value });
    else if (option) post({ type: 'config', id: option.id, value });
    else post({ type: 'model', id: value });
  }
  return (
    <Combobox
      options={options}
      value={current}
      onValueChange={choose}
      onInputValueChange={setTyped}
      placeholder={current || strings.model}
      searchPlaceholder={strings.modelSearch}
      emptyText={strings.modelNoMatch}
      variant="ghost"
      size="sm"
      align="end"
      popupWidth="md"
      disabled={state.configPending}
      aria-label={strings.model}
      className="w-auto max-w-40 min-w-0 shrink"
    />
  );
}

function AssistantComposer({ state }: { state: AssistantState }) {
  const { strings } = state;
  const [text, setText] = useState('');
  const [sent, setSent] = useState(0);
  const input = useRef<HTMLTextAreaElement>(null);
  const busy = state.state === 'busy';
  const reading = state.loading.length > 0;
  const canSend = !reading && (text.trim().length > 0 || state.attachments.length > 0);
  const selection = state.selected > 0 ? `${strings.attachSelection} (${state.selected})` : strings.attachSelection;
  const modes = [
    { value: 'ask' as const, label: strings.modeAsk },
    { value: 'edits' as const, label: strings.modeEdits },
    { value: 'all' as const, label: strings.modeAll },
  ];
  const mode = modes.find(option => option.value === state.permissions)?.label ?? strings.modeEdits;
  const skill = state.skills.find(option => option.id === state.skill);
  const picked = state.picked.groups.length > 0 || state.picked.omitted !== null;
  const selected = state.selection.groups.length > 0 || state.selection.omitted !== null;
  const chips = state.attachments.length + state.loading.length > 0 || picked || selected || (state.attachView && state.images);

  function submit(now = false) {
    if (!canSend) return;
    post(now ? { type: 'send', text, now } : { type: 'send', text });
    setText('');
    setSent(count => count + 1);
    input.current?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    const composing = event.nativeEvent.isComposing || event.keyCode === 229;
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !composing) {
      // Send now: this message and what is queued go next, the running step stops for them.
      event.preventDefault();
      if (canSend) submit(true);
      else if (!reading && state.pending.length > 0) post({ type: 'send_now' });
    } else if (event.key === 'Enter' && !event.shiftKey && !composing) {
      event.preventDefault();
      submit();
    } else if (event.key === 'Escape' && busy) {
      post({ type: 'stop' });
    }
  }

  return (
    <Composer
      aria-label={strings.composer}
      onSubmit={event => {
        event.preventDefault();
        submit();
      }}
    >
      <ComposerRing busy={busy}>
        <ComposerBox sendingPulseKey={sent}>
          {chips && (
            <ComposerAttachments aria-label={strings.withNext} className="flex flex-wrap gap-2 px-3 pt-3">
              {state.attachments.map((file, index) => (
                <Chip
                  key={`file-${index}`}
                  icon={
                    file.picture ? (
                      <img src={pictureUrl(file.picture)} alt="" className="size-4 rounded-sm object-cover" draggable={false} />
                    ) : (
                      <FileText aria-hidden />
                    )
                  }
                  label={file.name}
                  remove={strings.removeChip}
                  onRemove={() => post({ type: 'remove_file', index })}
                />
              ))}
              {state.loading.map((name, index) => (
                <Chip key={`loading-${index}`} icon={<FileText aria-hidden />} label={`${name}…`} />
              ))}
              {state.attachView && state.images && (
                <Chip
                  icon={<Camera aria-hidden />}
                  label={strings.attachView}
                  remove={strings.removeChip}
                  onRemove={() => post({ type: 'attach_view', on: false })}
                />
              )}
              {/* What is selected in the design, by name: the agent can read it through the app's
                  tools. With Attach selection on it is also written into the message, and the
                  cross takes that off again. */}
              {selected && (
                <div className="flex w-full min-w-0 items-start gap-1.5">
                  <span role="img" aria-label={strings.selectedNow} title={strings.selectedNow} className="flex h-6 shrink-0 items-center text-muted-foreground">
                    <MousePointerClick aria-hidden className="size-3.5" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <ContextChips context={state.selection} />
                  </div>
                  {state.attachSelection && (
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`${strings.removeChip} ${strings.attachSelection}`}
                      tooltip={strings.attachSelection}
                      onClick={() => post({ type: 'attach_selection', on: false })}
                    >
                      <X />
                    </Button>
                  )}
                </div>
              )}
              {picked && (
                <div className="w-full min-w-0">
                  <ContextChips
                    context={state.picked}
                    remove={strings.removeChip}
                    onRemove={chip => post({ type: 'remove_picked', index: chip.index, label: chip.label })}
                  />
                </div>
              )}
            </ComposerAttachments>
          )}
          <ComposerInput
            ref={input}
            aria-label={strings.messageBox}
            placeholder={busy ? strings.placeholderBusy : strings.placeholder}
            value={text}
            onChange={event => setText(event.target.value)}
            onKeyDown={onKeyDown}
          />
          <ComposerActions>
            <ComposerActionsStart className="gap-1">
              <DropdownMenu>
                <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={strings.add} tooltip={strings.add}>
                  <Plus />
                </DropdownMenu.Trigger>
                <DropdownMenu.Content side="top" align="start">
                  <DropdownMenu.Item onClick={() => post({ type: 'add_file' })}>
                    <FileText />
                    {strings.addFile}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item onClick={() => post({ type: 'paste_picture' })}>
                    <Image />
                    {strings.pastePicture}
                  </DropdownMenu.Item>
                  <DropdownMenu.Separator />
                  <DropdownMenu.CheckboxItem
                    checked={state.attachView && state.images}
                    disabled={!state.images}
                    onCheckedChange={on => post({ type: 'attach_view', on: Boolean(on) })}
                  >
                    {strings.attachView}
                  </DropdownMenu.CheckboxItem>
                  <DropdownMenu.CheckboxItem
                    checked={state.attachSelection}
                    onCheckedChange={on => post({ type: 'attach_selection', on: Boolean(on) })}
                  >
                    {selection}
                  </DropdownMenu.CheckboxItem>
                  <DropdownMenu.Item onClick={() => post({ type: 'pick_context' })}>
                    <Target />
                    {strings.pickContext}
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu>
              <DropdownMenu>
                <DropdownMenu.Trigger variant="ghost" size="sm" aria-label={mode}>
                  {mode}
                </DropdownMenu.Trigger>
                <DropdownMenu.Content side="top" align="start">
                  <DropdownMenu.RadioOptions options={modes} value={state.permissions} onValueChange={next => post({ type: 'permissions', mode: next })} />
                </DropdownMenu.Content>
              </DropdownMenu>
              {state.skills.length > 0 && (
                <DropdownMenu>
                  <DropdownMenu.Trigger variant="ghost" size="sm" disabled={busy} aria-label={skill?.title ?? state.skill}>
                    {skill?.title ?? state.skill}
                  </DropdownMenu.Trigger>
                  <DropdownMenu.Content side="top" align="start">
                    <DropdownMenu.RadioOptions
                      options={state.skills.map(option => ({ value: option.id, label: option.title }))}
                      value={state.skill}
                      onValueChange={next => post({ type: 'skill', id: next })}
                    />
                  </DropdownMenu.Content>
                </DropdownMenu>
              )}
            </ComposerActionsStart>
            <ComposerActionsEnd>
              <ModelPicker state={state} />
              {busy && (
                <Button type="button" size="icon-sm" className="shrink-0" aria-label={strings.stop} tooltip={strings.stop} onClick={() => post({ type: 'stop' })}>
                  <Square />
                </Button>
              )}
              <Button type="submit" size="icon-sm" variant="primary" className="shrink-0" aria-label={strings.send} disabled={!canSend}>
                <ArrowUp />
              </Button>
            </ComposerActionsEnd>
          </ComposerActions>
        </ComposerBox>
      </ComposerRing>
    </Composer>
  );
}

/** The state in a word, as the panel's badge says it. */
function stateBadge(state: AssistantState) {
  const { strings } = state;
  if (state.waiting > 0) return { label: strings.waiting, variant: 'warning' as const };
  switch (state.state) {
    case 'ready':
      return { label: strings.ready, variant: 'success' as const };
    case 'busy':
      return { label: strings.working, variant: 'info' as const };
    case 'starting':
      return { label: strings.starting, variant: 'info' as const };
    case 'auth':
      return { label: strings.auth, variant: 'warning' as const };
    case 'failed':
      return { label: strings.failed, variant: 'destructive' as const };
    case 'exited':
      return { label: strings.exited, variant: 'destructive' as const };
    default:
      return { label: strings.off, variant: 'neutral' as const };
  }
}

/** The panel's own bar: the agent and what it runs with, its state, and the panel's controls. */
function AssistantBar({ state }: { state: AssistantState }) {
  const { strings } = state;
  const agent = state.agents.find(entry => entry.id === state.agent);
  const name = agent?.name ?? (state.detecting ? strings.detecting : strings.noneFound);
  const badge = stateBadge(state);
  // The agent's own settings; its model is picked beside the message box.
  const settings = (state.config ?? []).filter(option => option.category !== 'model');
  return (
    <ChatShell.Bar>
      <ChatShell.Column className="flex-row items-center justify-between gap-2 py-2">
        <div className="-m-0.5 flex h-8 min-w-0 flex-1 flex-wrap content-start items-center gap-x-1.5 gap-y-2 overflow-hidden p-0.5">
          <DropdownMenu>
            <DropdownMenu.Trigger variant="ghost" size="sm" aria-label={strings.switchAgent} tooltip={strings.switchAgent} className="max-w-full min-w-12">
              <span className="truncate">{name}</span>
              <ChevronDown />
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="start">
              {state.agents.length > 0 && (
                <DropdownMenu.RadioOptions
                  options={state.agents.map(entry => ({ value: entry.id, label: entry.name }))}
                  value={state.agent ?? ''}
                  onValueChange={id => post({ type: 'agent', id })}
                  closeOnSelect
                />
              )}
              {(state.modes.length > 0 || settings.length > 0) && <DropdownMenu.Separator />}
              {state.modes.length > 0 && (
                <DropdownMenu.RadioSub
                  label={state.modes.find(mode => mode.id === state.mode)?.name ?? state.mode}
                  options={state.modes.map(mode => ({ value: mode.id, label: mode.name }))}
                  value={state.mode}
                  onValueChange={id => post({ type: 'agent_mode', id })}
                  closeOnSelect
                />
              )}
              {settings.map(option => (
                <DropdownMenu.RadioSub
                  key={option.id}
                  label={option.name}
                  options={option.options.map(choice => ({ value: choice.value, label: choice.name }))}
                  value={option.currentValue}
                  onValueChange={value => post({ type: 'config', id: option.id, value })}
                  closeOnSelect
                />
              ))}
              <DropdownMenu.Separator />
              <DropdownMenu.Item onClick={() => post({ type: 'add_agent' })}>{strings.addAgent}</DropdownMenu.Item>
              <DropdownMenu.Item onClick={() => post({ type: 'settings' })}>{strings.settings}</DropdownMenu.Item>
              <DropdownMenu.Item onClick={() => post({ type: 'refresh' })}>{strings.refresh}</DropdownMenu.Item>
              {/* Stopping the agent is the agent's own matter, said in full here: the square by Send stops only the running message. */}
              {state.running && (
                <>
                  <DropdownMenu.Separator />
                  <DropdownMenu.Item onClick={() => post({ type: 'disconnect' })}>{strings.disconnect}</DropdownMenu.Item>
                </>
              )}
            </DropdownMenu.Content>
          </DropdownMenu>
          {/* It wraps out of sight when the name leaves it no room. */}
          <Badge variant={badge.variant} emphasis="subtle" size="sm" className="shrink-0 whitespace-nowrap">
            {badge.label}
          </Badge>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          {state.agent && !state.running && (
            <Button type="button" size="sm" variant="ghost" onClick={() => post({ type: 'connect' })}>
              {state.state === 'failed' ? strings.retry : strings.connect}
            </Button>
          )}
          <Button type="button" size="icon-sm" variant="ghost" aria-label={strings.newChat} tooltip={strings.newChat} onClick={() => post({ type: 'new_chat' })}>
            <SquarePen />
          </Button>
          <DropdownMenu>
            <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={strings.title}>
              <Ellipsis />
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="end">
              <DropdownMenu.Item onClick={() => post({ type: 'side', left: !state.left })}>
                {state.left ? strings.dockRight : strings.dockLeft}
              </DropdownMenu.Item>
              <DropdownMenu.Item onClick={() => post({ type: 'minimize' })}>{strings.minimize}</DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu>
          <Button type="button" size="icon-sm" variant="ghost" aria-label={strings.close} tooltip={strings.close} onClick={() => post({ type: 'close' })}>
            <X />
          </Button>
        </div>
      </ChatShell.Column>
    </ChatShell.Bar>
  );
}

/** The agent asks for a sign-in: each method the way it is done, then Connect again. */
function SignIn({ state }: { state: AssistantState }) {
  const { strings } = state;
  const methods = state.auth ?? [];
  return (
    <Notice variant="warning" title={strings.authTitle}>
      <Notice.Message>{strings.authNeeded}</Notice.Message>
      <div className="flex min-w-0 flex-col gap-3 pt-2">
        {methods.length === 0 && <Txt variant="caption">{strings.authNone}</Txt>}
        {methods.map(method => (
          <div key={method.id} className="flex min-w-0 flex-col gap-1.5">
            <Txt variant="caption">{method.label}</Txt>
            {method.kind === 'agent' && (
              <div>
                <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'auth', id: method.id })}>
                  {method.action}
                </Button>
              </div>
            )}
            {method.kind === 'terminal' && method.command && (
              <>
                <Txt variant="caption">{strings.authTerminal}</Txt>
                <div className="flex min-w-0 items-start gap-2">
                  <ToolCallOutput text={method.command} />
                  <CopyButton content={method.command} />
                </div>
              </>
            )}
            {method.kind === 'env' && <Txt variant="caption">{method.note}</Txt>}
          </div>
        ))}
        <div>
          <Button type="button" size="sm" onClick={() => post({ type: 'connect' })}>
            {strings.authRetry}
          </Button>
        </div>
      </div>
    </Notice>
  );
}

/** Nothing said yet: what the Assistant is, or how to get an agent when there is none. */
function Welcome({ state }: { state: AssistantState }) {
  const { strings } = state;
  if (state.agents.length === 0 && !state.detecting) {
    return (
      <div className="flex min-w-0 flex-col gap-4">
        <EmptyState
          iconSlot={<MessageSquare />}
          titleSlot={strings.setupTitle}
          descriptionSlot={strings.setupBody}
          actionSlot={
            <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'settings' })}>
              {strings.openSettings}
            </Button>
          }
        />
        {state.missing.length > 0 && (
          <div className="flex min-w-0 flex-col gap-3">
            <Txt variant="caption">{strings.notInstalled}</Txt>
            {state.missing.map(agent => (
              <div key={agent.name} className="flex min-w-0 flex-col gap-1">
                <Txt variant="caption">{agent.name}</Txt>
                <div className="flex min-w-0 items-start gap-2">
                  <ToolCallOutput text={agent.install} />
                  <CopyButton content={agent.install} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }
  return (
    <EmptyState
      iconSlot={<MessageSquare />}
      titleSlot={strings.title}
      descriptionSlot={
        <>
          {strings.intro}
          <br />
          <br />
          {strings.privacy}
        </>
      }
    />
  );
}

export function App({ state }: { state: AssistantState | null }) {
  const turns = useMemo(() => turnsOf(state?.items ?? []), [state?.items]);
  if (!state) return null;
  const { strings } = state;
  const busy = state.state === 'busy';
  const plan = turns.at(-1)?.plan;
  const tasks: TaskListItem[] = (plan ?? []).map((step, index) => ({
    id: String(index),
    content: step.text,
    activeForm: step.text,
    status: step.status === 'completed' || step.status === 'in_progress' ? step.status : 'pending',
  }));
  return (
    <div className="h-dvh overflow-hidden bg-background" style={{ borderRadius: state.radius }}>
      <ChatShell className="h-full" scroller={{ autoScroll: true, defaultScrollPosition: 'end' }}>
        <AssistantBar state={state} />
        <ChatShell.Stage>
          <ChatShell.Viewport>
            <ChatShell.Content>
              <ChatShell.Column className="gap-8 py-6">
                {state.notes.length > 0 && (
                  <div className="flex min-w-0 flex-col gap-1">
                    {state.notes.map((note, index) => (
                      <Txt key={index} variant="caption">
                        {note}
                      </Txt>
                    ))}
                  </div>
                )}
                {state.confirmNew && (
                  <Notice variant="warning" title={strings.newChatConfirm}>
                    <div className="flex flex-wrap items-center gap-2 pt-2">
                      <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'confirm_new', yes: true })}>
                        {strings.newChat}
                      </Button>
                      <Button type="button" size="sm" onClick={() => post({ type: 'confirm_new', yes: false })}>
                        {strings.cancel}
                      </Button>
                    </div>
                  </Notice>
                )}
                {state.auth && <SignIn state={state} />}
                {turns.length === 0 && state.pending.length === 0 && !state.auth && <Welcome state={state} />}
                {turns.map((turn, index) => {
                  const live = busy && index === turns.length - 1;
                  return (
                    <ChatShell.Turn key={turn.id} role="region" aria-label={strings.turnLabel.replace('{n}', String(index + 1))} opensTurn holdsRoom={live} className="gap-6">
                      {turn.prompt && (
                        <MessageScrollerItem
                          messageId={turn.id}
                          scrollAnchor
                          className="ml-auto flex max-w-[85%] min-w-0 flex-col items-end gap-2"
                        >
                          {turn.prompt.text && (
                            <Message from="user" className="max-w-full whitespace-pre-wrap">
                              {turn.prompt.text}
                            </Message>
                          )}
                          <SentFiles prompt={turn.prompt} strings={strings} />
                          {/* The view's picture is taken when the agent starts on the message. */}
                          {turn.prompt.image && turn.prompt.pictures.length === 0 && <Txt variant="caption">{strings.viewAttached}</Txt>}
                          <ContextChips context={turn.prompt.context} end />
                        </MessageScrollerItem>
                      )}
                      <MessageScrollerItem messageId={`${turn.id}-reply`} className="flex min-w-0 flex-col gap-3">
                        <Response turn={turn} busy={live} strings={strings} />
                      </MessageScrollerItem>
                    </ChatShell.Turn>
                  );
                })}
              </ChatShell.Column>
            </ChatShell.Content>
            <ChatShell.Dock>
              <ChatShell.ScrollButton aria-label={strings.latest} />
              <ChatShell.Column className="gap-2">
                {state.dragging && (
                  <Notice variant="info" className="bg-card">
                    <Notice.Message>{strings.dropHint}</Notice.Message>
                  </Notice>
                )}
                {tasks.length > 0 && <TaskList defaultOpen={busy} hideWhenComplete={false} scrollActiveIntoView={false} tasks={tasks} />}
                <Queue state={state} />
                <ComposerStatus role="status" aria-live="polite">
                  {busy && state.turn && (
                    <span className="flex items-center gap-1.5 whitespace-nowrap">
                      <PendingIndicator />
                      {state.turn}
                    </span>
                  )}
                </ComposerStatus>
                <AssistantComposer state={state} />
              </ChatShell.Column>
            </ChatShell.Dock>
          </ChatShell.Viewport>
        </ChatShell.Stage>
      </ChatShell>
    </div>
  );
}
