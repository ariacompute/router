import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

// Separate from vite.config.ts so a production `vite build` never needs the
// vitest dev dependency. Vitest prefers this file automatically.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: false,
    include: ['tests/**/*.spec.{ts,tsx}'],
    css: false,
    restoreMocks: true,
  },
});
