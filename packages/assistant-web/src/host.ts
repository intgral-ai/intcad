// The page's side of the bridge to the desktop app. The app pushes the Assistant's state by
// calling `window.__assistantHost.state(...)`; the page asks for things with
// `window.ipc.postMessage(JSON)`. What the app accepts is listed in
// crates/ui-egui/src/agent_chat/web.rs (`act`).
import { useEffect, useState } from 'react';

/** One thing a message was sent with, and its place among all of that message's things. */
export interface ContextChip {
  index: number;
  label: string;
  detail: string;
}

/** What a message was sent with: a group for each kind, and what was left out, in words. */
export interface ContextGroups {
  groups: { heading: string; items: ContextChip[]; more: string | null }[];
  omitted: string | null;
}

/** A picture the app serves by `key` (see `pictureUrl`); the bytes are never part of the state. */
export interface PictureRef {
  key: string;
  note: string;
  view: boolean;
}

/** A message waiting in the queue. It has no name of its own: actions say which text they mean. */
export interface QueuedMessage {
  text: string;
  image: boolean;
  files: string[];
  context: ContextGroups;
}

export interface PermissionOption {
  id: string;
  name: string;
  kind: string;
}

export interface ConfigOption {
  id: string;
  name: string;
  description: string;
  category: string;
  currentValue: string;
  options: { value: string; name: string; description: string; group: string | null }[];
}

/** A slash command the agent advertised for the open session; `name` is its own, unaltered. */
export interface AvailableCommand {
  name: string;
  description: string;
}

export interface AuthMethod {
  id: string;
  label: string;
  kind: 'agent' | 'terminal' | 'env';
  action?: string;
  command?: string;
  note?: string;
}

export type ToolStatus = 'completed' | 'failed' | 'running' | 'pending';

/** One finding of a DFM check. `requirement` is that finding's own number, never a data table. */
export interface CheckFinding {
  id: string;
  sheet: number;
  body: string;
  from: string;
  rules: string[];
  state: string;
  status: string;
  code: string | null;
  measured: { value: number; unit: string } | null;
  requirement: { value: number; unit: string; layer: string; confirmed: string | null; source: string } | null;
  feature: { kind: string; number: number | null } | null;
  faces: string[];
  reason: string | null;
}

export interface CheckCard {
  runId: string;
  candidateDigest: string;
  configDigest: string;
  modelVersion: string;
  projectId: string;
  source: string;
  freshness: string;
  findings: CheckFinding[];
  rules: { ruleId: string; state: string; count: number }[];
  notChecked: string;
  status: 'running' | 'cancelled' | 'done' | 'error';
  error: string | null;
  counts: { passed: number; failed: number; advisory: number; checked: number; unsupported: number };
}

/** One entry of the app's transcript. */
export type Item =
  | { kind: 'user'; index: number; text: string; image: boolean; pictures: PictureRef[]; files: string[]; context: ContextGroups }
  /** `objects`: the names in this answer that the app can select, in the order a click names them by. */
  | { kind: 'text'; index: number; text: string; thought: boolean; objects: string[] }
  /**
   * `pictures`: what the call handed back as pictures (a file the agent read); `index` opens one, as a message's.
   * `file`: a file the call read that has a card; a click (`file` with `index`) previews its text here or opens it with the system's program.
   */
  | { kind: 'tool'; id: string; index: number; title: string; toolKind: string; status: ToolStatus; words: string | null; input: string | null; output: string | null; pictures: PictureRef[]; file: { name: string; act: 'preview' | 'open' } | null }
  | { kind: 'plan'; steps: { text: string; status: string }[] }
  | { kind: 'skill'; line: string; warnings: string[]; terminal: string | null; revalidate: boolean }
  | { kind: 'permission'; index: number; heading: string; answered: string | null; options: PermissionOption[] }
  | { kind: 'approval'; id: number; title: string; answer: boolean | null }
  | { kind: 'changes'; index: number; label: string; undone: boolean; undoable: boolean }
  /** A part ticket the agent drafted in this turn; it waits in the tickets view's 待接受 group. Ahead of the host. */
  | { kind: 'ticket'; id: string; title: string; state: TicketStateName }
  | { kind: 'review'; index: number; review: number; steps: string[]; more: number; made: { name: string; volume: number }[]; changed: { name: string; before: number; after: number }[]; removed: { name: string; volume: number }[]; outcome: string }
  /** `problem`: a turn of the built-in agent failed on its connection; `text` already says so in full. */
  | { kind: 'notice'; text: string; error: boolean; problem?: TurnProblem | null }
  | {
      kind: 'check';
      id: string;
      status: 'running' | 'cancelled' | 'done' | 'error' | 'pending';
      card: CheckCard | null;
      display: 'all' | 'selected' | 'off';
      selected: { runId: string; findingId: string } | null;
    };

/**
 * Why a turn of the built-in agent failed, and on which connection (`name` in the user's words).
 * `auth` is a plan whose sign-in was refused; `key` is an API key or a custom address that was.
 */
export interface TurnProblem {
  kind: 'auth' | 'key' | 'quota' | 'rate' | 'network' | 'model';
  prefix: string;
  name: string;
}

/**
 * The built-in agent's connections: what it can talk to, and a sign-in under way. `prefix` is the
 * part before `/` in the ids of the models a connection gives.
 */
export interface Connect {
  plans: { id: string; name: string; state: 'in' | 'out' | 'expired'; account: string | null; prefix: string }[];
  /** `shadowed`: a signed-in plan serves the same models, and wins. */
  keys: { id: string; name: string; saved: boolean; shadowed: boolean; prefix: string; title: string }[];
  custom: { id: string; name: string; url: string; models: string[]; keyed: boolean; prefix: string }[];
  login: { id: string; phase: 'starting' | 'browser' | 'device' | 'code'; userCode: string | null; error: 'incomplete' | null } | null;
  /** The last sign-in that could not finish. */
  problem: { id: string; kind: 'port' | 'offline' | 'failed' } | null;
  /** A key or custom address the app could not keep, in the user's language. */
  error: string | null;
  /** The key saves the app has answered (kept or refused), counted: a save is answered once it moves on. */
  keySaves: number;
  /** Release date (`YYYY-MM-DD`) by model id. */
  dates: Record<string, string>;
  /** A plan signed in, a key saved, or a custom address: there is something to talk to. */
  connected: boolean;
}

export interface MissingAgent {
  id: string;
  name: string;
  install: string;
  /** One of the basic agents: listed although it is not found, with `body` saying what to do. */
  basic: boolean;
  body: string | null;
}

export interface AssistantState {
  items: Item[];
  /** A text file open from a file card: the start of its text; `markdown`: drawn as Markdown. */
  filePreview: { name: string; text: string; markdown: boolean } | null;
  /** off, starting, ready, busy, auth, failed, exited */
  state: string;
  waiting: number;
  stopped: boolean;
  pending: QueuedMessage[];
  attachments: { name: string; picture: string | null; report: string | null }[];
  loading: string[];
  picked: ContextGroups;
  /** Files are being dragged over the panel. */
  dragging: boolean;
  attachView: boolean;
  attachSelection: boolean;
  images: boolean;
  selected: number;
  /** What is selected in the design now, as the agent can read it through the app's tools. */
  selection: ContextGroups;
  /** The corner radius of the page's own surface, in points: the floating panel's, 0 when docked. */
  radius: number;
  /** At the bottom centre of the window: the most height the page may take, in points; absent in the side panel. */
  centre?: number | null;
  permissions: 'ask' | 'edits' | 'all';
  skill: string;
  skills: { id: string; title: string }[];
  /** The running turn's change count, in words. */
  turn: string | null;
  /** The agents this conversation can use, and the one it uses. */
  /** `hint`: what is known of an installed agent without starting it, in a word; null for one the user added. */
  agents: { id: string; name: string; detail: string; hint: string | null }[];
  agent: string | null;
  detecting: boolean;
  running: boolean;
  /** The agent's own modes and models, when it has them; `config` replaces both when given. */
  modes: { id: string; name: string }[];
  mode: string;
  models: { id: string; name: string; group: string | null; tip: string }[];
  model: string;
  /** The agent takes a model it did not list. */
  customModels: boolean;
  config: ConfigOption[] | null;
  configPending: boolean;
  /** The slash commands the agent advertised for this session, in the agent's own words. */
  commands: AvailableCommand[];
  /** Lines about the agent worth saying: a model warning, the runtime, the sign-in. */
  notes: string[];
  /** The agent asks for a sign-in: how each of its methods is done. */
  auth: AuthMethod[] | null;
  /** The known agents not found. The basic ones are listed as not ready. */
  missing: MissingAgent[];
  confirmNew: boolean;
  left: boolean;
  dark: boolean;
  /** The page's own words in the app's language. */
  strings: Strings;
  /** Which findings the viewport marks, and the one selected. */
  dfm: { display: 'all' | 'selected' | 'off'; selected: { runId: string; findingId: string } | null };
  /** Present exactly when the current agent is the built-in one; without it the page is as it was. */
  connect?: Connect | null;
}

/** The names of the page's own words: the list `state` sends in crates/ui-egui/src/agent_chat/web.rs. */
// prettier-ignore
export type StringKey =
  | 'placeholder' | 'placeholderBusy' | 'send' | 'stop' | 'working' | 'ready' | 'off' | 'starting' | 'auth'
  | 'failed' | 'exited' | 'undone' | 'undo' | 'allowOnce' | 'alwaysAllow' | 'deny' | 'allowed' | 'denied'
  | 'approval' | 'title' | 'add' | 'addFile' | 'attachView'
  | 'attachSelection' | 'pickContext' | 'modeAsk' | 'modeEdits' | 'modeAll' | 'queued' | 'viewAttached' | 'waiting'
  | 'connect' | 'disconnect' | 'retry' | 'newChat' | 'newChatConfirm' | 'cancel' | 'close' | 'minimize'
  | 'dockLeft' | 'dockRight' | 'expand' | 'collapse' | 'addAgent' | 'settings' | 'switchAgent' | 'detecting' | 'noneFound' | 'notInstalled'
  | 'notReady' | 'notSetUp'
  | 'refresh' | 'authTitle' | 'authNeeded' | 'authNone' | 'authTerminal' | 'authRetry' | 'setupTitle'
  | 'setupBody' | 'openSettings' | 'model' | 'queuedStopped' | 'sendNow' | 'sendNowTip' | 'edit'
  | 'remove' | 'save' | 'modelSearch' | 'modelCustom' | 'modelNoMatch' | 'modelRefine' | 'modelFilter' | 'modelEffort' | 'revalidate' | 'dropHint' | 'removeChip'
  | 'openPicture' | 'selectedNow'
  | 'messageBox' | 'composer' | 'withNext' | 'latest' | 'turnLabel' | 'moveBox'
  | 'reviewTitle' | 'reviewAccept' | 'reviewReject' | 'reviewMade' | 'reviewChanged' | 'reviewRemoved'
  | 'reviewAccepted' | 'reviewRejected' | 'reviewMore' | 'ticketShow'
  | 'dfmCheckTitle' | 'dfmRunning' | 'dfmDone' | 'dfmCancelled' | 'dfmError' | 'dfmNotChecked'
  | 'dfmRefresh' | 'dfmStale' | 'dfmFindings' | 'dfmAdvisory' | 'dfmFailed' | 'dfmChecked'
  | 'dfmNotCovered' | 'dfmMeasured' | 'dfmRequirement' | 'dfmSource' | 'dfmOverride'
  | 'dfmAll' | 'dfmSelected' | 'dfmOff' | 'dfmSelect'
  | 'connTitle' | 'connNoneTitle' | 'connBack'
  | 'connPlans' | 'connKeys' | 'connCustom' | 'connSignedIn' | 'connExpired' | 'connSigningIn'
  | 'connSignIn' | 'connSignInAgain' | 'connSignOut' | 'connUse' | 'connInUse' | 'connShadowed'
  | 'connKeySaved' | 'connKeyAdd' | 'connKeyUpdate' | 'connRemove' | 'connKeyTitle'
  | 'connKeyBody' | 'connKeyPlaceholder' | 'connSave' | 'connCustomAdd' | 'connCustomName'
  | 'connCustomUrl' | 'connCustomKey' | 'connCustomModels' | 'connCustomNeedBoth' | 'connCustomNeedModel' | 'connCustomTooMany' | 'connCustomPlainUrl' | 'connCustomExists' | 'connAdd'
  | 'connWaiting' | 'connStarting' | 'connReopen' | 'connCodeHint' | 'connCodePlaceholder' | 'connCodeIncomplete'
  | 'connCodeDone' | 'connDeviceHint' | 'connProblemPort' | 'connProblemOffline' | 'connProblemFailed'
  | 'connButton' | 'connButtonNone' | 'connButtonTip'
  | 'planOpenaiCodex' | 'planAnthropic' | 'planKimiForCoding' | 'planXai' | 'planGithubCopilot'
  | 'problemAuth' | 'problemKey' | 'problemQuota' | 'problemRate' | 'problemNetwork' | 'problemModel'
  | 'effort' | 'effortOff' | 'effortLow' | 'effortMedium' | 'effortHigh' | 'effortXhigh' | 'effortMax';

export type Strings = Record<StringKey, string>;

export type Action =
  | { type: 'ready' }
  | { type: 'focus'; on: boolean }
  | { type: 'log'; text: string }
  | { type: 'send'; text: string; now?: boolean }
  | { type: 'send_now' }
  | { type: 'remove_pending'; index: number; text: string }
  | { type: 'edit_pending'; index: number; was: string; text: string }
  | { type: 'object'; index: number; ref: number; label: string }
  | { type: 'picture'; index: number; at: number }
  | { type: 'file'; index: number }
  | { type: 'file_close' }
  | { type: 'revalidate' }
  | { type: 'stop' }
  | { type: 'approve'; id: number; allow: boolean | 'always' }
  | { type: 'permission'; index: number; option: string }
  | { type: 'undo'; index: number }
  | { type: 'permissions'; mode: 'ask' | 'edits' | 'all' }
  | { type: 'attach_view'; on: boolean }
  | { type: 'attach_selection'; on: boolean }
  | { type: 'pick_context' }
  | { type: 'remove_picked'; index: number; label: string }
  | { type: 'add_file' }
  | { type: 'paste_picture' }
  | { type: 'remove_file'; index: number }
  | { type: 'skill'; id: string }
  | { type: 'new_chat' }
  | { type: 'confirm_new'; yes: boolean }
  | { type: 'agent'; id: string }
  | { type: 'connect' }
  | { type: 'disconnect' }
  | { type: 'agent_mode'; id: string }
  | { type: 'model'; id: string }
  | { type: 'custom_model'; text: string }
  | { type: 'config'; id: string; value: string }
  | { type: 'auth'; id: string }
  | { type: 'refresh' }
  | { type: 'add_agent' }
  | { type: 'settings' }
  | { type: 'close' }
  | { type: 'minimize' }
  /** At the bottom centre: how tall the page wants to be, in points (the host keeps it within `centre`). */
  | { type: 'size'; height: number }
  /** There: the user moves the box sideways; the pointer's travel since the press, in screen units (`screenX`). */
  | { type: 'move'; dx: number; done?: boolean }
  | { type: 'side'; left: boolean }
  | { type: 'surface'; color: string }
  | { type: 'copy'; text: string }
  | { type: 'review'; id: number; accept: boolean }
  /** Show a drafted part ticket in the tickets view. Ahead of the host: its `act` refuses it today. */
  | { type: 'ticket_show'; id: string }
  | { type: 'open_link'; url: string }
  // The built-in agent's connections; the app refuses these for any other agent.
  | { type: 'conn_login'; id: string }
  | { type: 'conn_code'; id: string; code: string }
  | { type: 'conn_reopen' }
  | { type: 'conn_cancel' }
  | { type: 'conn_logout'; id: string }
  | { type: 'conn_key'; provider: string; key: string }
  | { type: 'conn_key_remove'; provider: string }
  | { type: 'conn_custom'; name: string; url: string; key?: string; models: string[] }
  | { type: 'conn_custom_remove'; id: string }
  | { type: 'conn_retry' };

// ---- The project's part tickets (a second view of this page) ----
// The host pushes this state instead of the Assistant's, marked `view: 'tickets'`; the shapes
// are the store's own (crates/engine/src/project/tickets.rs). What a ticket says was written by
// an agent or a person: the page shows it as plain text.

export type TicketStateName = 'draft' | 'confirmed' | 'in_progress' | 'waiting' | 'done' | 'blocked';

/** A part a ticket bears on, as the model names it. */
export interface TicketPart {
  kind: string;
  id: string;
}

/** A file in the project folder a ticket points to; `picture`: the key the app serves a picture's thumbnail by (`pictureUrl`). */
export interface TicketAttachment {
  path: string;
  name: string;
  picture: string | null;
}

export interface Ticket {
  id: string;
  parent: string | null;
  title: string;
  text: string;
  requirements: string[];
  part: TicketPart | null;
  // The fields below are the page's ahead of the store: absent, a ticket reads as today's (`full` in shared.ts).
  /** The parts it bears on, none, one or several; `part` until the store keeps a list. */
  parts?: TicketPart[];
  attachments?: TicketAttachment[];
  /** The tickets it waits for: the Assistant cannot take it before they are done; a person is reminded. */
  waits?: string[];
  /** Who works it once it is started. */
  worker?: 'assistant' | 'person' | null;
  /** Deleted by a person: listed under 已删除, restorable, its record kept. */
  deleted?: boolean;
  /** The parent it was deleted together with; restoring that parent brings it back. */
  deletedWith?: string | null;
  /** A check applies to it (it bears on parts and the check is not switched off). Without one it may be finished unchecked. */
  checks?: boolean;
  /** The check is switched off for it (shown as a switch on a ticket with parts). */
  checksOff?: boolean;
  /** 需重查: changed after it was finished, and why. */
  recheck?: string | null;
  /** The Assistant says it has finished the work; a person runs the check. */
  finished?: boolean;
  /** The store's reasons, so the page never decides them itself: why it cannot be finished now, the tickets it still waits for, whether finishing leaves it unchecked. */
  whyNotDone?: { kind: 'subs'; ids: string[] } | { kind: 'check' } | { kind: 'recheck'; why: string } | null;
  openWaits?: string[];
  unchecked?: boolean;
  state: TicketStateName;
  /** The check bound to it: the run's own outcome word and counts, never a verdict of ours. */
  check: {
    state: 'asked' | 'running' | 'ran' | 'refused';
    run?: string;
    outcome?: string;
    counts?: Record<string, number>;
    freshness?: string;
    reason?: string;
    at: string;
  } | null;
  blocked: string | null;
  revision: number;
  created: string;
  updated: string;
}

/** One line of a ticket's record; `at` is RFC 3339 in UTC. A `comment` line's text is its `note`. */
export interface TicketEvent {
  at: string;
  actor: 'person' | 'agent' | 'app';
  ticket: string;
  action: string;
  from?: TicketStateName;
  to?: TicketStateName;
  note?: string;
  run?: string;
  /** A comment's files, or the files the Assistant attached when it said it had finished. */
  attachments?: TicketAttachment[];
  revision: number;
}

/** One item of a check's run, as the run reported it: its state, how many findings, on which parts. */
export interface CheckItem {
  /** The item's own name in the check (a rule's id), shown small. */
  rule: string;
  state: string;
  count: number;
  parts: string[];
  /** What was found against what is required, in the check's own words. */
  values?: string[];
}

/** The names of the tickets view's words: the list `tickets_state` sends in web.rs. */
// prettier-ignore
export type TicketStringKey =
  | 'title' | 'newTicket' | 'empty' | 'noProject' | 'previous' | 'next' | 'more'
  | 'stateDraft' | 'stateConfirmed' | 'stateInProgress' | 'stateWaiting' | 'stateDone' | 'stateBlocked'
  | 'properties' | 'part' | 'noPart' | 'check' | 'noCheck' | 'checkAsked' | 'checkRunning' | 'checkRefused'
  | 'checkTitle' | 'checkDone' | 'checkStale' | 'failed' | 'advisory' | 'checked' | 'notCovered'
  | 'requirements' | 'activity' | 'actorPerson' | 'actorAgent' | 'actorApp'
  | 'didDrafted' | 'didCreated' | 'didEdited' | 'didConfirmed' | 'didStarted' | 'didCheckAsked' | 'didCheckStarted'
  | 'didCheckRan' | 'didCheckRefused' | 'didBlocked' | 'didUnblocked' | 'didDone'
  | 'confirm' | 'start' | 'done' | 'delete' | 'cancel' | 'create'
  | 'fieldTitle' | 'fieldText' | 'fieldLines'
  | 'decline' | 'proposed' | 'state' | 'waitsFor' | 'setWaits' | 'worker' | 'workerNone' | 'workMyself' | 'askCheck'
  | 'whyOffline' | 'whyBusy' | 'whyWaits' | 'stillWaits' | 'whySubs' | 'whyCheck' | 'notChecked' | 'noApplicableCheck'
  | 'attachments' | 'attach' | 'subTickets' | 'addSub' | 'parent' | 'assign' | 'useSelection' | 'selectFirst'
  | 'filterPart' | 'clear' | 'askDraft' | 'deleted' | 'noDeleted' | 'restore' | 'deletedToast' | 'undo'
  | 'expand' | 'collapse' | 'addText'
  | 'didDeclined' | 'didDeleted' | 'didRestored' | 'didTaken' | 'didMoved' | 'didFinished' | 'didCheckOff' | 'didCheckOn' | 'didRecheck' | 'didViewAttached'
  | 'runCheck' | 'recheck' | 'finishedNote' | 'needsCheck' | 'deleteWithSubs' | 'confirmWithSubs' | 'together' | 'onlyThis' | 'whyRecheck' | 'withTicket'
  | 'whyStale' | 'checkOff'
  | 'comment' | 'commentPlaceholder' | 'backTitle' | 'backWhy' | 'backConfirm' | 'checkItems'
  | 'attachFile' | 'attachViewNow' | 'removeFile' | 'checkRan';

export interface TicketsState {
  view: 'tickets';
  dark: boolean;
  /** The app's language, for dates and times in the person's own form. */
  language: string;
  strings: Record<TicketStringKey, string>;
  /** The open design is in a project folder: without one there is nowhere to keep tickets. */
  project: boolean;
  tickets: Ticket[];
  /** The ticket whose page is open, and its record, oldest first. */
  selected: string | null;
  record: TicketEvent[];
  /** Each bound run's freshness now, by run id. */
  freshness: Record<string, string>;
  /** The open ticket's runs, by run id: each rule the run reported. */
  items?: Record<string, CheckItem[]>;
  /** Why a step was refused, in the store's words. */
  error: string | null;
  /** The same refusal as a kind the page says in the person's words; absent for any other. */
  refusal?:
    | { kind: 'stale'; id: string }
    | { kind: 'subs'; ids: string[] }
    | { kind: 'check' }
    | { kind: 'recheck'; why: string }
    | { kind: 'waits'; ids: string[] }
    | { kind: 'busy'; id: string }
    | { kind: 'offline' }
    | null;
  // Ahead of the host, like the ticket's new fields: absent, the view is as it was.
  /** The open ticket is shown large over the window: the app has widened this view to the window. */
  expanded?: boolean;
  /** A part picked in the 3D view or the tree: the list shows only the tickets that bear on it. */
  filter?: TicketPart | null;
  /** The parts selected in the model now: a new ticket starts with them. */
  selection?: TicketPart[];
  /** Files the app has taken into the project's `attachments/` for the ticket being created. */
  adding?: TicketAttachment[];
  /** Files taken in for the comment being written on the open ticket; they go with it when it is sent. */
  commenting?: TicketAttachment[];
  /** An agent is connected, so the Assistant can take a ticket. */
  assistant?: boolean;
  /** The view's latest requests as the host answered them: the host's count, the page's own number (`ask`) and whether the step was taken. Several: the page may send more than one between two states. */
  answered?: { n: number; ask: number | null; ok: boolean }[];
}

/** What the host pushes: the Assistant's state, or the tickets view's. */
export type PageState = AssistantState | TicketsState;

/**
 * The tickets view's requests; the host (crates/ui-egui/src/tickets.rs, `act`) has an arm for each
 * name, a test keeps the two equal, and the store checks each step against the ticket's state and
 * `revision`. `ticket_confirm` accepts a draft (`with_subs`: its draft sub-tickets too, the
 * default), `ticket_start` hands a ticket to the Assistant. `ask`: the page's own number, which the
 * host answers in `answered`.
 */
export type TicketAction = (
  | { type: 'ticket_open'; id: string }
  | { type: 'ticket_close' }
  | { type: 'ticket_start' | 'ticket_done' | 'ticket_delete'; id: string; revision: number }
  | { type: 'ticket_confirm'; id: string; revision: number; with_subs?: boolean }
  /** Only the fields that change; the others stay as they are. */
  | { type: 'ticket_edit'; id: string; revision: number; title?: string; text?: string; requirements?: string[]; checks?: boolean }
  | { type: 'ticket_create'; title: string; text: string; requirements: string[]; parts: TicketPart[]; attachments: string[]; parent: string | null }
  | { type: 'ticket_decline' | 'ticket_work_myself' | 'ticket_ask_check' | 'ticket_assign_parts'; id: string; revision: number }
  /** `reason`: why, when a person moves it back; optional. */
  | { type: 'ticket_set_state'; id: string; revision: number; to: TicketStateName; reason?: string }
  /** A comment: a person's text on the ticket, and the files taken in for it, in its record; it cannot be changed once sent. */
  | { type: 'ticket_comment'; id: string; text: string; attachments: string[] }
  /** A file for the comment being written: the app asks for one, or (`view`) takes a picture of the 3D view as it is now. */
  | { type: 'ticket_comment_attach'; id: string; view: boolean }
  | { type: 'ticket_waits'; id: string; revision: number; waits: string[] }
  | { type: 'ticket_restore' | 'ticket_expand' | 'ticket_show_parts'; id: string }
  /** `id`: the ticket a file goes to, null for the one being created; the app asks for the file. */
  | { type: 'ticket_attach'; id: string | null }
  /** A picture opens in the app's viewer, any other file in the system's program for it. */
  | { type: 'ticket_attachment'; path: string }
  | { type: 'ticket_collapse' | 'ticket_filter_clear' | 'ticket_ask_draft' }
) & { ask?: number };

declare global {
  interface Window {
    ipc?: { postMessage(message: string): void };
    __assistantHost?: { state(next: PageState): void };
  }
}

/** Where the app serves one of the conversation's pictures. */
export function pictureUrl(key: string) {
  return `picture/${encodeURIComponent(key)}`;
}

export function post(action: Action | TicketAction) {
  window.ipc?.postMessage(JSON.stringify(action));
}

let asked = 0;
const waiting = new Map<number, (ok: boolean) => void>();
let latest: TicketsState | null = null;

/** How long a request waits for its answer before it counts as not taken (a host that never answers must not hold the next save forever). */
const ANSWER_WAIT_MS = 10_000;

/** Send a tickets view request with a number of its own; `then` hears once whether the host took it. */
export function ask(action: TicketAction, then?: (ok: boolean) => void) {
  asked += 1;
  const number = asked;
  if (then) {
    waiting.set(number, then);
    window.setTimeout(() => {
      if (waiting.delete(number)) then(false);
    }, ANSWER_WAIT_MS);
  }
  post({ ...action, ask: number });
}

/** The tickets view's state as the host last pushed it. */
export const lastTickets = () => latest;

/** Each state the host pushes: kept, and each request it answers that still waits is told how it went. */
export function hear(state: TicketsState) {
  latest = state;
  for (const answer of state.answered ?? []) {
    const then = answer.ask != null ? waiting.get(answer.ask) : undefined;
    if (answer.ask != null && then) {
      waiting.delete(answer.ask);
      then(answer.ok);
    }
  }
}

/** The state as the app last pushed it (the Assistant's or the tickets view's); `null` until the first push. */
export function useAssistant(): PageState | null {
  const [state, setState] = useState<PageState | null>(null);
  useEffect(() => {
    window.__assistantHost = { state: setState };
    post({ type: 'ready' });
    return () => {
      window.__assistantHost = undefined;
    };
  }, []);
  return state;
}

/** Page-wide behaviour that makes a web view act as a panel of the app, not as a browser. */
export function installHostBehaviour() {
  // Errors and blocked loads go to the app's log: there is no console to look at.
  window.addEventListener('error', event => post({ type: 'log', text: `error: ${event.message}` }));
  window.addEventListener('unhandledrejection', event => post({ type: 'log', text: `rejection: ${String(event.reason)}` }));
  document.addEventListener('securitypolicyviolation', event =>
    post({ type: 'log', text: `blocked ${event.violatedDirective}: ${event.blockedURI}` }),
  );
  // The app decides what the keyboard and the Edit menu do while the page has the focus.
  window.addEventListener('focus', () => post({ type: 'focus', on: true }));
  window.addEventListener('blur', () => post({ type: 'focus', on: false }));
  // No browser menu on right-click; copying is Cmd+C or the copy buttons.
  document.addEventListener('contextmenu', event => event.preventDefault());
  // A link never navigates the panel: web addresses open in the system browser.
  document.addEventListener(
    'click',
    event => {
      const link = event.target instanceof Element ? event.target.closest('a[href]') : null;
      if (!link) return;
      event.preventDefault();
      const url = link.getAttribute('href') ?? '';
      if (/^https?:\/\//i.test(url)) post({ type: 'open_link', url });
    },
    true,
  );
  // Undo and redo in the message box stay in the message box: unhandled, the app's own Undo
  // would take a step off the design.
  document.addEventListener('keydown', event => {
    if (!event.metaKey || event.key.toLowerCase() !== 'z') return;
    event.preventDefault();
    document.execCommand(event.shiftKey ? 'redo' : 'undo');
  });
  // The page is not a secure context for the browser's clipboard: copying goes through the app.
  const clipboard = { writeText: async (text: string) => post({ type: 'copy', text }) };
  try {
    Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true });
  } catch {
    post({ type: 'log', text: 'could not route the clipboard through the app' });
  }
}
