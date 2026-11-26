// The Assistant's conversation and composer, drawn with Mastra's playground-ui components and
// fed with the desktop app's own transcript. Everything visual is imported from the Mastra
// checkout; this file only maps our items onto those components.
import {
  ArrowUp,
  Camera,
  Check,
  ChevronDown,
  ChevronRight,
  ClipboardCheck,
  Ellipsis,
  FileImage,
  FileText,
  GripVertical,
  Hammer,
  KeyRound,
  Map as MapIcon,
  Maximize2,
  MessageSquare,
  Minimize2,
  MousePointerClick,
  Pencil,
  Plug,
  Plus,
  ShieldCheck,
  Sparkles,
  Square,
  SquarePen,
  Target,
  Undo2,
  X,
  Zap,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, MouseEvent, ReactNode } from 'react';
import { flushSync } from 'react-dom';
import type { AssistantState, Connect, ContextChip, ContextGroups, Item, MissingAgent, PermissionOption, PictureRef, QueuedMessage, Strings, TurnProblem } from './host';
import { pictureUrl, post } from './host';
import { StateIcon } from './tickets/status-icons';
import { ModelPicker as MastraModelPicker } from '@/domains/llm/model-picker';
import type { ModelPickerProvider } from '@/domains/llm/model-picker-types';
import { ProviderLogoMonogram } from '@/domains/llm/provider-logo-monogram';
import { agentGlyph } from '@/ds/components/AgentIndicator';
import { Activity, ActivityContent, ActivityDetail, ActivityDisclosure, ActivityHeader, ActivityHeadline, ActivityIcon, ActivityLabel, ActivitySpacer, ActivityTrigger } from '@/ds/components/ai/activity';
import { messagePreview } from '@/ds/components/ai/activity/presets/message-preview';
import type { TaskListItem } from '@/ds/components/ai/task-list';
import { TaskList, TaskListContainer } from '@/ds/components/ai/task-list';
import { ToolCallBody, ToolCallGroup, ToolCallOutput, presentTool } from '@/ds/components/ai/tool-call';
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
  ComposerHeader,
  ComposerHeaderActions,
  ComposerHeaderContent,
  ComposerHeaderRow,
  ComposerInput,
  ComposerRing,
  ComposerStatus,
  ComposerToneLabel,
} from '@/ds/components/Composer';
import type { ComposerTone } from '@/ds/components/Composer';
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
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/ds/components/Select';
import { Status } from '@/ds/components/StatusIndicators/status';
import { Textarea } from '@/ds/components/Textarea';
import { ThinkingLevelPicker } from '@/ds/components/ThinkingLevel';
import type { ThinkingLevelOption } from '@/ds/components/ThinkingLevel';
import { Txt } from '@/ds/components/Txt';
import { SettingsContainer } from '@/ds/new/settings/settings-container';
import { SettingsGroup, SettingsHeader, SettingsTitle } from '@/ds/new/settings/settings-header';
import { humanize } from '@/lib/string';
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
  | { kind: 'ticket'; item: Of<'ticket'> }
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

/** A tool's arguments as the agent gave them, when what the app sent of them is still an object's JSON (a long one is cut). */
function argsOf(input: string | null): Record<string, unknown> | undefined {
  if (!input) return undefined;
  try {
    const value: unknown = JSON.parse(input);
    return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

/**
 * One tool call. With the built-in agent (`own`) the row is Mastra's own: its presenter picks the
 * argument worth a glance for the headline (the command, the path, the query), and its body draws
 * the details, a command with what it printed, or the arguments and the result.
 */
/** One picture a tool call handed back, as Mastra's sent picture: the card with its preview, or with an icon if the picture cannot be drawn. */
function HandedPicture({ picture, name, open }: { picture: PictureRef; name: string; open: () => void }) {
  const [failed, setFailed] = useState(false);
  return (
    <AttachmentCard
      name={name}
      typeLabel={typeOf(picture.note)}
      icon={<FileImage aria-hidden />}
      onClick={failed ? undefined : open}
      preview={
        failed ? undefined : (
          <img src={pictureUrl(picture.key)} alt={name} className="size-full object-cover" draggable={false} onError={() => setFailed(true)} />
        )
      }
    />
  );
}

/**
 * A text file opened from a file card, as Mastra's text preview: its name and its text. Markdown is
 * drawn as an answer's is, so a link in it opens in the browser and nothing loads from the network;
 * any other text, HTML too, is shown as it is written.
 */
function FilePreviewDialog({ file }: { file: AssistantState['filePreview'] }) {
  return (
    <Dialog open={file !== null} onOpenChange={open => !open && post({ type: 'file_close' })}>
      <DialogContent size="xl">
        <DialogHeader>
          <DialogTitle>{file?.name}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          {file &&
            (file.markdown ? (
              <MarkdownRenderer streaming={false}>{file.text}</MarkdownRenderer>
            ) : (
              <div className="wrap-anywhere whitespace-pre-wrap">{file.text}</div>
            ))}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

function ToolRow({ tool, approval, strings, own }: { tool: Tool; approval?: Approval; strings: Strings; own: boolean }) {
  const { name, detail } = nameOf(tool.title);
  const waiting = approval?.answer === null;
  const args = own ? argsOf(tool.input) : undefined;
  const shown = presentTool(name, args ?? {});
  const { icon: ToolIcon, label } = shown;
  const status = activityStatus(tool, waiting);
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Activity status={status} aria-label={tool.title}>
        <ActivityTrigger>
          <ActivityHeadline
            icon={<ToolIcon aria-hidden />}
            label={tool.words ?? label}
            detail={tool.words ? undefined : (detail ?? shown.detail)}
            description={tool.words ? undefined : shown.description}
            badges={approval ? approvalBadge(approval, strings) : undefined}
          />
        </ActivityTrigger>
        <ActivityContent>
          {own ? (
            <ToolCallBody
              toolName={name}
              args={args}
              argsText={args ? undefined : (tool.input ?? undefined)}
              status={status}
              command={shown.command}
              result={tool.output ?? undefined}
            />
          ) : (
            <>
              {tool.input && <ToolCallOutput text={tool.input} />}
              {tool.output && <ToolCallOutput text={tool.output} error={tool.status === 'failed'} />}
            </>
          )}
        </ActivityContent>
      </Activity>
      {/* A picture the call handed back, under the row as Mastra's tool card has it, in Mastra's
          attachment card. It opens in the app's viewer, as a sent picture does: the panel is narrow. */}
      {tool.pictures.length > 0 && (
        <div className="flex min-w-0 flex-wrap gap-2 pl-7">
          {tool.pictures.map((picture, at) => (
            <HandedPicture key={picture.key} picture={picture} name={picture.note || strings.viewAttached} open={() => post({ type: 'picture', index: tool.index, at })} />
          ))}
        </div>
      )}
      {/* A file the call read, as Mastra's attachment card: a text file is previewed here, a document
          opens in the system's program for it. */}
      {tool.file && (
        <div className="flex min-w-0 flex-wrap gap-2 pl-7">
          <AttachmentCard
            name={tool.file.name}
            typeLabel={typeOf(tool.file.name)}
            icon={<FileText aria-hidden />}
            onClick={() => post({ type: 'file', index: tool.index })}
          />
        </div>
      )}
      {/* Outside the row's fold: a request that arrives after the row is drawn must still be seen. */}
      {waiting && approval && (
        <div className="pl-7">
          <ApprovalChoices approval={approval} strings={strings} />
        </div>
      )}
    </div>
  );
}

function Tools({ part, strings, own }: { part: Extract<Part, { kind: 'tools' }>; strings: Strings; own: boolean }) {
  const rows = part.tools.map(({ tool, approval }) => <ToolRow key={tool.id} tool={tool} approval={approval} strings={strings} own={own} />);
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

/** A part ticket the agent drafted (D-87): its state, number and title, and a way to it in the tickets view. */
function TicketCard({ item, strings }: { item: Of<'ticket'>; strings: Strings }) {
  return (
    <div className="flex min-w-0 items-center gap-2 rounded-lg border border-border1 py-2 pr-2 pl-3">
      <StateIcon state={item.state} />
      <Txt variant="caption" tone="muted" className="shrink-0">
        {item.id}
      </Txt>
      <Txt variant="caption" className="min-w-0 flex-1 font-medium break-words">
        {item.title}
      </Txt>
      <Button type="button" size="sm" variant="ghost" className="shrink-0" onClick={() => post({ type: 'ticket_show', id: item.id })}>
        {strings.ticketShow}
      </Button>
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
  own,
  ways,
}: {
  turn: Turn;
  busy: boolean;
  strings: Strings;
  /** The agent is the built-in one: its tool rows are drawn as Mastra draws them. */
  own: boolean;
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
            return <Tools key={key} part={part} strings={strings} own={own} />;
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
          case 'ticket':
            return <TicketCard key={key} item={part.item} strings={strings} />;
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

declare global {
  // Read by Mastra's model picker in place of its fixed English (PICKER_TEXTS in vite.config.mjs).
  var assistantPickerText: { search: string; none: string; all: string; refine: string; filter: string; title: string } | undefined;
}

/**
 * The built-in agent's model and its reasoning effort, at the composer's lower right, as Mastra
 * groups the two (its story "ThinkingLevel / Picker / Grouped With Model Picker"). The model is
 * in Mastra's own model picker, and the list is the models of the connection in use, newest
 * first: a user who switched to one provider is not shown another's models. Another provider is
 * switched to on the Connections page, which the picker's last row opens. The effort is Mastra's
 * thinking-level picker beside it.
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
  const at = current ? placeOf(connect, current) : null;
  const inUse = at ? groups.get(at.group) : undefined;
  // Before the agent has a model there is no connection in use to keep to: every connection's models, a group each.
  // At the bottom centre, every connection's models: the picker's provider filter switches between them.
  const providers = inUse && !centred(state) ? [inUse] : [...groups.values()].sort((a, b) => rank(a.id) - rank(b.id));
  const effort = effortOf(state);
  const mark = (group: string) => <Mark name={names.get(group) ?? group} size={14} />;
  // Mastra's picker has no property for its own words: the build puts these in their place
  // (PICKER_TEXTS in vite.config.mjs).
  globalThis.assistantPickerText = {
    search: strings.modelSearch,
    none: strings.modelNoMatch,
    all: strings.dfmAll,
    refine: strings.modelRefine,
    filter: strings.modelFilter,
    title: strings.model,
  };
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
      header={centred(state) ? <PickerConnection via={at?.via} strings={strings} onConnections={onConnections} /> : undefined}
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
    <ButtonsGroup size="sm" aria-label={strings.modelEffort} className="min-w-0">
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

/**
 * How Mastra's composer dresses a session mode: its icon, and the tone the mode gives the label and
 * the composer's ring. A mode Mastra has no dress for is plain.
 */
const MODE_DRESS: Record<string, { Icon: typeof Hammer; tone: ComposerTone }> = {
  build: { Icon: Hammer, tone: 'green' },
  plan: { Icon: MapIcon, tone: 'purple' },
  fast: { Icon: Zap, tone: 'orange' },
};

/** The built-in agent's session mode (its `mode` option), by the names the agent gives its modes. */
function sessionMode(state: AssistantState) {
  const option = state.connect ? state.config?.find(entry => entry.category === 'mode') : undefined;
  if (!option) return null;
  const dress = MODE_DRESS[option.currentValue];
  return { option, tone: dress?.tone ?? ('default' as const), Icon: dress?.Icon };
}

/** The session mode in the composer, where and as Mastra's own composer has it: a select whose label wears the mode's tone. */
function SessionMode({ state, session }: { state: AssistantState; session: NonNullable<ReturnType<typeof sessionMode>> }) {
  const { option, tone, Icon } = session;
  const current = option.options.find(choice => choice.value === option.currentValue);
  return (
    <Select
      value={option.currentValue}
      onValueChange={value => {
        if (typeof value === 'string' && value !== option.currentValue) post({ type: 'config', id: option.id, value });
      }}
      disabled={state.configPending}
    >
      <SelectTrigger variant="ghost" size="sm" aria-label={option.name} className="w-auto">
        <ComposerToneLabel tone={tone} className="inline-flex items-center gap-1.5">
          {Icon && <Icon size={12} aria-hidden />}
          {current?.name ?? option.currentValue}
        </ComposerToneLabel>
      </SelectTrigger>
      {/* Named, so the page can tell an open select from one closed and kept mounted (Base UI hides it). */}
      <SelectContent data-slot="select-content">
        {option.options.map(choice => (
          <SelectItem key={choice.value} value={choice.value}>
            {choice.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function AssistantComposer({ state, onConnections, header }: { state: AssistantState; onConnections: () => void; header?: ReactNode }) {
  const { strings } = state;
  const [text, setText] = useState('');
  const [sent, setSent] = useState(0);
  const [menuClosed, setMenuClosed] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const input = useRef<HTMLTextAreaElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const busy = state.state === 'busy';
  const reading = state.loading.length > 0;
  // The agent's own slash commands: the menu is open while the draft starts with `/` and the
  // agent has a command whose name starts with what follows it.
  const afterSlash = text.startsWith('/') ? text.slice(1) : null;
  const matches = afterSlash === null ? [] : state.commands.filter(command => command.name.startsWith(afterSlash));
  const menuOpen = matches.length > 0 && !menuClosed;
  const highlighted = Math.min(highlight, matches.length - 1);

  // Keep the highlighted row in the list's own viewport, not the page's.
  useEffect(() => {
    if (!menuOpen) return;
    listbox.current?.children.item(highlighted)?.scrollIntoView({ block: 'nearest' });
  }, [highlighted, menuOpen]);
  const canSend = !reading && (text.trim().length > 0 || state.attachments.length > 0);
  const selection = state.selected > 0 ? `${strings.attachSelection} (${state.selected})` : strings.attachSelection;
  const modes = [
    { value: 'ask' as const, label: strings.modeAsk },
    { value: 'edits' as const, label: strings.modeEdits },
    { value: 'all' as const, label: strings.modeAll },
  ];
  const mode = modes.find(option => option.value === state.permissions)?.label ?? strings.modeEdits;
  const skill = state.skills.find(option => option.id === state.skill);
  const session = sessionMode(state);
  const picked = state.picked.groups.length > 0 || state.picked.omitted !== null;
  const selected = state.selection.groups.length > 0 || state.selection.omitted !== null;
  const chips = state.attachments.length + state.loading.length > 0 || picked || selected || (state.attachView && state.images);
  // At the bottom centre the box is 520 wide and as tall as it was at rest whatever happens: its
  // controls stay on one row (the model's name is cut short instead) and its placeholder on one line.
  const centre = centred(state);
  const hint = busy ? strings.placeholderBusy : strings.placeholder;

  function submit(now = false) {
    if (!canSend) return;
    post(now ? { type: 'send', text, now } : { type: 'send', text });
    setText('');
    setSent(count => count + 1);
    input.current?.focus();
  }

  // A command goes into the draft as the agent named it; it is never sent by choosing it.
  function choose(name: string) {
    setText(name);
    setMenuClosed(true);
    input.current?.focus();
  }

  function edit(next: string) {
    setText(next);
    setMenuClosed(false);
    setHighlight(0);
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    const composing = event.nativeEvent.isComposing || event.keyCode === 229;
    // An IME owns every key it is composing, Enter, Escape and the arrows included.
    if (menuOpen && !composing) {
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setHighlight(index => (index + 1) % matches.length);
        return;
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setHighlight(index => (index - 1 + matches.length) % matches.length);
        return;
      }
      // Every Enter variant picks the highlighted command; none of them sends.
      if (event.key === 'Enter') {
        event.preventDefault();
        const command = matches[highlighted];
        if (command) choose(command.name);
        return;
      }
      if (event.key === 'Escape') {
        event.preventDefault();
        setMenuClosed(true);
        return;
      }
    }
    // A held Enter repeats keydowns; a repeat must never send the draft it just inserted. A
    // composing key is left entirely to the input method.
    if (event.key === 'Enter' && event.repeat && !composing) {
      event.preventDefault();
      return;
    }
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !composing) {
      // Send now: this message and what is queued go next, the running step stops for them.
      event.preventDefault();
      if (canSend) submit(true);
      else if (!reading && state.pending.length > 0) post({ type: 'send_now' });
    } else if (event.key === 'Enter' && !event.shiftKey && !composing) {
      event.preventDefault();
      submit();
    } else if (event.key === 'Escape' && busy && !composing) {
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
      {header}
      {menuOpen && (
        <div
          ref={listbox}
          role="listbox"
          aria-label={strings.composer}
          className="absolute bottom-full left-0 z-50 mb-2 max-h-64 w-full overflow-y-auto rounded-md border border-border bg-card p-1 shadow-md"
        >
          {matches.map((command, index) => (
            <button
              key={command.name}
              type="button"
              role="option"
              tabIndex={-1}
              aria-selected={index === highlighted}
              aria-label={command.name}
              className={`flex w-full flex-col items-start rounded-sm px-2 py-1.5 text-left ${index === highlighted ? 'bg-surface-overlay-strong' : ''}`}
              onMouseDown={event => {
                event.preventDefault();
                choose(command.name);
              }}
            >
              <span className="w-full truncate text-sm">{command.name}</span>
              {command.description !== '' && <span className="w-full truncate text-xs text-muted-foreground">{command.description}</span>}
            </button>
          ))}
        </div>
      )}
      <ComposerRing tone={session?.tone} busy={busy}>
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
            className={centre ? 'placeholder:truncate' : undefined}
            aria-label={strings.messageBox}
            placeholder={centre ? oneLine(input.current, hint) : hint}
            value={text}
            onChange={event => edit(event.target.value)}
            onKeyDown={onKeyDown}
            onPaste={event => {
              // A picture on the clipboard is attached by the app, which reads the clipboard itself.
              const data = event.clipboardData;
              if (!data.types.includes('text/plain') && [...data.items].some(item => item.type.startsWith('image/'))) {
                event.preventDefault();
                post({ type: 'paste_picture' });
              }
            }}
          />
          <ComposerActions className={centre ? 'flex-nowrap cursor-grab active:cursor-grabbing' : undefined}>
            <ComposerActionsStart className="gap-1">
              <DropdownMenu>
                <DropdownMenu.Trigger variant="ghost" size="icon-sm" aria-label={strings.add} tooltip={strings.add}>
                  <Plus />
                </DropdownMenu.Trigger>
                {/* Four short rows: this menu never scrolls, so its scroll bar is never shown. */}
                <DropdownMenu.Content side="top" align="start" className="[&_[data-slot=scroll-area-scrollbar]]:hidden">
                  <DropdownMenu.Item onClick={() => post({ type: 'add_file' })}>
                    <FileText />
                    {strings.addFile}
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
              {session && <SessionMode state={state} session={session} />}
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

/**
 * The panel's own bar: the agent and what it runs with, its state, and the panel's controls. With
 * `onBack`, the connections page is open in place of the conversation, and its way back stands
 * where the connection and its state are named.
 */
function AssistantBar({
  state,
  onSetup,
  onConnections,
  onCollapse,
  onBack,
}: {
  state: AssistantState;
  onSetup: (id: string) => void;
  onConnections: () => void;
  onCollapse?: () => void;
  onBack?: () => void;
}) {
  const { strings } = state;
  const agent = state.agents.find(entry => entry.id === state.agent);
  const name = agent?.name ?? (state.detecting ? strings.detecting : strings.noneFound);
  const badge = stateBadge(state);
  // The agent's own settings; its model is picked beside the message box, and so are the built-in agent's effort and mode.
  const settings = (state.config ?? []).filter(option => option.category !== 'model' && !(state.connect && (option.category === 'thought_level' || option.category === 'mode')));
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
  // At the bottom centre the bar's empty part drags the box; only Collapse collapses it.
  const centre = centred(state);
  return (
    <ChatShell.Bar className={centre ? 'relative' : undefined}>
      {/* Under the row: what is not a control (the gaps, the state badge) lets the press through to
          it, to drag the box. */}
      {centre && (
        <div
          aria-hidden
          data-centre-bar-drag
          className="absolute inset-0 cursor-grab rounded-lg transition-colors active:cursor-grabbing duration-fast ease-out-custom hover:bg-fill-subtle motion-reduce:transition-none"
        />
      )}
      <ChatShell.Column className={cn('flex-row items-center justify-between gap-2 py-2', centre && 'pointer-events-none relative [&_button]:pointer-events-auto')}>
        <div className="-m-0.5 flex h-8 min-w-0 flex-1 flex-wrap content-start items-center gap-x-1.5 gap-y-2 overflow-hidden p-0.5">
          {onBack ? (
            // On the connections page the page itself says what is connected and how it stands.
            <Button type="button" size="sm" variant="primary" className="shrink-0" onClick={onBack}>
              {strings.connBack}
            </Button>
          ) : connect ? (
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
          {!onBack && !nothingConnected(state) && (
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
              {/* At the bottom centre the Assistant neither docks to a side nor folds to a strip. */}
              {!centre && (
                <>
                  <DropdownMenu.Item onClick={() => post({ type: 'side', left: !state.left })}>
                    {state.left ? strings.dockRight : strings.dockLeft}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item onClick={() => post({ type: 'minimize' })}>{strings.minimize}</DropdownMenu.Item>
                </>
              )}
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
          {/* At the bottom centre the right rail's Assistant button shows and hides it: in the close
              button's place is Collapse, back to the message box. */}
          {centre ? (
            <>
              <Button type="button" size="icon-sm" variant="ghost" aria-label={strings.collapse} tooltip={strings.collapse} onClick={onCollapse}>
                <Minimize2 />
              </Button>
              <Grip strings={strings} />
            </>
          ) : (
            <Button type="button" size="icon-sm" variant="ghost" aria-label={strings.close} tooltip={strings.close} onClick={() => post({ type: 'close' })}>
              <X />
            </Button>
          )}
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
  // Before the agent has a model, the first connection there is stands for all of them. A model
  // that no connection serves (its key was taken away, and the user has not picked another) names
  // none: the button then says only that there are connections.
  const current = modelChoice(state).current;
  const used = connect.connected ? (current ? connectionOf(connect, current) : liveOf(connect)[0]) : undefined;
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
 * keeps it in the system keychain; nothing shows it again. A save is answered when the app's
 * count of answered saves moves on: with no error the key is kept and the dialog is done; with
 * one it stays, with what was typed, to say so.
 */
function KeyDialog({
  entry,
  error,
  saves,
  strings,
  onClose,
  onSaved,
}: {
  entry: Connect['keys'][number];
  error: string | null;
  saves: number;
  strings: Strings;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [draft, setDraft] = useState('');
  // The count of answered saves when Save was pressed (`null`: no save waits), and whether the
  // last save was refused. An error that was there before this save is not about this key.
  const [asked, setAsked] = useState<number | null>(null);
  const [refused, setRefused] = useState(false);
  useEffect(() => {
    if (asked === null || saves === asked) return;
    setAsked(null);
    if (error) setRefused(true);
    else onSaved();
  }, [saves, asked]);
  const title = entry.title;
  function save() {
    const key = draft.trim();
    if (!key || asked !== null) return;
    setAsked(saves);
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
          {/* Cancel stays: an answer that never comes must not keep the dialog open. */}
          <DialogCancel>{strings.cancel}</DialogCancel>
          <DialogAction disabled={!draft.trim() || asked !== null} onConfirm={save}>
            {asked !== null ? strings.working : strings.connSave}
          </DialogAction>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** An element's ref that brings it into sight when it appears and when `news` changes: the page may be scrolled away from it. */
function useInSight(news: unknown) {
  const ref = useRef<HTMLDivElement>(null);
  // A block body: scrolling can return a promise, which React would take for the effect's cleanup.
  useEffect(() => {
    ref.current?.scrollIntoView({ block: 'nearest' });
  }, [news]);
  return ref;
}

/** A key the app has just kept, said under its row: the row can be at the page's lower edge. */
function KeySaved({ strings }: { strings: Strings }) {
  const ref = useInSight(null);
  return (
    <Notice ref={ref} variant="success" icon={<Check />} role="status" className="mt-1">
      <Notice.Message>{strings.connKeySaved}</Notice.Message>
    </Notice>
  );
}

/** What the app could not do, at the top of the connections page however far down it was asked. */
function PageError({ text }: { text: string }) {
  const ref = useInSight(text);
  return (
    <Notice ref={ref} variant="destructive" role="alert">
      <Notice.Message>{text}</Notice.Message>
    </Notice>
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
function ConnectionsPage({ state, connect }: { state: AssistantState; connect: Connect }) {
  const { strings } = state;
  const [keyFor, setKeyFor] = useState<string | null>(null);
  // The key whose save the app has just answered as kept: its row says so until something else is done here.
  const [savedNow, setSavedNow] = useState<string | null>(null);
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
  const askKey = (id: string) => {
    setSavedNow(null);
    setKeyFor(id);
  };
  return (
    <div className="flex min-w-0 flex-col gap-3 py-2">
      {/* The way back to the conversation is in the bar (AssistantBar's `onBack`). */}
      <h2 className="min-w-0 truncate text-sm font-medium">{strings.connTitle}</h2>
      {/* What the app could not do outside the key dialog and the address form, which say it
          themselves: a sign-in or sign-out the agent refused, a keychain that would not read a key
          or remove one. */}
      {connect.error && !keyEntry && !adding && <PageError text={connect.error} />}
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
      {connect.keys.map(key => {
        // A key the app has just kept is saved before the agent's next list says so.
        const saved = key.saved || savedNow === key.id;
        return (
          <ConnRow
            key={key.id}
            entry={key}
            note={saved && key.shadowed ? strings.connShadowed : undefined}
            status={saved ? standing(strings.connKeySaved, 'success') : undefined}
            actions={
              saved ? (
                <>
                  {!key.shadowed && usedOrUse(key)}
                  <Button type="button" size="sm" onClick={() => askKey(key.id)}>
                    {strings.connKeyUpdate}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => {
                      setSavedNow(null);
                      post({ type: 'conn_key_remove', provider: key.id });
                    }}
                  >
                    {strings.connRemove}
                  </Button>
                </>
              ) : (
                <Button type="button" size="sm" onClick={() => askKey(key.id)}>
                  {strings.connKeyAdd}
                </Button>
              )
            }
          >
            {/* An update leaves the row as it was: the save is said under it, where the dialog was opened. */}
            {savedNow === key.id && <KeySaved strings={strings} />}
          </ConnRow>
        );
      })}
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
      {keyEntry && (
        <KeyDialog
          entry={keyEntry}
          error={connect.error}
          saves={connect.keySaves}
          strings={strings}
          onClose={() => setKeyFor(null)}
          onSaved={() => {
            setSavedNow(keyEntry.id);
            setKeyFor(null);
          }}
        />
      )}
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
  // taken away behind the user's back: they go to the conversation with the button in the bar.
  const [connOpen, setConnOpen] = useState(false);
  // At the bottom centre: whether the conversation is open above the message box.
  const [centreOpen, setCentreOpen] = useState(false);
  const none = nothingConnected(state);
  useEffect(() => {
    // At the bottom centre a row over the message box offers the connections instead.
    if (none && !centred(state)) setConnOpen(true);
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
  // At the bottom centre the same body (bar, conversation, dock) opens above the centre's own message box.
  const centre = centred(state);
  // Back to the message box there: the bar's Collapse and a press on the bar's empty part.
  const collapse = () => {
    setCentreOpen(false);
    setConnOpen(false);
  };
  const shell = (
      <ChatShell
        className="h-full"
        // There the veil above the message box ramps to the header's surface (ChatShell's `--chat-surface`).
        style={centre ? ({ ...ROOM, '--chat-surface': 'var(--color-card)' } as CSSProperties) : ROOM}
        scroller={{ autoScroll: true, defaultScrollPosition: 'end' }}
      >
        <AssistantBar
          state={state}
          onSetup={setSetup}
          onConnections={openConn}
          onCollapse={collapse}
          // With nothing connected the page is the panel, and there is no conversation to go back to.
          onBack={connect && showConn && connect.connected ? () => setConnOpen(false) : undefined}
        />
        {connect && showConn && (
          <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
            {/* The bar's New chat asks here too: the conversation it is asked in is out of sight. */}
            {confirmNew}
            <ConnectionsPage state={state} connect={connect} />
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
                          own={Boolean(connect)}
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
                {!centre && <AssistantComposer state={state} onConnections={openConn} />}
              </ChatShell.Column>
            </ChatShell.Dock>
          </ChatShell.Viewport>
        </ChatShell.Stage>
      </ChatShell>
  );
  if (centre)
    return (
      <Centre
        state={state}
        openConn={openConn}
        body={shell}
        conn={connect ? listed(connect) && connOpen : false}
        none={none}
        shown={centreOpen}
        setShown={setCentreOpen}
        collapse={collapse}
      />
    );
  return (
    <div className="h-dvh overflow-hidden bg-background" style={{ borderRadius: state.radius }}>
      <FilePreviewDialog file={state.filePreview ?? null} />
      {shell}
    </div>
  );
}

// ---- At the bottom centre of the window (INT-1500) ----
// The host puts the page there with `centre` (the most height it may take). At rest the page is the
// message box and, above it, one line: the state of the agent's activity, as a row of Mastra's
// Activity (PreviewLine). Only that row, or Expand over a row that needs the user, opens the panel's
// own body (bar, conversation, dock) above the message box, in Mastra's ComposerHeader. What needs
// the user (an approval, an agent that is not running or must sign in, a sign-in under way, a turn
// that failed on its connection, files dragged over) floats over the message box in an overlay
// header while the body is closed, and is the last row above it while open (a failed turn is then at
// the conversation's end).

/** Whether the host shows the page at the bottom centre of the window. */
const centred = (state: AssistantState | null) => typeof state?.centre === 'number';

// A desktop page in a small box: Mastra's phone layouts (the model picker's bottom sheet below 640)
// do not apply at the bottom centre. The page's width there is the box's, so its width query never
// matches; in the side panel queries are answered as they are.
let centreNow = false;
const realMatchMedia = window.matchMedia.bind(window);
window.matchMedia = (query: string) => (centreNow && /max-width:\s*639px/.test(query) ? realMatchMedia('(max-width: 0px)') : realMatchMedia(query));

const plain = (text: string) => text.replace(/\*+|__|`/g, '').replace(/\s*\n+\s*/g, ' ').trim();

let measure: CanvasRenderingContext2D | null = null;

/** A placeholder that fits on one line of the message box, cut short with an ellipsis: a textarea's placeholder takes no `text-overflow`. */
function oneLine(box: HTMLTextAreaElement | null, text: string) {
  measure ??= document.createElement('canvas').getContext('2d');
  if (!box || !measure) return text;
  const style = getComputedStyle(box);
  const room = box.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  measure.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  if (!(room > 0) || measure.measureText(text).width <= room) return text;
  let end = text.length;
  while (end > 0 && measure.measureText(`${text.slice(0, end)}…`).width > room) end -= 1;
  return `${text.slice(0, end).trimEnd()}…`;
}

/**
 * What the line over the closed message box shows: the state of the agent's activity, as a row of
 * Mastra's Activity. Never the words it streams: in CAD the result shows in the view, and what one
 * wants from the line is what the agent is at. While it works, the most pressing first: an answer it
 * waits for, the tool or the check it runs, else working (with its plan's step). At rest: a turn
 * that failed, a turn done (what it changed, or its reply's first line), or which agent this is.
 */
interface Line {
  kind: 'empty' | 'done' | 'note' | 'working' | 'tool' | 'check' | 'waiting' | 'error';
  icon: LucideIcon;
  /** What, in plain words: the action, the state. */
  label?: string;
  /** What it acts on, or more of it: cut to one line as Mastra cuts a preview. */
  detail?: string;
  /** The agent is at it: the row shimmers. */
  live: boolean;
  /** Where the agent is in its plan ("2/3"), while it follows one. */
  step?: string;
  /** A turn that failed on its connection: what failed, for its ways on (ProblemWays). */
  problem?: TurnProblem;
}

// Mastra's agent indicator's poses held still, as its own rows use them.
const IdleGlyph = agentGlyph('idle');
const DoneGlyph = agentGlyph('done');
const NoticeGlyph = agentGlyph('notice');
const FailedGlyph = agentGlyph('error');

/** A tool Mastra's presenter does not know by name, by the kind the host gives it (an ACP tool kind). */
const KIND_TOOLS: Record<string, string> = { read: 'read_file', edit: 'edit_file', delete: 'delete_file', search: 'grep', execute: 'execute_command', fetch: 'web_fetch' };

/** A tool's own name in words, without the server an MCP tool is named after: "intcad-app_export" is "Export". */
const toolWords = (name: string) => humanize(name.replace(/^mcp__.+?__/, '').replace(/^\w+-[\w-]+?_(?=\w)/, ''));

/** A running tool as the conversation's row names it (ToolRow): its icon, the action, what it acts on. */
function toolLine(tool: Tool, own: boolean, step: string | undefined): Line {
  const { name, detail } = nameOf(tool.title);
  const args = (own ? argsOf(tool.input) : undefined) ?? {};
  const named = presentTool(name, args);
  const known = named.pose !== 'tool';
  const kindTool = known ? undefined : KIND_TOOLS[tool.toolKind];
  const byKind = kindTool ? presentTool(kindTool, args) : undefined;
  const line: Line = { kind: 'tool', icon: (byKind ?? named).icon, live: true, step };
  // The app's own words for what its tool does, in the user's language.
  if (tool.words) return { ...line, label: tool.words };
  if (known) return { ...line, label: named.description ?? named.label, detail: named.description ? undefined : (detail ?? named.detail) };
  // What it acts on: the title's own detail, else the first of its arguments that is a word or a path.
  const first = Object.values(args).find((value): value is string => typeof value === 'string' && value.trim() !== '');
  return byKind ? { ...line, label: byKind.label, detail: detail ?? byKind.detail ?? toolWords(name) } : { ...line, label: toolWords(name), detail: detail ?? first };
}

function previewOf(state: AssistantState): Line {
  const { items, strings } = state;
  const busy = state.state === 'busy';
  // This turn: everything since the last message sent.
  const turn = items.slice(items.findLastIndex(item => item.kind === 'user') + 1);
  // What the agent did last: a tool, a check, something it said or thought, what the app said.
  const now = turn.findLast(item => item.kind === 'tool' || item.kind === 'check' || item.kind === 'notice' || (item.kind === 'text' && item.text.trim() !== ''));
  const steps = turn.findLast((item): item is Of<'plan'> => item.kind === 'plan')?.steps ?? [];
  const at = busy ? steps.findIndex(step => step.status === 'in_progress') : -1;
  const step = at >= 0 ? `${at + 1}/${steps.length}` : undefined;
  // An answer the agent waits for, whatever the state says (the state's badge puts it first too).
  const asked = turn.findLast((item): item is Approval | Of<'permission'> => (item.kind === 'approval' && item.answer === null) || (item.kind === 'permission' && item.answered === null));
  if (asked || state.waiting > 0) {
    return { kind: 'waiting', icon: ShieldCheck, label: strings.waiting, detail: asked && (asked.kind === 'approval' ? asked.title : asked.heading), live: false, step };
  }
  if (busy) {
    if (now?.kind === 'tool' && (now.status === 'running' || now.status === 'pending')) return toolLine(now, Boolean(state.connect), step);
    if (now?.kind === 'check' && (now.status === 'running' || now.status === 'pending')) {
      return { kind: 'check', icon: ClipboardCheck, label: strings.dfmCheckTitle, detail: strings.dfmRunning, live: true, step };
    }
    // Thinking, or writing its reply: working, on its plan's step if it follows one.
    return { kind: 'working', icon: Sparkles, label: strings.working, detail: steps[at]?.text, live: true, step };
  }
  if (now?.kind === 'notice' && now.error) return { kind: 'error', icon: FailedGlyph, detail: plain(now.text), live: false, problem: now.problem ?? undefined };
  if (now?.kind === 'check' && now.status === 'error') return { kind: 'error', icon: FailedGlyph, detail: strings.dfmError, live: false };
  if (now?.kind === 'notice') return { kind: 'note', icon: NoticeGlyph, detail: plain(now.text), live: false };
  if (items.some(item => item.kind === 'user')) {
    // What the turn changed (the app counts it only when there is something), or its reply's first line.
    const changed = turn.findLast((item): item is Of<'changes'> => item.kind === 'changes' && !item.undone);
    const said = turn.findLast((item): item is Of<'text'> => item.kind === 'text' && !item.thought && item.text.trim() !== '');
    const first = said?.text.split('\n').find(row => row.trim() !== '');
    return { kind: 'done', icon: DoneGlyph, label: strings.dfmDone, detail: changed?.label ?? (first && plain(first)), live: false };
  }
  const agent = state.agents.find(entry => entry.id === state.agent);
  return { kind: 'empty', icon: IdleGlyph, label: agent?.name ?? strings.noneFound, detail: agent ? stateBadge(state).label : undefined, live: false };
}

/**
 * The line over the closed message box: a row of Mastra's Activity in its own looks, shimmering while
 * the agent is at it and still at rest, in the body's size (the header's small text was hard to read).
 * The whole row opens the conversation, with Mastra's disclosure chevron at its end.
 */
function PreviewLine({ line, open, strings, onExpand }: { line: Line; open: boolean; strings: Strings; onExpand: () => void }) {
  const Icon = line.icon;
  return (
    <Activity
      open={false}
      onOpenChange={next => next && onExpand()}
      // Under the open conversation the line is out of sight, and still.
      status={line.kind === 'error' ? 'error' : line.live && !open ? 'running' : 'idle'}
      className="w-full"
    >
      <ActivityTrigger title={strings.expand}>
        <ActivityHeader>
          <ActivityIcon className={cn(line.kind === 'waiting' && 'text-warning-foreground')}>
            <Icon aria-hidden />
          </ActivityIcon>
          {/* A label alone takes the row, as in Mastra's ActivityHeadline; beside a detail it keeps Mastra's cap. */}
          {line.label && <ActivityLabel className={cn('text-body', !line.detail && 'max-w-none min-w-0 shrink')}>{line.label}</ActivityLabel>}
          {line.step && (
            <Badge size="sm" className="shrink-0 tabular-nums">
              {line.step}
            </Badge>
          )}
          {line.detail && <ActivityDetail className="text-body">{messagePreview(line.detail)}</ActivityDetail>}
          <ActivitySpacer />
          <ActivityDisclosure />
        </ActivityHeader>
      </ActivityTrigger>
    </Activity>
  );
}

/** The grip the box at the bottom centre is dragged by, at its top right corner (the closed line's
 *  end, the open bar's end), as Mastra's MetricsCard.DragHandle draws one. A click does nothing. */
function Grip({ strings }: { strings: Strings }) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      data-centre-grip
      className="shrink-0 cursor-grab active:cursor-grabbing"
      aria-label={strings.moveBox}
      tooltip={strings.moveBox}
    >
      <GripVertical />
    </Button>
  );
}

/** The model picker's header (Mastra's `header` slot): the connection the model runs on, and the way to sign in or add one. */
function PickerConnection({ via, strings, onConnections }: { via: Connection | null | undefined; strings: Strings; onConnections: () => void }) {
  const account = via && 'account' in via && via.account ? via.account : null;
  return (
    <div className="flex min-w-0 items-center justify-between gap-2 px-2 pt-1 pb-2">
      <Txt as="span" variant="caption" tone="muted" className="min-w-0 truncate">
        {via ? `${via.name}${account ? ` · ${account}` : ''}` : strings.connTitle}
      </Txt>
      <Button
        type="button"
        size="sm"
        icon={<KeyRound aria-hidden />}
        onClick={event => {
          onConnections();
          // Closes the picker, as its own "+" does.
          event.currentTarget.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        }}
      >
        {strings.connTitle}
      </Button>
    </div>
  );
}

/** A waiting approval or permission with its answers, small enough for the header over the message box. */
function WaitingRow({ item, strings }: { item: Approval | Of<'permission'>; strings: Strings }) {
  return (
    <ComposerHeaderRow>
      <ComposerHeaderContent className="flex min-w-0 flex-col gap-2 text-foreground">
        <span className="flex min-w-0 items-start gap-2">
          <Badge variant="warning" emphasis="subtle" size="sm" icon={<ShieldCheck aria-hidden />}>
            {strings.approval}
          </Badge>
          <span className="line-clamp-2 min-w-0">{item.kind === 'approval' ? item.title : item.heading}</span>
        </span>
        {item.kind === 'approval' ? (
          <ApprovalChoices approval={item} strings={strings} />
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            {item.options.map(option => (
              <Button
                key={option.id}
                type="button"
                size="sm"
                variant={option.kind === 'allow_once' ? 'primary' : 'default'}
                onClick={() => post({ type: 'permission', index: item.index, option: option.id })}
              >
                {option.name}
              </Button>
            ))}
          </div>
        )}
      </ComposerHeaderContent>
    </ComposerHeaderRow>
  );
}

/**
 * What opens over the page, and whether a press outside the page closes it (`blur`) as a press
 * outside it within the page would: menus, selects and the model and effort pickers do; a dialog
 * keeps what was typed into it (a key copied from another app) and closes from its own buttons.
 */
const POPUPS = [
  { selector: '[data-slot="popover-content"]', blur: true },
  { selector: '[data-slot="drawer-content"]', blur: true },
  { selector: '[role="menu"]', blur: true },
  { selector: '[data-slot="select-content"][data-open]', blur: true },
  { selector: '[data-slot="dialog-content"]', blur: false },
];
/** Every popup that is open. */
const OPEN = POPUPS.map(entry => entry.selector).join(', ');
/** The popups a press outside the page closes. */
const ON_BLUR = POPUPS.filter(entry => entry.blur).map(entry => entry.selector).join(', ');
const NATIVE_PORTALS = `${OPEN}, [role="tooltip"], [data-slot="alert-dialog-content"]`;

/** A computed length we can use without guessing at unresolved units or expressions. */
function paintPixels(value: string) {
  return /^-?(?:\d+\.?\d*|\.\d+)px$/.test(value) ? parseFloat(value) : NaN;
}

/** Conservative CSS-pixel outsets on the four sides. Unknown effects keep the border shape. */
function paintOverflow(surface: HTMLElement, style: CSSStyleDeclaration) {
  let left = 0, top = 0, right = 0, bottom = 0;
  const outset = (x: number, y: number, extent: number) => {
    left = Math.max(left, extent - x);
    right = Math.max(right, extent + x);
    top = Math.max(top, extent - y);
    bottom = Math.max(bottom, extent + y);
  };
  // Remove serialized color functions before splitting the list: their comma-separated
  // channels aren't additional shadows. An inset shadow does not extend the border box.
  for (const shadow of style.boxShadow.replace(/[\w-]+\([^()]*\)/g, '').split(',')) {
    const words = shadow.trim().split(/\s+/);
    if (words.includes('inset') || words.length < 2 || words.length > 4) continue;
    const [x, y, blur = 0, spread = 0] = words.map(paintPixels);
    // Box shadow's standard deviation is half its blur radius. Include three deviations.
    if ([x, y, blur, spread].every(Number.isFinite)) outset(x, y, blur * 1.5 + spread);
  }
  const outline = paintPixels(style.outlineWidth) + paintPixels(style.outlineOffset);
  if (style.outlineStyle !== 'none' && Number.isFinite(outline)) outset(0, 0, outline);

  const box = surface.querySelector('[data-slot="composer-box"]');
  if (box) {
    const boxStyle = getComputedStyle(box);
    // The current composer clips its spotlight to the inner box. If it no longer clips,
    // an active blur() can extend at most this far beyond the enclosing ring.
    if (boxStyle.overflowX === 'visible' || boxStyle.overflowY === 'visible') {
      const spotlight = getComputedStyle(box, '::before');
      const blur = /^blur\(([^()]+)\)$/.exec(spotlight.filter)?.[1];
      const radius = blur ? paintPixels(blur) : NaN;
      if (spotlight.display !== 'none' && Number(spotlight.opacity) > 0 && Number.isFinite(radius)) outset(0, 0, radius * 3);
    }
  }
  return { left, top, right, bottom };
}

/** Border boxes that paint the centred composer, with CSS ellipses in TL/TR/BR/BL order. */
function paintedFootprint(element: HTMLElement) {
  const surfaces = Array.from(
    element.querySelectorAll<HTMLElement>('[data-slot="composer-ring"], [data-slot="composer-header-surface"]'),
  );
  return surfaces.flatMap(surface => {
    const rect = surface.getBoundingClientRect();
    const style = getComputedStyle(surface);
    const header = surface.closest('[data-slot="composer-header"]');
    if (!rect.width || !rect.height || style.visibility !== 'visible' || style.display === 'none' ||
        header?.hasAttribute('data-centre-wait')) return [];
    const clip = surface.parentElement?.getBoundingClientRect();
    const bottom = header && clip ? Math.min(clip.bottom, rect.bottom) : rect.bottom;
    if (bottom <= rect.top) return [];
    const radius = (value: string) => {
      const [x, y = x] = value.trim().split(/\s+/);
      const resolve = (length: string, extent: number) => {
        if (!/^(?:\d+\.?\d*|\.\d+)(?:px|%)$/.test(length)) return NaN;
        return length.endsWith('%') ? parseFloat(length) * extent / 100 : parseFloat(length);
      };
      return [resolve(x, rect.width), resolve(y, rect.height)];
    };
    const radii = [
      ...radius(style.borderTopLeftRadius), ...radius(style.borderTopRightRadius),
      ...radius(style.borderBottomRightRadius), ...radius(style.borderBottomLeftRadius),
    ];
    if (!radii.every(Number.isFinite)) return [];
    const paint = paintOverflow(surface, style);
    const clipped = bottom < rect.bottom;
    const edges = [paint.left, paint.top, paint.right, paint.top, paint.right, paint.bottom, paint.left, paint.bottom];
    const outerRadii = radii.map((value, index) => clipped && index >= 4 ? 0 : value + edges[index]);
    return [[rect.left - paint.left, rect.top - paint.top, rect.right + paint.right,
      clipped ? bottom : bottom + paint.bottom, ...outerRadii]];
  });
}

type X11ShapeHost = Window & {
  __assistantX11Shape?: true;
  __assistantX11Unmasked?: (token: number) => void;
};

/** Closes the page's open menus, selects and pickers, as a press outside them would; its dialogs stay. */
function closePopups() {
  if (!document.querySelector(ON_BLUR)) return;
  document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
  (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

/** The headers over the message box: the line, a row that needs the user, the opened body. */
const HEADERS = ':scope > [data-slot="composer"] > [data-slot="composer-header"]';
/** The parts of the box at the bottom centre a press on moves it: no control, no words. */
const EMPTY_OF_BOX = '[data-centre-bar-drag], [data-slot^="composer-actions"]';
/** The box's grip, which drags it from anywhere on it, its icon too. */
const GRIP = '[data-centre-grip]';

/** Where a header over the message box ends at the top once its opening transition is over. */
function topOf(header: Element) {
  const rect = header.getBoundingClientRect();
  const surface = header.querySelector<HTMLElement>('[data-slot="composer-header-surface"]');
  return surface ? Math.min(rect.top, rect.bottom - surface.offsetHeight) : rect.top;
}

/** How tall the page is: the message box and every header over it, each as tall as it is opening to. */
function heightOf(element: HTMLElement) {
  const box = element.getBoundingClientRect();
  let top = box.top;
  for (const header of element.querySelectorAll(HEADERS)) top = Math.min(top, topOf(header));
  return Math.ceil(box.bottom - top);
}

function Centre({
  state,
  openConn,
  body,
  conn,
  none,
  shown,
  setShown,
  collapse,
}: {
  state: AssistantState;
  openConn: () => void;
  body: ReactNode;
  conn: boolean;
  none: boolean;
  shown: boolean;
  setShown: (shown: boolean) => void;
  collapse: () => void;
}) {
  const { strings } = state;
  // The body's height: the host's most, less everything else the page shows.
  const [room, setRoom] = useState(300);
  // The line over the closed box, as tall as it is drawn.
  const [lineHeight, setLineHeight] = useState(28);
  const measured = useRef<HTMLDivElement>(null);
  const most = useRef(500);
  most.current = state.centre ?? 500;
  centreNow = true;
  // Asks the host for this much height at once and keeps it asked for until called with 0 (set by the effect below).
  const grow = useRef<(to: number) => void>(() => {});
  // The body's room is measured before the page is drawn: after each render, so the body opens at
  // its height and not a frame later, and whenever the view or the page changes size (`now`: from
  // outside React, drawn at once), so a smaller window keeps the bar with Collapse in sight. When the
  // page has too little room the conversation gives way first, down to the bar; the rows that need
  // the user and the message box keep their height.
  // The connections page opens the body too.
  const open = conn || shown;
  const fitRoom = useRef<(now: boolean) => void>(() => {});
  fitRoom.current = now => {
    const element = measured.current;
    const inBody = element?.querySelector<HTMLElement>('[data-centre-body]');
    const fold = element?.querySelector<HTMLElement>('[data-centre-fold]');
    if (!element || !inBody || !fold) return;
    const line = element.querySelector<HTMLElement>('[data-centre-line]');
    if (line && line.offsetHeight > 0 && line.offsetHeight !== lineHeight) setLineHeight(line.offsetHeight);
    const bar = inBody.firstElementChild?.firstElementChild;
    // Everything but the fold (the conversation open, the line closed, or between them while it
    // moves) stays. Closed with a row floating over the box as well, the room measured last is kept.
    if (!open && element.querySelectorAll(HEADERS).length > 1) return;
    const others = heightOf(element) - fold.offsetHeight;
    const next = Math.max(bar instanceof HTMLElement ? bar.offsetHeight : 0, Math.floor(most.current - others - 2));
    if (Math.abs(next - room) <= 1) return;
    if (now) flushSync(() => setRoom(next));
    else setRoom(next);
  };
  useLayoutEffect(() => fitRoom.current(false));
  // The page tells the host how tall it is; the host keeps it within `centre`. While a popup is open
  // (model picker, mode menu, a dialog) the page asks for all the room there is, from the press that
  // opens it, so the popup has room when it appears.
  useEffect(() => {
    const root = document.documentElement;
    root.classList.add('centre');
    const element = measured.current;
    if (!element) return;
    let menu = false;
    let check = 0;
    const native = window as X11ShapeHost;
    let nativePopup: number | null = null;
    let nextPopup = 0;
    let unmasked: number | null = null;
    let popupSince = 0;
    let retryPopup = 0;
    let sentFootprint = '';
    // The host resizes the native view a frame or two after it is asked, so a view resized while
    // something animates jumps. Taller is asked at once, so what opens has its room as it is drawn;
    // shorter only once the page has been still for a moment, never under a closing animation.
    // Never more than `centre`, which is all the host gives.
    let told = -1;
    let shrinkTo = -1;
    let shrink = 0;
    // A height asked for ahead of what needs it (the conversation about to open): not given back before.
    let floor = 0;
    const wanted = () => Math.min(most.current, Math.max(menu ? most.current : heightOf(element), floor));
    const send = (next: number) => {
      window.clearTimeout(shrink);
      shrinkTo = -1;
      if (next === told) return;
      told = next;
      post({ type: 'size', height: next });
    };
    // Nothing is drawn taller than the view. While the view grows to what the page asked for, a
    // header whose top it would cut off, and any popup, waits unseen, and is shown in the frame the
    // view has its room. It runs before the page is drawn: from the resize event, the observers'
    // callbacks and the press that opens a popup. A host that never grows hides nothing past a second.
    let since = 0;
    let late = 0;
    const fit = () => {
      const view = window.innerHeight;
      let growing = view + 1 < Math.min(told, most.current);
      if (!growing) since = 0;
      else if (!since) {
        since = performance.now();
        window.clearTimeout(late);
        late = window.setTimeout(fit, 1000);
      } else if (performance.now() - since > 990) growing = false;
      for (const header of element.querySelectorAll(HEADERS)) header.toggleAttribute('data-centre-wait', growing && topOf(header) < -1);
      const portals = document.querySelectorAll(native.__assistantX11Shape ? NATIVE_PORTALS : OPEN);
      if (native.__assistantX11Shape) {
        if (!menu && !portals.length) nativePopup = null;
        else if (nativePopup === null) {
          nativePopup = ++nextPopup;
          popupSince = performance.now();
        }
      }
      const awaitingUnmask = nativePopup !== null && nativePopup !== unmasked;
      const lostAck = awaitingUnmask && performance.now() - popupSince >= 800;
      for (const portal of portals) {
        portal.toggleAttribute('data-centre-wait', growing || (awaitingUnmask && !lostAck));
      }
      if (native.__assistantX11Shape) {
        const surfaces = nativePopup === null ? paintedFootprint(element) : [];
        const footprint = JSON.stringify({
          type: 'painted-footprint', width: window.innerWidth, height: window.innerHeight, popup: nativePopup, surfaces,
        });
        if (footprint !== sentFootprint) {
          try {
            window.ipc?.postMessage(footprint);
            sentFootprint = footprint;
          } catch {
            // A send failure must not stop the retry timer or leave the portal hidden.
          }
        }
        if (!awaitingUnmask) {
          window.clearTimeout(retryPopup);
          retryPopup = 0;
        } else if (!retryPopup) retryPopup = window.setTimeout(() => {
          // Keep the request rectangular and resend even after the portal has failed open.
          retryPopup = 0;
          sentFootprint = '';
          fit();
        }, lostAck ? 400 : 100);
      }
    };
    if (native.__assistantX11Shape) {
      native.__assistantX11Unmasked = token => {
        if (token !== nativePopup) return;
        unmasked = token;
        fit();
      };
    }
    grow.current = to => {
      floor = Math.min(to, most.current);
      if (floor > told) send(floor);
      fit();
    };
    const tell = () => {
      fitRoom.current(true);
      const next = wanted();
      // A pixel or two less is rounding, not a change: shrinking for it, then growing back, made the
      // view flicker. Any growth is kept, or the top of the page would be cut off.
      if (next < told && told - next <= 2) {
        window.clearTimeout(shrink);
        shrinkTo = -1;
      } else if (next >= told) send(next);
      else if (next !== shrinkTo) {
        shrinkTo = next;
        window.clearTimeout(shrink);
        shrink = window.setTimeout(() => {
          send(wanted());
          fit();
        }, 250);
      }
      fit();
    };
    // Headers open and close with a transition that changes no DOM: keep measuring until it settles.
    // A timer, not animation frames: those pause where the page is not drawn.
    let until = 0;
    let frame = 0;
    const follow = () => {
      tell();
      if (performance.now() < until) frame = window.setTimeout(follow, 16);
    };
    const settleSize = () => {
      until = performance.now() + 700;
      window.clearTimeout(frame);
      frame = window.setTimeout(follow, 0);
    };
    const popup = () => Boolean(document.querySelector(OPEN));
    const settle = () => {
      window.clearTimeout(check);
      check = window.setTimeout(() => {
        if (menu && !popup()) {
          menu = false;
          tell();
        }
      }, 400);
    };
    // The line's own row (Mastra's disclosure, `aria-expanded`) opens the conversation, which asks for its room itself.
    const down = (event: PointerEvent) => {
      const at = event.target as Element | null;
      if (!menu && at?.closest('[aria-haspopup], [aria-expanded]') && !at.closest('[data-centre-line]')) {
        menu = true;
        tell();
        settle();
      }
    };
    // A press outside the page (the model, the window) closes its menus, as one inside would;
    // switching to another app to copy a key does not close the dialog it goes into.
    const blur = () => closePopups();
    // A mode picked from its menu closes the menu.
    const picked = (event: globalThis.MouseEvent) => {
      if ((event.target as Element | null)?.closest('[role="menuitemradio"]')) window.setTimeout(closePopups, 120);
    };
    // What changed is measured before it is drawn (a mutation observer's callback runs before the
    // page is painted), then followed while its transition settles.
    const watch = new MutationObserver(() => {
      if (popup()) menu = true;
      tell();
      if (!menu) settleSize();
      settle();
    });
    watch.observe(document.body, native.__assistantX11Shape
      ? { childList: true, subtree: true, attributes: true, attributeFilter: ['data-busy'] }
      : { childList: true, subtree: true });
    const effectChanged = () => { fit(); settleSize(); };
    const ringHover = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.matches('[data-slot="composer-ring"]')) effectChanged();
    };
    const forcedColors = matchMedia('(forced-colors: active)');
    const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
    if (native.__assistantX11Shape) {
      element.addEventListener('pointerenter', ringHover, true);
      element.addEventListener('pointerleave', ringHover, true);
      forcedColors.addEventListener('change', effectChanged);
      reducedMotion.addEventListener('change', effectChanged);
    }
    document.addEventListener('pointerdown', down, true);
    document.addEventListener('click', picked, true);
    window.addEventListener('blur', blur);
    window.addEventListener('resize', tell);
    tell();
    // The page, and the view itself (the root is as tall as the view): its observer is told of a
    // new size in the frame it is drawn at, where the resize event can come a frame later.
    const observer = new ResizeObserver(tell);
    observer.observe(element);
    observer.observe(root);
    return () => {
      root.classList.remove('centre');
      centreNow = false;
      observer.disconnect();
      watch.disconnect();
      document.removeEventListener('pointerdown', down, true);
      document.removeEventListener('click', picked, true);
      window.removeEventListener('blur', blur);
      window.removeEventListener('resize', tell);
      window.clearTimeout(check);
      window.clearTimeout(frame);
      window.clearTimeout(shrink);
      window.clearTimeout(late);
      window.clearTimeout(retryPopup);
      if (native.__assistantX11Shape) {
        element.removeEventListener('pointerenter', ringHover, true);
        element.removeEventListener('pointerleave', ringHover, true);
        forcedColors.removeEventListener('change', effectChanged);
        reducedMotion.removeEventListener('change', effectChanged);
      }
      delete native.__assistantX11Unmasked;
    };
  }, []);
  // A press on the grip or an empty part of the box (the gap in the message box's bottom row; open,
  // the bar's empty part) that goes sideways moves the box along the bottom of the window; the host keeps it
  // there. A press that does not go a few points keeps its meaning, and the click a move ends in is
  // not one. Screen units, as the view moves under the pointer meanwhile.
  useEffect(() => {
    const element = measured.current;
    if (!element) return;
    let press: { id: number; x: number; dx: number; moving: boolean } | null = null;
    const swallow = (event: Event) => event.stopPropagation();
    const down = (event: PointerEvent) => {
      if (event.button !== 0 || !(event.target instanceof Element)) return;
      const at = event.target.closest(GRIP) ?? (event.target.matches(EMPTY_OF_BOX) ? event.target : null);
      if (!at) return;
      press = { id: event.pointerId, x: event.screenX, dx: 0, moving: false };
      at.setPointerCapture(event.pointerId);
    };
    const move = (event: PointerEvent) => {
      if (press?.id !== event.pointerId) return;
      press.dx = event.screenX - press.x;
      if (!press.moving && Math.abs(press.dx) < 4) return;
      press.moving = true;
      post({ type: 'move', dx: press.dx });
    };
    const up = (event: PointerEvent) => {
      if (press?.id !== event.pointerId) return;
      const { dx, moving } = press;
      press = null;
      if (!moving) return;
      post({ type: 'move', dx, done: true });
      window.addEventListener('click', swallow, { capture: true, once: true });
      window.setTimeout(() => window.removeEventListener('click', swallow, { capture: true }));
    };
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerup', up);
    element.addEventListener('pointercancel', up);
    element.addEventListener('lostpointercapture', up);
    return () => {
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', up);
      element.removeEventListener('lostpointercapture', up);
    };
  }, []);
  // The conversation opens only once the view has its height: opened into a view still growing, it
  // would be cut off. It waits for the view to be that tall, not for a time; a host that never grows
  // has it opened after a second all the same.
  const roomFirst = (then: () => void) => {
    const ready = () => window.innerHeight >= most.current - 1;
    if (ready()) {
      then();
      return;
    }
    grow.current(most.current);
    const go = (anyway: boolean) => {
      if (!anyway && !ready()) return;
      window.removeEventListener('resize', resized);
      window.clearTimeout(late);
      grow.current(0);
      then();
    };
    const resized = () => go(false);
    window.addEventListener('resize', resized);
    const late = window.setTimeout(() => go(true), 1000);
  };
  const connections = () => roomFirst(openConn);
  const preview = previewOf(state);
  const agent = state.agents.find(entry => entry.id === state.agent);
  const badge = stateBadge(state);
  const waitingItem = [...state.items]
    .reverse()
    .find((item): item is Approval | Of<'permission'> => (item.kind === 'approval' && item.answer === null) || (item.kind === 'permission' && item.answered === null));
  const stopped = Boolean(state.agent) && !state.running;
  const signIn = state.state === 'auth';
  // A sign-in under way goes on on the connections page (a code to paste, a browser to come back
  // from): closed, or in the conversation, the way back there stays in sight.
  const login = conn ? null : (state.connect?.login ?? null);
  // A turn that failed on its connection, with its ways on (sign in again, the connections, send it
  // again): closed, it is the row over the box; open, the conversation shows it at its end.
  const failed = !open && preview.kind === 'error' && preview.problem && state.connect ? { problem: preview.problem, connect: state.connect } : null;
  const attention = state.dragging ? (
    <ComposerHeaderRow>
      <ComposerHeaderContent className="text-foreground">{strings.dropHint}</ComposerHeaderContent>
    </ComposerHeaderRow>
  ) : waitingItem ? (
    <WaitingRow item={waitingItem} strings={strings} />
  ) : login ? (
    <ComposerHeaderRow>
      <Badge variant="info" emphasis="subtle" size="sm" className="shrink-0 whitespace-nowrap">
        {strings.connSigningIn}
      </Badge>
      <ComposerHeaderContent className="truncate text-foreground">{state.connect?.plans.find(plan => plan.id === login.id)?.name ?? login.id}</ComposerHeaderContent>
      <ComposerHeaderActions>
        <Button type="button" size="sm" variant="primary" icon={<KeyRound aria-hidden />} onClick={connections}>
          {strings.connTitle}
        </Button>
      </ComposerHeaderActions>
    </ComposerHeaderRow>
  ) : stopped || signIn || (none && !conn) ? (
    <ComposerHeaderRow>
      <Badge variant={badge.variant} emphasis="subtle" size="sm" className="shrink-0 whitespace-nowrap">
        {badge.label}
      </Badge>
      <ComposerHeaderContent className="truncate text-foreground">{agent?.name ?? strings.noneFound}</ComposerHeaderContent>
      <ComposerHeaderActions>
        {stopped && (
          <Button type="button" size="sm" variant="primary" onClick={() => post({ type: 'connect' })}>
            {state.state === 'failed' ? strings.retry : strings.connect}
          </Button>
        )}
        {none && !stopped && (
          <Button type="button" size="sm" variant="primary" icon={<KeyRound aria-hidden />} onClick={connections}>
            {strings.connTitle}
          </Button>
        )}
      </ComposerHeaderActions>
    </ComposerHeaderRow>
  ) : failed ? (
    <ComposerHeaderRow tone="destructive" className="flex-nowrap items-start">
      <FailedGlyph aria-hidden className="mt-0.5" />
      <ComposerHeaderContent className="flex min-w-0 flex-col">
        <span className="line-clamp-2">{preview.detail}</span>
        <ProblemWays problem={failed.problem} connect={failed.connect} busy={false} strings={strings} onConnections={connections} />
      </ComposerHeaderContent>
    </ComposerHeaderRow>
  ) : null;
  // Expand over a row that needs the user, which covers the line; open, Collapse is in the bar's top
  // right, and a press on the bar's empty part collapses it too (AssistantBar).
  const toggle = conn ? null : (
    <Button type="button" variant="ghost" size="sm" icon={<Maximize2 />} onClick={() => roomFirst(() => setShown(true))}>
      {strings.expand}
    </Button>
  );
  const toggleRow = toggle && (
    <ComposerHeaderRow className="justify-end">
      <ComposerHeaderActions>{toggle}</ComposerHeaderActions>
    </ComposerHeaderRow>
  );
  // Closed, the line floats over the message box (Mastra's overlay header), and a row that needs the
  // user floats in front of it: the line stays in sight until the row has the room to be drawn. Open,
  // the body is in the page's flow, with what needs the user as its last row, right above the box.
  // The first header is the same element open or closed, and changes placement without Mastra's slide
  // of its lower edge.
  // The body is never taken down: closed, it waits unseen at the height it opens at, so it opens as
  // it was left, scrolled to the end with every row in place, and a running turn's rows do not come
  // in again. The fold over the box is as tall as the line closed and as the body open, and grows
  // between them with Mastra's own motion (the header's tokens): the conversation rises from the box
  // as the line gives way to it. The view keeps its height meanwhile where the host clips it.
  const header = (
    <>
      <ComposerHeader open placement={open ? 'stacked' : 'overlay'} inert={!open && Boolean(attention)} style={open ? { transition: 'none' } : undefined}>
        <div data-centre-fold className="centre-fold relative" style={{ height: open ? room : lineHeight }}>
          <div
            data-centre-body
            data-open={open || undefined}
            inert={!open}
            aria-hidden={open ? undefined : true}
            className="centre-fold-body absolute inset-x-0 bottom-0"
            style={{ height: room }}
          >
            {body}
          </div>
          {!conn && (
            <ComposerHeaderRow
              data-centre-line
              data-open={open || undefined}
              inert={open}
              aria-hidden={open ? true : undefined}
              className="centre-fold-line absolute inset-x-0 bottom-0 flex-nowrap"
            >
              <ComposerHeaderContent className="min-w-0">
                <PreviewLine line={preview} open={open} strings={strings} onExpand={() => roomFirst(() => setShown(true))} />
              </ComposerHeaderContent>
              <Grip strings={strings} />
            </ComposerHeaderRow>
          )}
        </div>
        {open && attention}
      </ComposerHeader>
      {!open && attention && (
        <ComposerHeader placement="overlay" open>
          {toggleRow}
          {attention}
        </ComposerHeader>
      )}
    </>
  );
  return (
    <div className="flex h-dvh flex-col justify-end overflow-hidden">
      <FilePreviewDialog file={state.filePreview ?? null} />
      <div ref={measured} className="mx-auto w-full">
        <AssistantComposer state={state} onConnections={connections} header={header} />
      </div>
    </div>
  );
}
