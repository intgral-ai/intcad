// Start with an idle desktop session. Tests can push host updates through the same interface.
const emptyContext = { groups: [], omitted: null };
const session = {
  state: 'ready', running: true, items: [], waiting: 0, stopped: false, pending: [],
  attachments: [], loading: [], picked: emptyContext, selection: emptyContext, selected: 0,
  dragging: false, attachView: false, attachSelection: false, images: false, radius: 0,
  permissions: 'ask', skill: '', skills: [], turn: null, detecting: false,
  agents: [{ id: 'fixture', name: 'Fixture', detail: '', hint: null }], agent: 'fixture',
  modes: [], mode: '', models: [], model: '', customModels: false, config: null,
  configPending: false, commands: [], notes: [], auth: null, missing: [], confirmNew: false,
  left: false, dark: false, dfm: { display: 'off', selected: null },
  strings: new Proxy({
    messageBox: 'Message', send: 'Send', stop: 'Stop', removeChip: 'Remove',
    allowOnce: 'Allow once', alwaysAllow: 'Always allow', deny: 'Deny', allowed: 'Allowed', denied: 'Denied',
    reviewAccept: 'Accept changes', reviewReject: 'Reject changes',
    reviewAccepted: 'Changes accepted', reviewRejected: 'Changes rejected',
    dfmMeasured: 'Measured', dfmRequirement: 'Requirement', dfmStale: 'Out of date',
    newChat: 'New chat', newChatConfirm: 'Start a new conversation?', cancel: 'Cancel',
    dfmSelect: 'Selected finding', dfmSelected: 'Selected only', dfmNotCovered: 'Not checked',
  }, {
    get: (labels, key) => labels[key] ?? key,
  }),
};

window.assistantFixture = {
  actions: [],
  show(overrides) { window.__assistantHost.state({ ...session, ...overrides }); },
};
window.ipc = {
  postMessage(raw) {
    const action = JSON.parse(raw);
    if (action.type === 'ready') window.assistantFixture.show({});
    else if (!['surface', 'focus'].includes(action.type)) window.assistantFixture.actions.push(action);
    if (action.type === 'send') {
      const evidence = document.documentElement.dataset;
      evidence.lastSent = action.text;
      evidence.sends = String(Number(evidence.sends || 0) + 1);
    }
  },
};
