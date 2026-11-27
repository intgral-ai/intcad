import { StrictMode, useEffect } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './assistant.css';
import { installHostBehaviour, post, useAssistant } from './host';
import { Tickets } from './tickets/Tickets';
import { ThemeContext } from '@/ds/components/ThemeProvider/theme-context';
import { TooltipProvider } from '@/ds/components/Tooltip';

installHostBehaviour();

/** The page follows the app's theme, which arrives with the state. */
function Themed() {
  const pushed = useAssistant();
  // The host pushes either the Assistant's state or the tickets view's, marked by `view`.
  const tickets = pushed && 'view' in pushed ? pushed : null;
  const state = pushed && !('view' in pushed) ? pushed : null;
  const dark = pushed?.dark ?? true;
  const theme = dark ? 'dark' : 'light';
  useEffect(() => {
    document.documentElement.classList.remove('light', 'dark');
    document.documentElement.classList.add(theme);
    // The app fills the panel's frame with the page's own surface, so there is no seam around it:
    // the colour of Mastra's background in this theme, read as the browser paints it.
    const swatch = document.body.appendChild(document.createElement('div'));
    swatch.className = 'bg-background';
    const probe = document.createElement('canvas').getContext('2d');
    if (probe) {
      probe.fillStyle = getComputedStyle(swatch).backgroundColor;
      probe.fillRect(0, 0, 1, 1);
      const [r = 0, g = 0, b = 0] = probe.getImageData(0, 0, 1, 1).data;
      post({ type: 'surface', color: `#${[r, g, b].map(part => part.toString(16).padStart(2, '0')).join('')}` });
    }
    swatch.remove();
  }, [theme]);
  // With the built-in agent the page is Mastra's own size; the stylesheet has the two sizes. The
  // tickets view is always Mastra's size.
  const builtIn = Boolean(state?.connect) || Boolean(tickets);
  useEffect(() => {
    if (builtIn) document.documentElement.dataset.sizes = 'mastra';
    else delete document.documentElement.dataset.sizes;
  }, [builtIn]);
  return (
    <ThemeContext.Provider value={{ theme, resolvedTheme: theme, systemTheme: theme, setTheme: () => {} }}>
      <TooltipProvider>{tickets ? <Tickets state={tickets} /> : <App state={state} />}</TooltipProvider>
    </ThemeContext.Provider>
  );
}

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <Themed />
    </StrictMode>,
  );
}
