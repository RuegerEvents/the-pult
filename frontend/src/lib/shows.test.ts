import { describe as group, expect, it } from 'vitest';

import {
	asSize,
	baseName,
	describe,
	folderName,
	links,
	orderRecent,
	parentPath,
	shouldReload,
	type ShowSummary
} from './shows.js';
import type { BackendConfig } from './ws/endpoint.js';

function a_config(show: string | null): BackendConfig {
	return {
		wsPath: '/ws',
		port: 7700,
		syncPort: 7701,
		nodeId: 'n',
		version: '0.1.0',
		show: show === null ? null : { path: show, name: baseName(show) },
		showsDir: '/shows',
		repository: 'https://example.invalid/the-pult'
	};
}

group('the reload rule', () => {
	it('reloads when the station has moved to a different show', () => {
		// Opening a show is the station stopping and another starting in its place,
		// so every store in this tab is holding the last show's rig.
		expect(shouldReload(a_config('/shows/A.pult'), a_config('/shows/B.pult'))).toBe(true);
	});

	it('reloads when a show was closed, and when one was opened', () => {
		expect(shouldReload(a_config('/shows/A.pult'), a_config(null))).toBe(true);
		expect(shouldReload(a_config(null), a_config('/shows/A.pult'))).toBe(true);
	});

	it('does not reload when nothing moved', () => {
		expect(shouldReload(a_config('/shows/A.pult'), a_config('/shows/A.pult'))).toBe(false);
		expect(shouldReload(a_config(null), a_config(null))).toBe(false);
	});

	it('does not reload on the first answer, or on one that never came', () => {
		// A page that has just loaded has nothing to compare against, and reloading
		// then would be a loop.
		expect(shouldReload(null, a_config('/shows/A.pult'))).toBe(false);
		expect(shouldReload(a_config('/shows/A.pult'), null)).toBe(false);
	});
});

group('naming a show', () => {
	it('keeps what an operator typed, and replaces only what a filesystem misreads', () => {
		expect(folderName('Hänsel & Gretel')).toBe('Hänsel & Gretel.pult');
		expect(folderName('Act 1/2')).toBe('Act 1-2.pult');
		expect(folderName('../etc')).toBe('-etc.pult');
		expect(folderName('   ')).toBe('Untitled Show.pult');
	});

	it('reads a path back as a name and a place', () => {
		expect(baseName('/shows/Panto.pult')).toBe('Panto.pult');
		expect(parentPath('/shows/Panto.pult')).toBe('/shows');
		expect(parentPath('Panto.pult')).toBe('');
	});
});

group('the recent list', () => {
	const at = (path: string, lastOpened: string, missing = false): ShowSummary => ({
		path,
		name: path,
		lastOpened,
		missing
	});

	it('puts the most recent first', () => {
		const order = orderRecent([
			at('/a', '2026-01-01T00:00:00Z'),
			at('/b', '2026-03-01T00:00:00Z'),
			at('/c', '2026-02-01T00:00:00Z')
		]);
		expect(order.map((s) => s.path)).toEqual(['/b', '/c', '/a']);
	});

	it('keeps a show that has gone, at the bottom', () => {
		// A show on a stick that is not plugged in is exactly the row somebody is
		// looking for; a list that forgot it would be one nobody could rely on.
		const order = orderRecent([
			at('/gone', '2026-04-01T00:00:00Z', true),
			at('/here', '2026-01-01T00:00:00Z')
		]);
		expect(order.map((s) => s.path)).toEqual(['/here', '/gone']);
	});
});

group('what a card says', () => {
	it('counts what is in the show', () => {
		expect(describe({ path: '/a', name: 'A', fixtures: 5, cues: 3 })).toBe('5 fixtures · 3 cues');
		expect(describe({ path: '/a', name: 'A', fixtures: 1, cues: 1, versions: 2 })).toBe(
			'1 fixture · 1 cue · 2 saved'
		);
	});

	it('says what is wrong instead, when something is', () => {
		expect(describe({ path: '/a', name: 'A', missing: true })).toBe('not where it was');
		expect(describe({ path: '/a', name: 'A', madeByAnotherBuild: true })).toContain(
			'another version'
		);
		expect(describe({ path: '/a', name: 'A', problem: 'disk error' })).toBe('disk error');
	});

	it('says a size the way a person would', () => {
		expect(asSize(0)).toBe('—');
		expect(asSize(512)).toBe('512 B');
		expect(asSize(2048)).toBe('2.0 kB');
		expect(asSize(5 * 1024 * 1024)).toBe('5.0 MB');
	});
});

group('the links', () => {
	it('are built from the repository the binary carries', () => {
		// So a fork's console points at the fork rather than at this one.
		const built = links('https://example.invalid/someone/the-pult.git');
		expect(built.map((l) => l.href)).toEqual([
			'https://example.invalid/someone/the-pult',
			'https://example.invalid/someone/the-pult/issues',
			'https://example.invalid/someone/the-pult/releases',
			'https://example.invalid/someone/the-pult/blob/main/docs/SPEC.md'
		]);
	});

	it('offer nothing when the binary was built without one', () => {
		expect(links('')).toEqual([]);
	});
});
