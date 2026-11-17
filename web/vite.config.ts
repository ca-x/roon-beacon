import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  base: '/assets/',
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  esbuild: { legalComments: 'none' },
  build: {
    assetsInlineLimit: (filePath) => /\.woff2?$/.test(filePath) ? true : undefined,
    outDir: '../crates/roon-beacon/src/web',
    emptyOutDir: true,
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        entryFileNames: 'app.js',
        assetFileNames: 'styles.css',
        inlineDynamicImports: true
      }
    }
  }
});
