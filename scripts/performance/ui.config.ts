import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { resolve } from 'node:path';
export default defineConfig({plugins:[vue()],build:{target:'chrome105',minify:'esbuild',outDir:process.env.SF_UI_OUTPUT,emptyOutDir:true,rollupOptions:{input:resolve('scripts/performance/index.html')}}});
