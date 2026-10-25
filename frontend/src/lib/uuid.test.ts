import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { fromRandomBytes } from './uuid.js';

describe('uuid', () => {
	it('builds a version 4 id from random bytes', () => {
		const id = fromRandomBytes(new Uint8Array(16).fill(0xff));
		expect(id).toBe('ffffffff-ffff-4fff-bfff-ffffffffffff');
		expect(fromRandomBytes(new Uint8Array(16))).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
	});

	// A phone on the network is not a secure context, so `crypto.randomUUID` is not
	// there and the call throws. One direct call in a page's start-up path is a console
	// that works on the desk and not on a tablet.
	it('is the only way the page makes an id', () => {
		const root = join(__dirname, '..');
		const offenders: string[] = [];
		const walk = (dir: string) => {
			for (const name of readdirSync(dir)) {
				const path = join(dir, name);
				if (statSync(path).isDirectory()) {
					if (name !== 'generated' && name !== 'evaluator') walk(path);
				} else if (/\.(ts|svelte)$/.test(name) && !/\.test\.ts$/.test(name) && !path.endsWith(join('lib', 'uuid.ts'))) {
					if (readFileSync(path, 'utf8').includes('crypto.randomUUID')) offenders.push(path.slice(root.length + 1));
				}
			}
		};
		walk(root);
		expect(offenders).toEqual([]);
	});
});
