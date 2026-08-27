import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		adapter: adapter({
			fallback: 'index.html', // SPA mode
			// The build is embedded in the backend binary and served from it, so it
			// is squeezed once here rather than on every request from every tablet
			// in the room. The server picks the .br or the .gz off Accept-Encoding.
			precompress: true
		})
	}
};

export default config;
