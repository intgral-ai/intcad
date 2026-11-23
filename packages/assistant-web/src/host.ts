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
  | 'messageBox' | 'composer' | 'withNext' | 'latest' | 'turnLabel'
  | 'reviewTitle' | 'reviewAccept' | 'reviewReject' | 'reviewMade' | 'reviewChanged' | 'reviewRemoved'
  | 'reviewAccepted' | 'reviewRejected' | 'reviewMore'
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
  | { type: 'side'; left: boolean }
  | { type: 'surface'; color: string }
  | { type: 'copy'; text: string }
  | { type: 'review'; id: number; accept: boolean }
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

declare global {
  interface Window {
    ipc?: { postMessage(message: string): void };
    __assistantHost?: { state(next: AssistantState): void };
  }
}

/** Where the app serves one of the conversation's pictures. */
export function pictureUrl(key: string) {
  return `picture/${encodeURIComponent(key)}`;
}

export function post(action: Action) {
  window.ipc?.postMessage(JSON.stringify(action));
}

/** The Assistant's state as the app last pushed it; `null` until the first push. */
export function useAssistant(): AssistantState | null {
  const [state, setState] = useState<AssistantState | null>(null);
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
