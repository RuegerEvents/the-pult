import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

/**
 * In a release the console serves this app itself, so the page and the socket
 * share an origin and the frontend never has to be told a port. The dev server
 * proxies the same prefixes through to a running backend so that dev works the
 * same way rather than being the one arrangement that needs a query string.
 *
 * `/stock` is one of them and was missed when it was added, which is worth the
 * note: an unproxied prefix does not 404 here, it returns the SPA's own HTML —
 * so the loader got a page where a `.glb` should have been, failed, and every
 * truss in the rig came back as `geometry.ts`'s placeholder cube. A prefix the
 * station serves has to be listed here or dev quietly draws something else.
 */
const backend = process.env.PULT_BACKEND ?? 'http://localhost:7700';

export default defineConfig({
	plugins: [sveltekit()],
	server: {
		proxy: {
			'/ws': { target: backend, ws: true },
			'/assets': { target: backend },
			'/stock': { target: backend },
			'/api': { target: backend }
		}
	}
});
