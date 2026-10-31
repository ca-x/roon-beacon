import { defineConfig } from 'vite';

export default defineConfig({
  base: '/assets/',
  esbuild: { legalComments: 'none' },
  build: {
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
