import { describe, it, expect } from 'vitest';
import { backendOrigin, fetchConfig, wsUrl, wsUrlFrom } from './endpoint.js';

const at = (origin: string, search = '') => ({ origin, search });

describe('finding the backend', () => {
	it('uses the origin the page came from', () => {
		expect(backendOrigin(at('http://localhost:7700'))).toBe('http://localhost:7700');
		expect(backendOrigin(at('http://10.0.0.9:7700'))).toBe('http://10.0.0.9:7700');
	});

	it('lets ?port name a second station on the same host', () => {
		expect(backendOrigin(at('http://localhost:5173', '?port=7710'))).toBe('http://localhost:7710');
	});

	it('keeps the host when the port is overridden', () => {
		// The second station is on this machine, not on localhost as seen from a tablet.
		expect(backendOrigin(at('http://10.0.0.9:7700', '?port=7710'))).toBe('http://10.0.0.9:7710');
	});

	it('ignores a ?port that is not one', () => {
		expect(backendOrigin(at('http://localhost:5173', '?port=nowhere'))).toBe(
			'http://localhost:5173'
		);
	});
});

describe('naming the socket', () => {
	it('follows the origin from http to ws', () => {
		expect(wsUrlFrom('http://localhost:7700', '/ws')).toBe('ws://localhost:7700/ws');
	});

	it('follows it from https to wss, so a proxied console is not downgraded', () => {
		expect(wsUrlFrom('https://console.example', '/ws')).toBe('wss://console.example/ws');
	});

	it('needs nothing but the page it is on', () => {
		expect(wsUrl(at('http://10.0.0.9:7700'))).toBe('ws://10.0.0.9:7700/ws');
		expect(wsUrl(at('http://localhost:5173', '?port=7710'))).toBe('ws://localhost:7710/ws');
	});
});

describe('asking the station', () => {
	it('reads the config it answers with', async () => {
		const config = { wsPath: '/ws', port: 7700, syncPort: 7701, nodeId: 'n', version: '0.1.0' };
		const got = await fetchConfig('http://localhost:7700', async () =>
			Response.json(config)
		);
		expect(got).toEqual(config);
	});

	it('gives up quietly when there is nothing there', async () => {
		expect(
			await fetchConfig('http://localhost:7700', async () => {
				throw new Error('connection refused');
			})
		).toBeNull();
		expect(
			await fetchConfig('http://localhost:7700', async () => new Response('', { status: 502 }))
		).toBeNull();
	});
});
