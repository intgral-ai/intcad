// The Assistant's conversation and composer, drawn with Mastra's playground-ui components and
// fed with the desktop app's own transcript. Everything visual is imported from the Mastra
// checkout; this file only maps our items onto those components.
import {
  ArrowUp,
  Camera,
  ChevronDown,
  ChevronRight,
  ClipboardCheck,
  Ellipsis,
  FileText,
  Image,
  MessageSquare,
  MousePointerClick,
  Pencil,
  Plug,
  Plus,
  ShieldCheck,
  Square,
  SquarePen,
  Target,
  Undo2,
  X,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import type { AssistantState, Connect, ContextChip, ContextGroups, Item, MissingAgent, PermissionOption, PictureRef, QueuedMessage, Strings, TurnProblem } from './host';
import { pictureUrl, post } from './host';
import { ModelPicker as MastraModelPicker } from '@/domains/llm/model-picker';
import type { ModelPickerProvider } from '@/domains/llm/model-picker-types';
import { ProviderLogoMonogram } from '@/domains/llm/provider-logo-monogram';
import { Activity, ActivityContent, ActivityHeadline, ActivityTrigger } from '@/ds/components/ai/activity';
import type { TaskListItem } from '@/ds/components/ai/task-list';
import { TaskList, TaskListContainer } from '@/ds/components/ai/task-list';
import { ToolCallGroup, ToolCallOutput, presentTool } from '@/ds/components/ai/tool-call';
import { AttachmentCard } from '@/ds/components/AttachmentCard';
import { Badge } from '@/ds/components/Badge';
import { Button } from '@/ds/components/Button';
import { ButtonsGroup } from '@/ds/components/ButtonsGroup';
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
import { Dialog, DialogAction, DialogBody, DialogCancel, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/ds/components/Dialog';
import { DropdownMenu } from '@/ds/components/DropdownMenu';
import { EmptyState } from '@/ds/components/EmptyState';
import { Field, FieldLabel } from '@/ds/components/Field';
import { Input } from '@/ds/components/Input';
import { MarkdownRenderer } from '@/ds/components/MarkdownRenderer';
import { Message } from '@/ds/components/Message';
import { MessageScrollerItem } from '@/ds/components/MessageScroller';
import { Notice } from '@/ds/components/Notice';
import { PendingIndicator } from '@/ds/components/PendingIndicator';
import { Status } from '@/ds/components/StatusIndicators/status';
import { Textarea } from '@/ds/components/Textarea';
import { ThinkingLevelPicker } from '@/ds/components/ThinkingLevel';
import type { ThinkingLevelOption } from '@/ds/components/ThinkingLevel';
import { Txt } from '@/ds/components/Txt';
import { SettingsContainer } from '@/ds/new/settings/settings-container';
import { SettingsGroup, SettingsHeader, SettingsTitle } from '@/ds/new/settings/settings-header';
import { cn } from '@/lib/utils';
import { compareModelReleaseDates, formatModelName } from '@/utils/model';

type Of<K extends Item['kind']> = Extract<Item, { kind: K }>;
type Tool = Of<'tool'>;
type Approval = Of<'approval'>;

type Part =
  | { kind: 'text'; index: number; text: string; thought: boolean; objects: string[] }
  | { kind: 'tools'; tools: { tool: Tool; approval?: Approval }[] }
  | { kind: 'permission'; item: Of<'permission'> }
  | { kind: 'approval'; item: Approval }
  | { kind: 'changes'; item: Of<'changes'> }
  | { kind: 'review'; item: Of<'review'> }
  | { kind: 'notice'; item: Of<'notice'> }
  | { kind: 'check'; item: Of<'check'> }
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

/** The agent's turn on a copy: what it made, changed and removed, and Accept or Reject. */
function ReviewCard({ item, strings }: { item: Of<'review'>; strings: Strings }) {
  const bodies = (label: string, names: string[]) =>
    names.length > 0 && (
      <Txt variant="caption" tone="muted">
        {label.replace('{bodies}', names.join(', '))}
      </Txt>
    );
  return (
    <div className="flex min-w-0 flex-col gap-2 rounded-lg border border-border1 p-3">
      <Txt variant="ui-md" weight="medium">{strings.reviewTitle}</Txt>
      {item.steps.map((step, at) => (
        <Txt key={at} variant="caption" tone="muted">• {step}</Txt>
      ))}
      {item.more > 0 && <Txt variant="caption" tone="muted">{strings.reviewMore.replace('{n}', String(item.more))}</Txt>}
      {bodies(strings.reviewMade, item.made.map(b => b.name))}
      {bodies(strings.reviewChanged, item.changed.map(b => b.name))}
      {bodies(strings.reviewRemoved, item.removed.map(b => b.name))}
      {item.outcome === 'Waiting' ? (
        <div className="flex items-center gap-2">
          <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'review', id: item.review, accept: true })}>
            {strings.reviewAccept}
          </Button>
          <Button type="button" size="sm" onClick={() => post({ type: 'review', id: item.review, accept: false })}>
            {strings.reviewReject}
          </Button>
        </div>
      ) : (
        <Txt variant="caption" tone="muted">{item.outcome === 'Accepted' ? strings.reviewAccepted : strings.reviewRejected}</Txt>
      )}
    </div>
  );
}

/** The DFM check card: counts, "not checked", and one expandable row per finding. */
function CheckCardView({ item, strings }: { item: Of<'check'>; strings: Strings }) {
  const card = item.card;
  const [open, setOpen] = useState<string | null>(null);
  // A queued run finishes on the host's worker; ask the host for the report until it does.
  useEffect(() => {
    if (item.status !== 'running' && item.status !== 'pending') return;
    const timer = window.setInterval(() => post({ type: 'dfm_poll' }), 500);
    return () => window.clearInterval(timer);
  }, [item.status]);
  if (item.status === 'running' || item.status === 'pending') {
    return (
      <Notice variant="note" title={strings.dfmCheckTitle}>
        <Notice.Message>{strings.dfmRunning}</Notice.Message>
      </Notice>
    );
  }
  if (item.status === 'error' || !card || card.status === 'error') {
    return (
      <Notice variant="destructive" title={strings.dfmCheckTitle}>
        <Notice.Message>{card?.error ?? strings.dfmError}</Notice.Message>
      </Notice>
    );
  }
  const state = card.status === 'cancelled' ? strings.dfmCancelled : strings.dfmDone;
  const stale = card.freshness !== 'current';
  return (
    <div className="flex min-w-0 flex-col gap-2 rounded-lg border border-border1 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Txt variant="ui-md" weight="medium">{strings.dfmCheckTitle}</Txt>
        <Badge>{state}</Badge>
        {stale && <Badge variant="warning">{strings.dfmStale}</Badge>}
        <Button type="button" size="sm" variant="ghost" onClick={() => post({ type: 'dfm_check' })}>
          {strings.dfmRefresh}
        </Button>
      </div>
      <Txt variant="caption" tone="muted">
        {card.counts.failed} {strings.dfmFailed} · {card.counts.advisory} {strings.dfmAdvisory} · {card.counts.checked} {strings.dfmChecked} ·{' '}
        {card.counts.unsupported} {strings.dfmNotCovered}
      </Txt>
      {card.notChecked && <Txt variant="caption" tone="muted">{card.notChecked}</Txt>}
      <div className="flex flex-wrap items-center gap-1">
        {(['all', 'selected', 'off'] as const).map(mode => (
          <Button
            key={mode}
            type="button"
            size="sm"
            variant={item.display === mode ? 'default' : 'ghost'}
            onClick={() => post({ type: 'dfm_display', mode })}
          >
            {mode === 'all' ? strings.dfmAll : mode === 'selected' ? strings.dfmSelected : strings.dfmOff}
          </Button>
        ))}
      </div>
      <div className="flex min-w-0 flex-col">
        {card.findings.map(finding => {
          const shown = open === finding.id;
          const selected = item.selected?.runId === card.runId && item.selected.findingId === finding.id;
          return (
            <div key={finding.id} className="border-t border-border1 py-1">
              <button
                type="button"
                className="flex w-full min-w-0 items-center gap-2 text-left"
                aria-expanded={shown}
                onClick={() => {
                  setOpen(shown ? null : finding.id);
                  post({ type: 'finding', runId: card.runId, id: finding.id });
                }}
              >
                <Badge variant={finding.state === 'failed' ? 'destructive' : finding.state === 'advisory' ? 'warning' : 'neutral'}>
                  {finding.state}
                </Badge>
                <Txt variant="caption" className="min-w-0 truncate">
                  {finding.rules.join(', ')} · {finding.body}
                </Txt>
                {selected && <Txt variant="caption" tone="muted">{strings.dfmSelect}</Txt>}
              </button>
              {shown && (
                <div className="flex flex-col gap-1 pt-1">
                  <Txt variant="caption">
                    {finding.measured ? `${strings.dfmMeasured}: ${finding.measured.value} ${finding.measured.unit}` : strings.dfmNotCovered}
                  </Txt>
                  {finding.requirement ? (
                    <Txt variant="caption" tone="muted">
                      {strings.dfmRequirement}: {finding.requirement.value} {finding.requirement.unit} · {finding.requirement.layer} ·{' '}
                      {finding.requirement.source}
                      {finding.requirement.confirmed ? ` · ${finding.requirement.confirmed}` : ''}
                    </Txt>
                  ) : (
                    <Txt variant="caption" tone="muted">{finding.reason ?? strings.dfmNotCovered}</Txt>
                  )}
                  {finding.code && <Txt variant="caption" tone="muted">{finding.code}</Txt>}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Response({
  turn,
  busy,
  strings,
  ways,
}: {
  turn: Turn;
  busy: boolean;
  strings: Strings;
  /** The built-in agent's connections, for the turn whose failure can still be acted on: the last one. */
  ways?: { connect: Connect; onConnections: () => void };
}) {
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
          case 'review':
            return <ReviewCard key={key} item={part.item} strings={strings} />;
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
                {ways && part.item.problem && <ProblemWays problem={part.item.problem} busy={busy} strings={strings} {...ways} />}
              </Notice>
            );
          case 'check':
            return <CheckCardView key={key} item={part.item} strings={strings} />;
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

/** The models the agent lists and the one in use: its model option when it describes itself by options. */
function modelChoice(state: AssistantState) {
  const option = state.config?.find(entry => entry.category === 'model');
  return {
    option,
    current: option ? option.currentValue : state.model,
    ids: option ? option.options.map(choice => choice.value) : state.models.map(model => model.id),
  };
}

type ModelOption = ReturnType<typeof modelChoice>['option'];

/** Picking a model is setting the agent's model option, as develop's picker does it. */
function pickModel(option: ModelOption, id: string) {
  post(option ? { type: 'config', id: option.id, value: id } : { type: 'model', id });
}

/** One of the built-in agent's connections, whichever kind it is. */
type Connection = Connect['plans'][number] | Connect['keys'][number] | Connect['custom'][number];

/**
 * The connections there is something to talk through, in the order one is taken before another
 * with the same prefix: a plan that is signed in, a saved key no plan stands before, a custom
 * address. A plan whose sign-in has expired comes last: its models are still named after it
 * when nothing else gives them.
 */
function liveOf(connect: Connect): Connection[] {
  return [
    ...connect.plans.filter(plan => plan.state === 'in'),
    ...connect.keys.filter(key => key.saved && !key.shadowed),
    ...connect.custom,
    ...connect.plans.filter(plan => plan.state === 'expired'),
  ];
}

/**
 * The connection a model goes through: the one whose prefix, then `/`, its id starts with, and
 * the longest when several do. A prefix may itself hold a `/` (a custom address), so a model id
 * is never cut at a slash to find its provider.
 */
function connectionOf(connect: Connect, model: string) {
  let found: Connection | undefined;
  for (const entry of liveOf(connect)) {
    if (model.startsWith(`${entry.prefix}/`) && (!found || entry.prefix.length > found.prefix.length)) found = entry;
  }
  return found;
}

/**
 * Where a model stands in the picker: under the connection it goes through, by the rest of its
 * id. One that no connection gives stands under the first part of its id.
 */
function placeOf(connect: Connect, id: string) {
  const via = connectionOf(connect, id);
  const at = via ? via.prefix.length : id.indexOf('/');
  return at > 0 ? { group: id.slice(0, at), model: id.slice(at + 1), via } : { group: '', model: id, via };
}

/**
 * The mark beside a connection: the first letter of its name in a square. Not the provider's own
 * logo: this repository carries no other product's icons.
 */
function Mark({ name, size }: { name: string; size: number }) {
  return <ProviderLogoMonogram providerId={name} style={{ width: size, height: size }} />;
}

/** A connection's name for a narrow place: "ChatGPT Plus/Pro (Codex Subscription)" reads "ChatGPT Plus/Pro". */
const shortName = (name: string) => name.replace(/\s*[(\uFF08][^()\uFF08\uFF09]*[)\uFF09]$/, '') || name;

/** The words of this page for the effort levels an agent names; a level it names otherwise keeps the agent's own word. */
const EFFORT_WORDS = { off: 'effortOff', low: 'effortLow', medium: 'effortMedium', high: 'effortHigh', xhigh: 'effortXhigh', max: 'effortMax' } as const;

/** The agent's reasoning effort (its `thought_level` option), as Mastra's thinking-level parts take it. */
function effortOf(state: AssistantState) {
  const option = state.config?.find(entry => entry.category === 'thought_level');
  if (!option) return null;
  const { strings } = state;
  const options: ThinkingLevelOption[] = option.options.map(choice => {
    const known = EFFORT_WORDS[choice.value as keyof typeof EFFORT_WORDS];
    return {
      value: choice.value,
      label: (known && strings[known]) || choice.name,
      ...(choice.value === 'off' ? { emphasis: 'muted' as const } : choice.value === 'max' ? { emphasis: 'warning' as const } : {}),
    };
  });
  return { options, value: option.currentValue, label: strings.effort || option.name, set: (value: string) => post({ type: 'config', id: option.id, value }) };
}

/**
 * The built-in agent's model and its reasoning effort, at the composer's lower right, as Mastra
 * groups the two (its story "ThinkingLevel / Picker / Grouped With Model Picker"). The model is
 * in Mastra's own model picker: the models of every connection, a group to a connection, newest
 * first, searched across all of them, with the row that opens the connections, where another
 * provider is added. The effort is Mastra's thinking-level picker beside it.
 */
function ProviderPicker({ state, connect, onConnections }: { state: AssistantState; connect: Connect; onConnections: () => void }) {
  const { strings } = state;
  const { option, current, ids } = modelChoice(state);
  if (ids.length === 0) return null;
  const groups = new Map<string, ModelPickerProvider>();
  const names = new Map<string, string>();
  for (const id of ids) {
    const { group, model, via } = placeOf(connect, id);
    names.set(group, via?.name ?? group);
    const provider = groups.get(group) ?? { id: group, name: via ? shortName(via.name) : group, models: [] };
    provider.models.push({ id: model, releaseDate: connect.dates[id] });
    groups.set(group, provider);
  }
  // The connections in their own order; what no connection gives comes last.
  const order = liveOf(connect).map(entry => entry.prefix);
  const rank = (group: string) => (order.includes(group) ? order.indexOf(group) : order.length);
  const providers = [...groups.values()].sort((a, b) => rank(a.id) - rank(b.id));
  const at = current ? placeOf(connect, current) : null;
  const effort = effortOf(state);
  const mark = (group: string) => <Mark name={names.get(group) ?? group} size={14} />;
  const model = (
    <MastraModelPicker
      providers={providers}
      value={at ? { provider: at.group, model: at.model } : undefined}
      onValueChange={next => {
        const id = `${next.provider}/${next.model}`;
        if (id !== current) pickModel(option, id);
      }}
      renderProviderIcon={mark}
      addProvider={{ label: strings.connTitle, render: <button type="button" onClick={onConnections} /> }}
      busy={state.configPending}
      side="top"
      align="end"
    >
      <span className="sr-only">{strings.model}: </span>
      {at && (
        <span aria-hidden className="flex shrink-0">
          {mark(at.group)}
        </span>
      )}
      <span className="max-w-40 truncate">{at ? formatModelName(at.model) : strings.model}</span>
    </MastraModelPicker>
  );
  if (!effort) return model;
  return (
    <ButtonsGroup size="sm" aria-label={`${strings.model}, ${effort.label}`} className="min-w-0">
      {model}
      <ThinkingLevelPicker options={effort.options} value={effort.value} label={effort.label} onChange={effort.set} />
    </ButtonsGroup>
  );
}

/**
 * The agent's model, searched by typing; an agent that takes a model it did not list is sent what
 * was typed. The list is the agent's own models, or its model option when it describes itself by
 * options. Mastra's trigger is as wide as its row by default; `w-auto` keeps it to its text, so it
 * takes no room from the controls beside it and is the one that is cut short in a narrow panel.
 * The built-in agent's list is drawn by `ProviderPicker`.
 */
function ModelPicker({ state, onConnections }: { state: AssistantState; onConnections: () => void }) {
  const { strings } = state;
  const [typed, setTyped] = useState('');
  if (state.connect) return <ProviderPicker state={state} connect={state.connect} onConnections={onConnections} />;
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

function AssistantComposer({ state, onConnections }: { state: AssistantState; onConnections: () => void }) {
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
                    ) : file.report ? (
                      <ClipboardCheck aria-hidden />
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
              <ModelPicker state={state} onConnections={onConnections} />
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

/** What the agent menu lists though it cannot be picked: the basic agents not found. */
function notReadyOf(state: AssistantState) {
  return state.missing.filter(agent => agent.basic);
}

/** What is known of an agent, at the end of its row in the menu. */
const ROW_HINT = 'ml-auto inline-block pl-4 whitespace-nowrap text-muted-foreground first-letter:uppercase';

/** The panel's own bar: the agent and what it runs with, its state, and the panel's controls. */
function AssistantBar({ state, onSetup, onConnections }: { state: AssistantState; onSetup: (id: string) => void; onConnections: () => void }) {
  const { strings } = state;
  const agent = state.agents.find(entry => entry.id === state.agent);
  const name = agent?.name ?? (state.detecting ? strings.detecting : strings.noneFound);
  const badge = stateBadge(state);
  // The agent's own settings; its model is picked beside the message box, and so is the built-in agent's effort.
  const settings = (state.config ?? []).filter(option => option.category !== 'model' && !(state.connect && option.category === 'thought_level'));
  const notReady = notReadyOf(state);
  // The built-in agent is the only one there is: its connections stand where the agents are picked.
  const connect = state.connect;
  // What the agent itself offers to set, in the menu the agent is in: its modes and its options.
  const ownSettings = (
    <>
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
    </>
  );
  return (
    <ChatShell.Bar>
      <ChatShell.Column className="flex-row items-center justify-between gap-2 py-2">
        <div className="-m-0.5 flex h-8 min-w-0 flex-1 flex-wrap content-start items-center gap-x-1.5 gap-y-2 overflow-hidden p-0.5">
          {connect ? (
            <ConnectionButton state={state} connect={connect} onOpen={onConnections} />
          ) : (
            <DropdownMenu>
              <DropdownMenu.Trigger variant="ghost" size="sm" aria-label={strings.switchAgent} tooltip={strings.switchAgent} className="max-w-full min-w-12">
                <span className="truncate">{name}</span>
                <ChevronDown />
              </DropdownMenu.Trigger>
              {/* As tall as the panel has room for: the rows under the agents are not left to a scroll. */}
              <DropdownMenu.Content align="start" className="max-h-[var(--available-height)]">
                {/* The agents to pick, each with what is known of it without starting it. */}
                {state.agents.length > 0 && (
                  <DropdownMenu.RadioGroup
                    value={state.agent ?? ''}
                    onValueChange={(id: unknown) => {
                      if (typeof id === 'string' && state.agents.some(entry => entry.id === id)) post({ type: 'agent', id });
                    }}
                  >
                    {state.agents.map(entry => (
                      <DropdownMenu.RadioItem key={entry.id} value={entry.id} closeOnClick>
                        <span className="flex min-w-0 flex-1 items-center">
                          <span className="truncate">{entry.name}</span>
                          {entry.hint && <span className={ROW_HINT}>{entry.hint}</span>}
                        </span>
                        {/* The room the picked row's check mark takes, so the words line up. */}
                        {entry.id !== state.agent && <span aria-hidden className="size-icon-sm shrink-0" />}
                      </DropdownMenu.RadioItem>
                    ))}
                  </DropdownMenu.RadioGroup>
                )}
                {/* Not agents to pick: a row says what is missing, and opens how to add it. */}
                {notReady.length > 0 && (
                  <>
                    {state.agents.length > 0 && <DropdownMenu.Separator />}
                    <DropdownMenu.Label>{strings.notReady}</DropdownMenu.Label>
                    {notReady.map(entry => (
                      <DropdownMenu.Item key={entry.id} onClick={() => onSetup(entry.id)}>
                        <span className="flex min-w-0 flex-1 items-center">
                          <span className="truncate">{entry.name}</span>
                          <span className={ROW_HINT}>{strings.notSetUp}</span>
                        </span>
                        <span aria-hidden className="size-icon-sm shrink-0" />
                      </DropdownMenu.Item>
                    ))}
                  </>
                )}
                {(state.modes.length > 0 || settings.length > 0) && <DropdownMenu.Separator />}
                {ownSettings}
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
          )}
          {/* It wraps out of sight when the name leaves it no room. With nothing connected the button beside it says so, and a state would only repeat it. */}
          {!nothingConnected(state) && (
            <Badge variant={badge.variant} emphasis="subtle" size="sm" className="shrink-0 whitespace-nowrap">
              {badge.label}
            </Badge>
          )}
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
              {/* The built-in agent has no menu of its own in the bar: what that menu held for the agent is here. */}
              {connect && (
                <>
                  <DropdownMenu.Separator />
                  <DropdownMenu.Item onClick={onConnections}>{strings.connTitle}</DropdownMenu.Item>
                  {ownSettings}
                  <DropdownMenu.Item onClick={() => post({ type: 'settings' })}>{strings.settings}</DropdownMenu.Item>
                  {state.running && (
                    <>
                      <DropdownMenu.Separator />
                      <DropdownMenu.Item onClick={() => post({ type: 'disconnect' })}>{strings.disconnect}</DropdownMenu.Item>
                    </>
                  )}
                </>
              )}
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

/** A basic agent that is not found: what to do, the line that sets it up, and Refresh. */
function AdapterSetup({ agent, strings, onClose }: { agent: MissingAgent; strings: Strings; onClose: () => void }) {
  return (
    // Capped as the other parts by the message box are: a long path scrolls in its own place, and in
    // a short panel the card scrolls, so the message box under it is never pushed out of the panel.
    <Notice variant="info" title={agent.name} className="max-h-[calc(100dvh-12rem)] overflow-y-auto bg-card">
      <div className="flex max-h-[30dvh] min-w-0 shrink-0 flex-col gap-2 overflow-y-auto">
        <Notice.Message>{agent.body}</Notice.Message>
      </div>
      <div className="flex min-w-0 flex-col gap-3 pt-2">
        <div className="flex min-w-0 items-start gap-2">
          <ToolCallOutput text={agent.install} />
          <CopyButton content={agent.install} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'refresh' })}>
            {strings.refresh}
          </Button>
          <Button type="button" size="sm" onClick={onClose}>
            {strings.close}
          </Button>
        </div>
      </div>
    </Notice>
  );
}

/* ---- The built-in agent's connections ----
 * The button in the bar, the page that takes the conversation's place, and the page's parts.
 * All of it is drawn only while the app sends `connect`, which it does for the built-in agent
 * alone, and every action here is one the app refuses for any other agent.
 */

/**
 * The way to the connections, always in the bar: it names the connection in use, or says that
 * nothing is connected, and opens the page where one signs in, signs out and changes provider.
 */
function ConnectionButton({ state, connect, onOpen }: { state: AssistantState; connect: Connect; onOpen: () => void }) {
  const { strings } = state;
  // Before the agent has a model, the first connection there is stands for all of them.
  const used = connect.connected ? (connectionOf(connect, modelChoice(state).current) ?? liveOf(connect)[0]) : undefined;
  return (
    <Button type="button" variant="ghost" size="sm" aria-label={strings.connButtonTip} tooltip={strings.connButtonTip} className="max-w-full min-w-12" onClick={onOpen}>
      {used ? (
        <span aria-hidden className="flex shrink-0">
          <Mark name={used.name} size={14} />
        </span>
      ) : (
        <Plug />
      )}
      {connect.connected && <span className="shrink-0 text-muted-foreground">{strings.connButton}</span>}
      {/* An agent that is not running has not said what is connected: the button names the page, not a state. */}
      <span className="truncate">{connect.connected ? used && shortName(used.name) : listed(connect) ? strings.connButtonNone : strings.connTitle}</span>
      <ChevronRight />
    </Button>
  );
}

/** How a connection stands, for Mastra's status mark: one that is not there is a ring, not a dot. */
function standing(label: string, tone: 'success' | 'neutral' | 'error' | 'progress') {
  return { label, description: label, tone, glyph: tone === 'neutral' ? ('ring' as const) : ('filled' as const) };
}

/**
 * One connection on the connections page: what it is, what can be done, and under it a sign-in
 * under way. How it stands is said only when the button beside it does not say it already: a row
 * with **Sign in** needs no "not signed in". `detail` is the one more thing worth a glance, the
 * account or the address.
 */
function ConnRow({
  entry,
  detail,
  note,
  status,
  actions,
  children,
}: {
  entry: Connection;
  detail?: string | null;
  note?: string;
  status?: ReturnType<typeof standing>;
  actions: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5 px-3 py-1.5">
      {/* One line where the panel has room: the name, how it stands, what can be done. In a panel as
          narrow as the app's the standing goes under the name and the buttons under both, so that no
          name is cut short. */}
      <div className="flex min-h-control-sm min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
        <span className="flex min-w-0 flex-1 basis-44 flex-wrap items-center gap-x-2.5 gap-y-0.5">
          <span className="flex min-w-0 items-center gap-2">
            <span aria-hidden className="flex shrink-0">
              <Mark name={entry.name} size={14} />
            </span>
            <span className="min-w-0 text-label break-words">{entry.name}</span>
          </span>
          {status && <Status presentation={status} className="shrink-0 text-caption" />}
          {detail && (
            <Txt variant="caption" tone="muted" className="min-w-0 break-all">
              {detail}
            </Txt>
          )}
        </span>
        <span className="flex shrink-0 flex-wrap items-center gap-1.5">{actions}</span>
      </div>
      {note && (
        <Txt variant="caption" tone="muted" className="ps-[1.375rem] break-words">
          {note}
        </Txt>
      )}
      {children}
    </div>
  );
}

/** What went wrong, where it went wrong: under the row or in the form, in the colour of an error. */
function Wrong({ children }: { children: ReactNode }) {
  return (
    <Txt as="p" variant="caption" className="text-destructive-foreground" role="alert">
      {children}
    </Txt>
  );
}

/**
 * A key for one provider. What is typed stays in this box until Save hands it to the app, which
 * keeps it in the system keychain; nothing shows it again. A save is answered by a later state:
 * the key saved and no error, and the dialog is done; an error, and it stays to say so.
 */
function KeyDialog({
  entry,
  error,
  stamp,
  strings,
  onClose,
}: {
  entry: Connect['keys'][number];
  error: string | null;
  stamp: unknown;
  strings: Strings;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState('');
  // Whether a save waits for its answer, and whether the last one was refused. An error that
  // was there before this save is not about this key, so only an answer is shown.
  const [waiting, setWaiting] = useState(false);
  const [refused, setRefused] = useState(false);
  useEffect(() => {
    if (!waiting) return;
    if (error) {
      setWaiting(false);
      setRefused(true);
    } else if (entry.saved) {
      onClose();
    }
  }, [stamp]);
  const title = strings.connKeyTitle.replace('{name}', entry.name);
  function save() {
    const key = draft.trim();
    if (!key) return;
    setWaiting(true);
    setRefused(false);
    post({ type: 'conn_key', provider: entry.id, key });
  }
  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      {/* Cancel closes it: Mastra's own cross has an English name. */}
      <DialogContent showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{strings.connKeyBody}</DialogDescription>
        </DialogHeader>
        <DialogBody>
          <Input
            autoFocus
            type="password"
            maxLength={4096}
            aria-label={title}
            placeholder={strings.connKeyPlaceholder}
            value={draft}
            onChange={event => setDraft(event.target.value)}
            onKeyDown={event => {
              if (event.key === 'Enter') save();
            }}
          />
          {refused && error && <Wrong>{error}</Wrong>}
        </DialogBody>
        <DialogFooter>
          <DialogCancel>{strings.cancel}</DialogCancel>
          <DialogAction disabled={!draft.trim()} onConfirm={save}>
            {strings.connSave}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Whether an address is one the app keeps: http or https, a host, and neither a sign-in nor a query in it. */
function plainAddress(text: string): 'ok' | 'not' | 'secret' {
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return 'not';
  }
  if ((url.protocol !== 'http:' && url.protocol !== 'https:') || !url.hostname || /\s/.test(text)) return 'not';
  // A key belongs in the key box: in the address it would be kept and shown as part of it.
  return url.username || url.password || url.search ? 'secret' : 'ok';
}

/**
 * An address that speaks OpenAI's protocol, or a model on this computer. Its models are typed:
 * none are found by asking the address. What is typed is checked here as the app checks it, and
 * said under the boxes; the form stays, with what was typed, until the app has kept the address.
 */
function CustomForm({
  connect,
  stamp,
  strings,
  onDone,
}: {
  connect: Connect;
  stamp: unknown;
  strings: Strings;
  onDone: () => void;
}) {
  const [draft, setDraft] = useState({ name: '', url: '', key: '', models: '' });
  const [missing, setMissing] = useState<string | null>(null);
  // The name sent, while the app's answer is awaited: the address listed, or its reason for not keeping it.
  const [waiting, setWaiting] = useState<string | null>(null);
  useEffect(() => {
    if (!waiting) return;
    if (connect.error) {
      setWaiting(null);
      setMissing(connect.error);
    } else if (connect.custom.some(entry => entry.name === waiting)) {
      onDone();
    }
  }, [stamp]);
  function add() {
    const name = draft.name.trim();
    const url = draft.url.trim();
    const address = plainAddress(url);
    if (!name || address === 'not') return setMissing(strings.connCustomNeedBoth);
    if (address === 'secret') return setMissing(strings.connCustomPlainUrl);
    if (connect.custom.some(entry => entry.name === name)) return setMissing(strings.connCustomExists);
    const models = draft.models
      // A comma as it is typed in Chinese separates as well.
      .split(/[,\uFF0C]/)
      .map(model => model.trim())
      .filter(Boolean);
    if (models.length === 0) return setMissing(strings.connCustomNeedModel);
    if (models.length > 16 || models.some(model => model.length > 120)) return setMissing(strings.connCustomTooMany);
    const key = draft.key.trim();
    setMissing(null);
    setWaiting(name);
    post(key ? { type: 'conn_custom', name, url, key, models } : { type: 'conn_custom', name, url, models });
  }
  return (
    <div className="flex min-w-0 flex-col gap-3 rounded-lg border border-border p-3">
      {missing && <Wrong>{missing}</Wrong>}
      <Field className="gap-1">
        <FieldLabel>{strings.connCustomName}</FieldLabel>
        <Input autoFocus size="sm" maxLength={40} value={draft.name} onChange={event => setDraft({ ...draft, name: event.target.value })} />
      </Field>
      <Field className="gap-1">
        <FieldLabel>{strings.connCustomUrl}</FieldLabel>
        <Input size="sm" maxLength={400} value={draft.url} onChange={event => setDraft({ ...draft, url: event.target.value })} />
      </Field>
      <Field className="gap-1">
        <FieldLabel>{strings.connCustomKey}</FieldLabel>
        <Input
          type="password"
          size="sm"
          maxLength={4096}
          placeholder={strings.connKeyPlaceholder}
          value={draft.key}
          onChange={event => setDraft({ ...draft, key: event.target.value })}
        />
      </Field>
      <Field className="gap-1">
        <FieldLabel>{strings.connCustomModels}</FieldLabel>
        <Input size="sm" value={draft.models} onChange={event => setDraft({ ...draft, models: event.target.value })} />
      </Field>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" variant="primary" disabled={waiting !== null} onClick={add}>
          {strings.connAdd}
        </Button>
        <Button type="button" size="sm" onClick={onDone}>
          {strings.cancel}
        </Button>
      </div>
    </div>
  );
}

/**
 * A sign-in that ends with a code: the browser shows it once the user agrees, and it is pasted
 * back here. The agent judges it; one it finds incomplete stays in the box to be mended.
 */
function PasteCode({ id, incomplete, strings }: { id: string; incomplete: boolean; strings: Strings }) {
  const [draft, setDraft] = useState('');
  const code = draft.trim();
  const done = () => post({ type: 'conn_code', id, code });
  return (
    <div className="flex min-w-0 flex-col gap-2 pt-2">
      <Txt variant="caption">{strings.connCodeHint}</Txt>
      <Input
        size="sm"
        maxLength={4096}
        aria-label={strings.connCodePlaceholder}
        placeholder={strings.connCodePlaceholder}
        value={draft}
        onChange={event => setDraft(event.target.value)}
        onKeyDown={event => {
          if (event.key === 'Enter' && code) done();
        }}
      />
      {incomplete && <Wrong>{strings.connCodeIncomplete}</Wrong>}
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" variant="primary" disabled={!code} onClick={done}>
          {strings.connCodeDone}
        </Button>
        <Button type="button" size="sm" onClick={() => post({ type: 'conn_cancel' })}>
          {strings.cancel}
        </Button>
      </div>
    </div>
  );
}

/**
 * Where a plan's sign-in stands, under its row: the page is being opened, the browser has it, a
 * code is to be typed there, or a code is to be pasted here. A sign-in waits for the user as
 * long as it takes, so each step that waits can be given up.
 */
function SignInProgress({ login, strings }: { login: NonNullable<Connect['login']>; strings: Strings }) {
  const ways = (
    <div className="flex flex-wrap items-center gap-2">
      <Button type="button" size="sm" onClick={() => post({ type: 'conn_reopen' })}>
        {strings.connReopen}
      </Button>
      <Button type="button" size="sm" onClick={() => post({ type: 'conn_cancel' })}>
        {strings.cancel}
      </Button>
    </div>
  );
  switch (login.phase) {
    case 'starting':
      // Getting the address can take a provider some seconds, or not come at all: this too can be given up.
      return (
        <div className="flex min-w-0 flex-col gap-2 pt-2">
          <Txt variant="caption">{strings.connStarting}</Txt>
          <div>
            <Button type="button" size="sm" onClick={() => post({ type: 'conn_cancel' })}>
              {strings.cancel}
            </Button>
          </div>
        </div>
      );
    case 'browser':
      return (
        <div className="flex min-w-0 flex-col gap-2 pt-2">
          <Txt variant="caption">{strings.connWaiting}</Txt>
          {ways}
        </div>
      );
    case 'device':
      return (
        <div className="flex min-w-0 flex-col gap-2 pt-2">
          <Txt variant="caption">{strings.connDeviceHint}</Txt>
          {login.userCode && (
            <div className="flex min-w-0 items-start gap-2">
              <ToolCallOutput text={login.userCode} />
              <CopyButton content={login.userCode} />
            </div>
          )}
          {ways}
        </div>
      );
    case 'code':
      return <PasteCode id={login.id} incomplete={login.error === 'incomplete'} strings={strings} />;
  }
}

/** A sign-in that could not finish, under its row: why, and the two ways on. */
function SignInProblem({ problem, busy, strings }: { problem: NonNullable<Connect['problem']>; busy: boolean; strings: Strings }) {
  const why = { port: strings.connProblemPort, offline: strings.connProblemOffline, failed: strings.connProblemFailed }[problem.kind];
  return (
    <div className="flex min-w-0 flex-col gap-2 pt-2">
      <Wrong>{why ?? strings.connProblemFailed}</Wrong>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" variant="primary" disabled={busy} onClick={() => post({ type: 'conn_login', id: problem.id })}>
          {strings.retry}
        </Button>
        <Button type="button" size="sm" onClick={() => post({ type: 'conn_cancel' })}>
          {strings.cancel}
        </Button>
      </div>
    </div>
  );
}

/**
 * The one place for connections: every way the built-in agent can reach a model, each with how
 * it stands and what can be done. A new user lands here, and the button in the bar comes back to
 * it at any time.
 */
function ConnectionsPage({ state, connect, onClose }: { state: AssistantState; connect: Connect; onClose: () => void }) {
  const { strings } = state;
  const [keyFor, setKeyFor] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const { option, current, ids } = modelChoice(state);
  const used = connectionOf(connect, current);
  // Only one sign-in runs at a time.
  const signing = connect.login !== null;
  const keyEntry = connect.keys.find(key => key.id === keyFor);
  /** A connection's newest model, in the order Mastra's picker lists them: by release date, then by id. */
  const newest = (entry: Connection) =>
    ids.filter(id => connectionOf(connect, id) === entry).sort((a, b) => compareModelReleaseDates(connect.dates[a], connect.dates[b]) || a.localeCompare(b))[0];
  // Changing provider is picking a model of it. A connection the agent lists no model for has nothing to pick.
  const usedOrUse = (entry: Connection) => {
    if (entry === used) {
      return (
        <Badge variant="success" emphasis="subtle" size="sm" className="shrink-0 whitespace-nowrap">
          {strings.connInUse}
        </Badge>
      );
    }
    const model = newest(entry);
    return (
      model && (
        <Button type="button" size="sm" disabled={state.configPending} onClick={() => pickModel(option, model)}>
          {strings.connUse}
        </Button>
      )
    );
  };
  const askKey = (id: string) => setKeyFor(id);
  return (
    <div className="flex min-w-0 flex-col gap-3 py-2">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <h2 className="min-w-0 truncate text-sm font-medium">{strings.connTitle}</h2>
        {connect.connected && (
          <Button type="button" size="sm" variant="primary" className="shrink-0" onClick={onClose}>
            {strings.connBack}
          </Button>
        )}
      </div>
      {/* What is in use is marked on its row below. */}
      {!connect.connected && (
        <Txt variant="caption" tone="muted">
          {strings.connNoneTitle}
        </Txt>
      )}

      <SettingsGroup>
      <SettingsHeader>
        <SettingsTitle>{strings.connPlans}</SettingsTitle>
      </SettingsHeader>
      <SettingsContainer>
      {connect.plans.map(plan => {
        const here = connect.login?.id === plan.id ? connect.login : null;
        const status =
          plan.state === 'in'
            ? standing(strings.connSignedIn, 'success')
            : here
              ? standing(strings.connSigningIn, 'progress')
              : plan.state === 'expired'
                ? standing(strings.connExpired, 'error')
                : undefined;
        const signOut = (
          <Button type="button" size="sm" onClick={() => post({ type: 'conn_logout', id: plan.id })}>
            {strings.connSignOut}
          </Button>
        );
        // The row whose sign-in is under way has its own ways on under it; the others wait for it.
        const signIn = (label: string) =>
          !here && (
            <Button type="button" size="sm" variant={plan.state === 'expired' ? 'primary' : 'default'} disabled={signing} onClick={() => post({ type: 'conn_login', id: plan.id })}>
              {label}
            </Button>
          );
        return (
          <ConnRow
            key={plan.id}
            entry={plan}
            detail={plan.state === 'out' ? undefined : plan.account}
            status={status}
            actions={
              plan.state === 'in' ? (
                <>
                  {usedOrUse(plan)}
                  {signOut}
                </>
              ) : plan.state === 'expired' ? (
                <>
                  {signIn(strings.connSignInAgain)}
                  {signOut}
                </>
              ) : (
                signIn(strings.connSignIn)
              )
            }
          >
            {here ? (
              <SignInProgress login={here} strings={strings} />
            ) : (
              connect.problem?.id === plan.id && <SignInProblem problem={connect.problem} busy={signing} strings={strings} />
            )}
          </ConnRow>
        );
      })}
      </SettingsContainer>
      </SettingsGroup>

      <SettingsGroup>
      <SettingsHeader>
        <SettingsTitle>{strings.connKeys}</SettingsTitle>
      </SettingsHeader>
      <SettingsContainer>
      {connect.keys.map(key => (
        <ConnRow
          key={key.id}
          entry={key}
          note={key.saved && key.shadowed ? strings.connShadowed : undefined}
          status={key.saved ? standing(strings.connKeySaved, 'success') : undefined}
          actions={
            key.saved ? (
              <>
                {!key.shadowed && usedOrUse(key)}
                <Button type="button" size="sm" onClick={() => askKey(key.id)}>
                  {strings.connKeyUpdate}
                </Button>
                <Button type="button" size="sm" onClick={() => post({ type: 'conn_key_remove', provider: key.id })}>
                  {strings.connRemove}
                </Button>
              </>
            ) : (
              <Button type="button" size="sm" onClick={() => askKey(key.id)}>
                {strings.connKeyAdd}
              </Button>
            )
          }
        />
      ))}
      </SettingsContainer>
      </SettingsGroup>

      <SettingsGroup>
      <SettingsHeader>
        <SettingsTitle>{strings.connCustom}</SettingsTitle>
      </SettingsHeader>
      <SettingsContainer>
      {connect.custom.map(entry => (
        <ConnRow
          key={entry.id}
          entry={entry}
          detail={entry.url}
          actions={
            <>
              {usedOrUse(entry)}
              <Button type="button" size="sm" onClick={() => post({ type: 'conn_custom_remove', id: entry.id })}>
                {strings.connRemove}
              </Button>
            </>
          }
        />
      ))}
      {adding ? (
        <div className="min-w-0 px-3 py-2.5">
          <CustomForm connect={connect} stamp={state} strings={strings} onDone={() => setAdding(false)} />
        </div>
      ) : (
        <div className="px-3 py-1.5">
          <Button type="button" size="sm" variant="ghost" className="-mx-2" onClick={() => setAdding(true)}>
            <Plus />
            {strings.connCustomAdd}
          </Button>
        </div>
      )}
      </SettingsContainer>
      </SettingsGroup>
      {keyEntry && <KeyDialog entry={keyEntry} error={connect.error} stamp={state} strings={strings} onClose={() => setKeyFor(null)} />}
    </div>
  );
}

/**
 * The ways on from a turn that failed on its connection, under the app's sentence about it:
 * sign in again, look at the connections, or send the message again.
 */
function ProblemWays({
  problem,
  connect,
  busy,
  strings,
  onConnections,
}: {
  problem: TurnProblem;
  connect: Connect;
  busy: boolean;
  strings: Strings;
  onConnections: () => void;
}) {
  // A sign-in that was refused is a plan's (`auth`); a key that was refused (`key`) is mended on the page.
  const plan = connect.plans.find(entry => entry.prefix === problem.prefix && entry.state !== 'out');
  const way =
    problem.kind === 'auth' ? (
      <Button
        type="button"
        size="sm"
        variant="primary"
        onClick={() => {
          if (plan && !connect.login) post({ type: 'conn_login', id: plan.id });
          // The sign-in goes on where its steps are shown: a code to type or to paste is asked for there.
          onConnections();
        }}
      >
        {strings.connSignInAgain}
      </Button>
    ) : problem.kind === 'key' ? (
      <>
        <Button type="button" size="sm" variant="primary" onClick={onConnections}>
          {strings.connTitle}
        </Button>
        {/* The message is kept: with the key mended it is sent from here. */}
        <Button type="button" size="sm" disabled={busy} onClick={() => post({ type: 'conn_retry' })}>
          {strings.retry}
        </Button>
      </>
    ) : problem.kind === 'quota' || problem.kind === 'model' ? (
      <Button type="button" size="sm" onClick={onConnections}>
        {strings.connTitle}
      </Button>
    ) : problem.kind === 'network' || problem.kind === 'rate' ? (
      <Button type="button" size="sm" disabled={busy} onClick={() => post({ type: 'conn_retry' })}>
        {strings.retry}
      </Button>
    ) : null;
  return way && <div className="flex flex-wrap items-center gap-2 pt-2">{way}</div>;
}

/** Nothing said yet: the Assistant's name, or how to get an agent when there is none. */
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
  return <EmptyState iconSlot={<MessageSquare />} titleSlot={strings.title} />;
}

/**
 * The room Mastra's chat frame leaves round the conversation and the message box is for a page as
 * wide as a browser window. In a panel beside a model it is a wide band on each side of the box and
 * under it, so the frame's own measures are set smaller here: the room at each side (`--chat-edge`,
 * and the column's own in `COLUMN`), under the box (`--chat-gutter`) and the band the conversation
 * fades out in above it (`--chat-fade`).
 */
const ROOM = { '--chat-edge': '0rem', '--chat-gutter': '0.5rem', '--chat-fade': '1.25rem' } as CSSProperties;
const COLUMN = 'px-1 md:px-1';

/** Whether the built-in agent has told its connections: it lists its sign-ins and keys whenever it runs. */
const listed = (connect: Connect) => connect.plans.length + connect.keys.length + connect.custom.length > 0;

/** The built-in agent runs and has nothing to talk to: the Connections page is then the panel. */
const nothingConnected = (state: AssistantState | null) => Boolean(state?.connect && listed(state.connect) && !state.connect.connected);

export function App({ state }: { state: AssistantState | null }) {
  const turns = useMemo(() => turnsOf(state?.items ?? []), [state?.items]);
  // The agent whose missing adapter is being explained; the card goes once Refresh finds it.
  const [setup, setSetup] = useState<string | null>(null);
  // The built-in agent's connections page takes the conversation's place while it is open. With
  // the agent running and nothing connected it is the panel; once a connection is there it is not
  // taken away behind the user's back: they go to the conversation with its button.
  const [connOpen, setConnOpen] = useState(false);
  const none = nothingConnected(state);
  useEffect(() => {
    if (none) setConnOpen(true);
  }, [none]);
  if (!state) return null;
  const { strings } = state;
  const connect = state.connect;
  const explained = notReadyOf(state).find(agent => agent.id === setup);
  const busy = state.state === 'busy';
  // The page is made of what the agent says of its connections. One that is not running (not
  // started yet, stopped, or failed to start) has said nothing, and the page would be empty: the
  // conversation stays, with the app's words about the agent and the bar's button to start it.
  const showConn = connect ? listed(connect) && (connOpen || none) : false;
  const openConn = () => {
    setConnOpen(true);
    // Asked for while the agent is not running: it is started, and the page shows once it has answered.
    if (connect && !listed(connect) && !state.running) post({ type: 'connect' });
  };
  // The built-in agent's sign-ins are on its connections page, not on the app's sign-in card.
  const signIn = Boolean(state.auth) && !connect;
  const confirmNew = state.confirmNew && (
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
  );
  const plan = turns.at(-1)?.plan;
  const tasks: TaskListItem[] = (plan ?? []).map((step, index) => ({
    id: String(index),
    content: step.text,
    activeForm: step.text,
    status: step.status === 'completed' || step.status === 'in_progress' ? step.status : 'pending',
  }));
  return (
    <div className="h-dvh overflow-hidden bg-background" style={{ borderRadius: state.radius }}>
      <ChatShell className="h-full" style={ROOM} scroller={{ autoScroll: true, defaultScrollPosition: 'end' }}>
        <AssistantBar state={state} onSetup={setSetup} onConnections={openConn} />
        {connect && showConn && (
          <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
            {/* The bar's New chat asks here too: the conversation it is asked in is out of sight. */}
            {confirmNew}
            <ConnectionsPage state={state} connect={connect} onClose={() => setConnOpen(false)} />
          </div>
        )}
        {/* Out of sight, not gone: what is typed in the message box is still there on the way back. */}
        <ChatShell.Stage className={showConn ? 'hidden' : undefined}>
          <ChatShell.Viewport>
            <ChatShell.Content>
              <ChatShell.Column className={`gap-8 py-6 ${COLUMN}`}>
                {state.notes.length > 0 && (
                  <div className="flex min-w-0 flex-col gap-1">
                    {state.notes.map((note, index) => (
                      <Txt key={index} variant="caption">
                        {note}
                      </Txt>
                    ))}
                  </div>
                )}
                {confirmNew}
                {signIn && <SignIn state={state} />}
                {turns.length === 0 && state.pending.length === 0 && !signIn && <Welcome state={state} />}
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
                        <Response
                          turn={turn}
                          busy={live}
                          strings={strings}
                          ways={connect && index === turns.length - 1 ? { connect, onConnections: openConn } : undefined}
                        />
                      </MessageScrollerItem>
                    </ChatShell.Turn>
                  );
                })}
              </ChatShell.Column>
            </ChatShell.Content>
            <ChatShell.Dock>
              <ChatShell.ScrollButton aria-label={strings.latest} />
              <ChatShell.Column className={`gap-2 ${COLUMN}`}>
                {state.dragging && (
                  <Notice variant="info" className="bg-card">
                    <Notice.Message>{strings.dropHint}</Notice.Message>
                  </Notice>
                )}
                {/* By the message box, so it is in sight wherever the conversation is scrolled to. */}
                {explained && <AdapterSetup agent={explained} strings={strings} onClose={() => setSetup(null)} />}
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
                <AssistantComposer state={state} onConnections={openConn} />
              </ChatShell.Column>
            </ChatShell.Dock>
          </ChatShell.Viewport>
        </ChatShell.Stage>
      </ChatShell>
    </div>
  );
}
