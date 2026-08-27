import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The panel is bundled into the app, so it is never served over a network and
// never needs a base path. Tauri watches port 1420 in dev.
export default defineConfig({
	plugins: [svelte()],
	clearScreen: false,
	server: { port: 1420, strictPort: true }
});
