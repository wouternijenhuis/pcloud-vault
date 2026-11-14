import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
// The desktop configuration is the single source of truth for the app version.
import desktopConfig from '../src-tauri/tauri.conf.json'

export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify(desktopConfig.version) },
  test: { environment: 'jsdom', setupFiles: './src/test-setup.ts' },
})
