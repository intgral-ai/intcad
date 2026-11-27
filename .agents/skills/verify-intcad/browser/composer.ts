import { always, eventually, next } from '@antithesishq/bombadil';
import { actions, extract, getFingerprint } from '@antithesishq/bombadil/browser';
import { clickablePoint } from '@antithesishq/bombadil/browser/dom';
export { noUncaughtExceptions, noUnhandledPromiseRejections } from '@antithesishq/bombadil/browser/defaults';

const composer = extract(({ document, lastAction }) => {
  const box = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]');
  const button = document.querySelector<HTMLButtonElement>('button[aria-label="Send"]');
  if (!box || !button) return null;
  const boxPoint = clickablePoint(box);
  const buttonPoint = clickablePoint(button);
  if (!boxPoint || !buttonPoint) return null;
  return {
    draft: box.value,
    disabled: button.disabled,
    focused: box === document.activeElement,
    box: { fingerprint: getFingerprint(box), point: boxPoint },
    button: { fingerprint: getFingerprint(button), point: buttonPoint },
    sends: Number(document.documentElement.dataset.sends || 0),
    lastSent: document.documentElement.dataset.lastSent ?? null,
    pressedEnter: !!lastAction && typeof lastAction === 'object' && 'PressKey' in lastAction && lastAction.PressKey.code === 13,
    clickedSend: !!lastAction && typeof lastAction === 'object' && 'Click' in lastAction && lastAction.Click.fingerprint.tag === 'button',
  };
});

export const sendMatchesDraft = always(() => {
  const ui = composer.current;
  return !ui || ui.disabled === (ui.draft.trim() === '');
});

// Compare what was visible before the action with the message the host actually received.
export const submitSendsDraftOnceAndClears = always(() => {
  const before = composer.current;
  if (!before) return true;
  return next(() => {
    const after = composer.current;
    if (!after) return false;
    if (!before.draft.trim() || !(after.pressedEnter || after.clickedSend)) {
      return after.sends === before.sends;
    }
    return after.sends === before.sends + 1 && after.lastSent === before.draft &&
      after.draft === '' && after.disabled;
  });
});

export const exercisedBothSubmitRoutes = eventually(() =>
  (composer.current?.sends ?? 0) >= 2
).within(15, 'seconds');

export const exerciseComposer = actions(() => {
  const ui = composer.current;
  if (!ui) return ['Wait'];
  if (!ui.focused) return [{ Click: ui.box }];
  // Try Enter on whitespace before adding generated text. Cover each submit route once
  // before letting Bombadil choose between them.
  if (ui.draft === '') return [{ TypeText: { text: { Regexp: ' {1,3}' } } }];
  const enter = { PressKey: { code: 13 } };
  if (ui.draft.trim() === '') {
    return ui.pressedEnter ? [{ TypeText: { text: { Regexp: '[a-zA-Z0-9]{1,20}' } } }] : [enter];
  }
  const click = { Click: ui.button };
  if (ui.sends === 0) return [click];
  if (ui.sends === 1) return [enter];
  return [click, enter];
});
