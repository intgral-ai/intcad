// Which models the picker lists, and in what order.

// Names of models that cannot hold a conversation: pictures, sound, vectors, live audio, moderation.
const NOT_CHAT = /embedding|image|imagine|realtime|lyria|veo|imagen|tts|audio|live|whisper|moderation|-video/i;

/** Whether a model id (`provider/model`) names a model one can talk to. */
export function isChatModel(id) {
  return !NOT_CHAT.test(id.slice(id.indexOf('/') + 1));
}

/**
 * Release dates by model id, from Mastra's provider registry (`providers.<id>.modelReleaseDates`).
 * The registry is what the model picker's own order is meant to be fed with.
 */
export function releaseDates(registry) {
  const dates = new Map();
  for (const [provider, config] of Object.entries(registry?.providers ?? {})) {
    for (const [model, date] of Object.entries(config?.modelReleaseDates ?? {})) {
      if (typeof date === 'string' && date) dates.set(`${provider}/${model}`, date);
    }
  }
  return dates;
}

/** Newest first; a model without a date follows the dated ones; equal dates keep the order by id. */
export function newestFirst(dates) {
  return (a, b) => {
    const [da, db] = [dates.get(a) ?? '', dates.get(b) ?? ''];
    if (da !== db) return da < db ? 1 : -1;
    return a < b ? -1 : a > b ? 1 : 0;
  };
}

/**
 * The options of a session's model setting as the picker gets them: only models one can talk to,
 * newest first. The model in use stays in the list whatever it is, so the setting never points at
 * nothing. `typed(id)` says a model's name was typed by the user (a custom address's): those are
 * all listed, whatever their name or their address's name looks like.
 */
export function orderModelOptions(options, dates, current, typed = () => false) {
  const order = newestFirst(dates);
  return options.filter(option => option.value === current || typed(option.value) || isChatModel(option.value)).sort((a, b) => order(a.value, b.value));
}
