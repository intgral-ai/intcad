// The Assistant's page for the desktop app's web view: our glue in src/, Mastra's playground-ui
// components from a checkout of mastra-ai/mastra (see README.md for the commit and the rights
// status). Built into dist/ as one script, one stylesheet and the fonts, with fixed names, which
// apps/solidcraft embeds. Nothing is written into the Mastra checkout: run the build with
// `--configLoader native`.
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const here = dirname(fileURLToPath(import.meta.url));
const mastra = resolve(here, process.env.MASTRA_CHECKOUT ?? '../../../../open-source/mastra');
const ui = resolve(mastra, 'packages/playground-ui');

// Mastra's model picker has fixed English texts and no property for them. As the page is built,
// the ones this page can show are replaced by the page's own messages, which src/App.tsx sets
// (`assistantPickerText`); the English stays as the fallback. Each text must be found exactly
// once in its file and every one must be met, so a Mastra commit that words or moves one fails
// the build instead of leaving a text untranslated. Not listed, because this page never shows
// them: the trigger's own "Model: " and "Select model" (the page gives the trigger its content),
// the "Default" badge (the page names no default model) and the texts for providers that are not
// connected (the page lists none).
const PICKER_TEXTS = [
  ['domains/llm/model-picker-panel.tsx', 'placeholder="Search models"', 'placeholder={globalThis.assistantPickerText?.search ?? "Search models"}'],
  [
    'domains/llm/model-picker-panel.tsx',
    '<CommandEmpty>No matching model.</CommandEmpty>',
    '<CommandEmpty>{globalThis.assistantPickerText?.none ?? "No matching model."}</CommandEmpty>',
  ],
  ['domains/llm/model-picker-provider-filter.tsx', '>\n        All\n      </Button>', '>\n        {globalThis.assistantPickerText?.all ?? "All"}\n      </Button>'],
  [
    'domains/llm/model-picker-panel.tsx',
    'Refine the search to see more.',
    '{globalThis.assistantPickerText?.refine ?? "Refine the search to see more."}',
  ],
  [
    'domains/llm/model-picker-provider-filter.tsx',
    'aria-label="Filter by provider"',
    'aria-label={globalThis.assistantPickerText?.filter ?? "Filter by provider"}',
  ],
  [
    'domains/llm/model-picker.tsx',
    '<DrawerTitle className="sr-only">Choose a model</DrawerTitle>',
    '<DrawerTitle className="sr-only">{globalThis.assistantPickerText?.title ?? "Choose a model"}</DrawerTitle>',
  ],
];

function pickerTexts() {
  const met = new Set();
  return {
    name: 'assistant-web-picker-texts',
    enforce: 'pre',
    transform(code, id) {
      const file = id.split('?')[0].replaceAll('\\', '/');
      const texts = [...PICKER_TEXTS.entries()].filter(([, [path]]) => file.endsWith(`/playground-ui/src/${path}`));
      if (texts.length === 0) return null;
      let out = code.replace(/\r\n/g, '\n');
      for (const [index, [path, from, to]] of texts) {
        if (out.split(from).length !== 2) this.error(`${path}: expected ${JSON.stringify(from)} exactly once (see PICKER_TEXTS in vite.config.mjs)`);
        out = out.replace(from, to);
        met.add(index);
      }
      return { code: out, map: null };
    },
    buildEnd(error) {
      if (!error && met.size !== PICKER_TEXTS.length) this.error(`the model picker's texts were met ${met.size} times, not ${PICKER_TEXTS.length} (see PICKER_TEXTS in vite.config.mjs)`);
    },
  };
}

export default defineConfig({
  root: here,
  base: './',
  cacheDir: resolve(here, '.vite-cache'),
  plugins: [pickerTexts(), react(), tailwindcss()],
  resolve: {
    alias: [{ find: /^@\//, replacement: `${ui}/src/` }],
  },
  build: {
    outDir: resolve(here, 'dist'),
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: false,
    chunkSizeWarningLimit: 4000,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
        entryFileNames: 'assistant.js',
        assetFileNames: '[name][extname]',
      },
    },
  },
});
