// Derived from Circle, mock-data/status.tsx (github.com/ln-dev7/circle at b0ba7dd): the status
// icon builders, unchanged in drawing. MIT, Copyright (c) 2025 lndev-ui | Circle Template.
// Here: one icon and one colour for each state a part ticket can be in.
import type { TicketStateName } from '../host';

/** Dashed ring (Circle's "Backlog" gear): a draft nobody has confirmed. */
const GEAR_PATH =
  'M13.9408 7.91426L11.9576 7.65557C11.9855 7.4419 12 7.22314 12 7C12 6.77686 11.9855 6.5581 11.9576 6.34443L13.9408 6.08573C13.9799 6.38496 14 6.69013 14 7C14 7.30987 13.9799 7.61504 13.9408 7.91426ZM13.4688 4.32049C13.2328 3.7514 12.9239 3.22019 12.5538 2.73851L10.968 3.95716C11.2328 4.30185 11.4533 4.68119 11.6214 5.08659L13.4688 4.32049ZM11.2615 1.4462L10.0428 3.03204C9.69815 2.76716 9.31881 2.54673 8.91341 2.37862L9.67951 0.531163C10.2486 0.767153 10.7798 1.07605 11.2615 1.4462ZM7.91426 0.0591659L7.65557 2.04237C7.4419 2.01449 7.22314 2 7 2C6.77686 2 6.5581 2.01449 6.34443 2.04237L6.08574 0.059166C6.38496 0.0201343 6.69013 0 7 0C7.30987 0 7.61504 0.0201343 7.91426 0.0591659ZM4.32049 0.531164L5.08659 2.37862C4.68119 2.54673 4.30185 2.76716 3.95716 3.03204L2.73851 1.4462C3.22019 1.07605 3.7514 0.767153 4.32049 0.531164ZM1.4462 2.73851L3.03204 3.95716C2.76716 4.30185 2.54673 4.68119 2.37862 5.08659L0.531164 4.32049C0.767153 3.7514 1.07605 3.22019 1.4462 2.73851ZM0.0591659 6.08574C0.0201343 6.38496 0 6.69013 0 7C0 7.30987 0.0201343 7.61504 0.059166 7.91426L2.04237 7.65557C2.01449 7.4419 2 7.22314 2 7C2 6.77686 2.01449 6.5581 2.04237 6.34443L0.0591659 6.08574ZM0.531164 9.67951L2.37862 8.91341C2.54673 9.31881 2.76716 9.69815 3.03204 10.0428L1.4462 11.2615C1.07605 10.7798 0.767153 10.2486 0.531164 9.67951ZM2.73851 12.5538L3.95716 10.968C4.30185 11.2328 4.68119 11.4533 5.08659 11.6214L4.32049 13.4688C3.7514 13.2328 3.22019 12.9239 2.73851 12.5538ZM6.08574 13.9408L6.34443 11.9576C6.5581 11.9855 6.77686 12 7 12C7.22314 12 7.4419 11.9855 7.65557 11.9576L7.91427 13.9408C7.61504 13.9799 7.30987 14 7 14C6.69013 14 6.38496 13.9799 6.08574 13.9408ZM9.67951 13.4688L8.91341 11.6214C9.31881 11.4533 9.69815 11.2328 10.0428 10.968L11.2615 12.5538C10.7798 12.9239 10.2486 13.2328 9.67951 13.4688ZM12.5538 11.2615L10.968 10.0428C11.2328 9.69815 11.4533 9.31881 11.6214 8.91341L13.4688 9.67951C13.2328 10.2486 12.924 10.7798 12.5538 11.2615Z';

function GearIcon({ color }: { color: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill={color} role="img" focusable="false" aria-hidden>
      <path d={GEAR_PATH} stroke="none" />
    </svg>
  );
}

/** A ring with a pie inside; `fraction` is how much of the pie is filled (0: an empty ring). */
function PieIcon({ color, fraction }: { color: string; fraction: number }) {
  const clamped = Math.max(0, Math.min(fraction, 0.9999));
  const angle = clamped * 2 * Math.PI;
  const x = 3.5 + 3.5 * Math.sin(angle);
  const y = 3.5 - 3.5 * Math.cos(angle);
  const largeArc = clamped > 0.5 ? 1 : 0;
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" role="img" focusable="false" aria-hidden>
      <rect x="1" y="1" width="12" height="12" rx="6" stroke={color} strokeWidth="1.5" />
      {clamped > 0 && (
        <path
          fill={color}
          stroke="none"
          d={`M 3.5,3.5 L3.5,0 A3.5,3.5 0 ${largeArc},1 ${x.toFixed(4)},${y.toFixed(4)} z`}
          transform="translate(3.5,3.5)"
        />
      )}
    </svg>
  );
}

/** A filled disc with a check (Circle's "Done"). */
function CheckIcon({ color }: { color: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" fill={color} role="img" focusable="false" aria-hidden>
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M7 0C3.13401 0 0 3.13401 0 7C0 10.866 3.13401 14 7 14C10.866 14 14 10.866 14 7C14 3.13401 10.866 0 7 0ZM11.101 5.10104C11.433 4.76909 11.433 4.23091 11.101 3.89896C10.7691 3.56701 10.2309 3.56701 9.89896 3.89896L5.5 8.29792L4.10104 6.89896C3.7691 6.56701 3.2309 6.56701 2.89896 6.89896C2.56701 7.2309 2.56701 7.7691 2.89896 8.10104L4.89896 10.101C5.2309 10.433 5.7691 10.433 6.10104 10.101L11.101 5.10104Z"
      />
    </svg>
  );
}

/** Each state's colour (Circle's: Backlog, Todo, In Progress, Technical Review, Done, Blocked). */
export const STATE_COLOUR: Record<TicketStateName, string> = {
  draft: '#95a2b3',
  confirmed: '#99a2b2',
  in_progress: '#facc15',
  waiting: '#22c55e',
  done: '#5e6ad2',
  blocked: '#eb5757',
};

export function StateIcon({ state }: { state: TicketStateName }) {
  const color = STATE_COLOUR[state];
  switch (state) {
    case 'draft':
      return <GearIcon color={color} />;
    case 'confirmed':
      return <PieIcon color={color} fraction={0} />;
    case 'in_progress':
      return <PieIcon color={color} fraction={0.25} />;
    case 'waiting':
      return <PieIcon color={color} fraction={0.4} />;
    case 'blocked':
      return <PieIcon color={color} fraction={0.65} />;
    case 'done':
      return <CheckIcon color={color} />;
  }
}

/**
 * The order of the list's groups: the Assistant's drafts on top for a person to accept (D-87),
 * then Linear's, started work first (Circle's display order).
 */
export const STATE_ORDER: TicketStateName[] = ['draft', 'in_progress', 'waiting', 'blocked', 'confirmed', 'done'];
