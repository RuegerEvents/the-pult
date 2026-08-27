import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

/**
 * In a release the console serves this app itself, so the page and the socket
 * share an origin and the frontend never has to be told a port. The dev server
 * proxies the same three prefixes through to a running backend so that dev works
 * the same way rather than being the one arrangement that needs a query string.
 */
const backend = process.env.PULT_BACKEND ?? 'http://localhost:7700';

export default defineConfig({
	plugins: [sveltekit()],
	server: {
		proxy: {
			'/ws': { target: backend, ws: true },
			'/assets': { target: backend },
			'/api': { target: backend }
		}
	}
});
