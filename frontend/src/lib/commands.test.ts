import { describe, expect, it } from 'vitest';
import { writable } from 'svelte/store';

import { allCommands, objectCommands, registerCommands, searchCommands } from './commands.js';
import type { ObjectKind } from './objects.js';

describe('the command registry', () => {
	it('asks every source each time, so a new object is findable at once', () => {
		let names = ['Warm'];
		const off = registerCommands('test', () =>
			names.map((n) => ({ id: n, title: n, section: 'Objects' as const, run: () => {} }))
		);
		expect(allCommands().map((c) => c.title)).toContain('Warm');
		names = ['Warm', 'Cold'];
		expect(allCommands().map((c) => c.title)).toContain('Cold');
		off();
		expect(allCommands().map((c) => c.title)).not.toContain('Cold');
	});

	it('loses only the commands of a source that throws', () => {
		const bad = registerCommands('bad', () => {
			throw new Error('no');
		});
		const good = registerCommands('good', () => [{ id: 'g', title: 'Good', section: 'Console', run: () => {} }]);
		expect(allCommands().map((c) => c.id)).toContain('g');
		bad();
		good();
	});

	it('lists an object once to act and once to inspect', () => {
		const presets = writable([{ id: 'p', name: 'Warm' }]);
		const kind = {
			kind: 'preset',
			label: 'Preset',
			category: 'program',
			icon: {} as never,
			rows: presets,
			actLabel: 'Apply',
			act: () => {}
		} as ObjectKind;
		const commands = objectCommands(new Map([['preset', kind]]));
		expect(commands.map((c) => c.hint)).toEqual(['Apply', 'Inspect']);
	});

	it('finds by keywords as well as titles', () => {
		const commands = [
			{ id: 'a', title: 'Warm', section: 'Objects' as const, keywords: 'Preset', run: () => {} },
			{ id: 'b', title: 'Stations', section: 'Panels' as const, run: () => {} }
		];
		expect(searchCommands('preset', commands).map((c) => c.id)).toEqual(['a']);
	});
});
