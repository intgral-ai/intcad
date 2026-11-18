// Supply one ready, idle desktop session through the page's normal host interface.
const emptyContext = { groups: [], omitted: null };
const session = {
  state: 'ready', running: true, items: [], waiting: 0, stopped: false, pending: [],
  attachments: [], loading: [], picked: emptyContext, selection: emptyContext, selected: 0,
  dragging: false, attachView: false, attachSelection: false, images: false, radius: 0,
  permissions: 'ask', skill: '', skills: [], turn: null, detecting: false,
  agents: [{ id: 'fixture', name: 'Fixture', detail: '', hint: null }], agent: 'fixture',
  modes: [], mode: '', models: [], model: '', customModels: false, config: null,
  configPending: false, notes: [], auth: null, missing: [], confirmNew: false,
  left: false, dark: false, dfm: { display: 'off', selected: null },
  strings: new Proxy({ messageBox: 'Message', send: 'Send' }, {
    get: (labels, key) => labels[key] ?? key,
  }),
};

const evidence = document.documentElement.dataset;
window.ipc = {
  postMessage(raw) {
    const action = JSON.parse(raw);
    if (action.type === 'ready') window.__assistantHost.state(session);
    if (action.type === 'send') {
      evidence.lastSent = action.text;
      evidence.sends = String(Number(evidence.sends || 0) + 1);
    }
  },
};
