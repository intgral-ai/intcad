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

export default defineConfig({
  root: here,
  base: './',
  cacheDir: resolve(here, '.vite-cache'),
  plugins: [react(), tailwindcss()],
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
