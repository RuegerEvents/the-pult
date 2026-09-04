/**
 * The mounts corpus, from the browser's side.
 *
 * `testdata/mounts.json` is read by this and by `crates/pult-schema/tests/mounts.rs`.
 * The browser is the *writer* of a mount, because resolving one on a truss out of a
 * drawing means measuring its mesh and only this side ever loads one — so the station
 * is checking work done here, and this file is what says the two agree about it.
 */
import { readFileSync } from 'node:fs';

import { describe, it, expect } from 'vitest';

import type { Mount, Vec3 } from './generated/index.js';
import { chordsFor, mountPoint, mountTransform, nearestMount, type Chord } from './mount.js';
import { piece } from './stock.js';

type Placed = { name: string; chords: string; mount: Mount; transform: { position: Vec3; rotation: Vec3 } };
type Nearest = { name: string; chords: string; point: Vec3; mount: Mount; distance: number | null };

const corpus: {
	chordSets: Record<string, Chord[]>;
	transforms: Placed[];
	nearest: Nearest[];
} = JSON.parse(readFileSync(new URL('../../../testdata/mounts.json', import.meta.url), 'utf8'));

function near(got: Vec3, want: Vec3, what: string, name: string) {
	for (const axis of ['x', 'y', 'z'] as const) {
		expect(
			Math.abs(got[axis] - want[axis]),
			`${name}: ${what}.${axis} is ${got[axis]}, expected ${want[axis]}`
		).toBeLessThan(1e-3);
	}
}

describe('resolving a clamp', () => {
	for (const each of corpus.transforms) {
		it(each.name, () => {
			const got = mountTransform(each.mount, corpus.chordSets[each.chords]);
			near(got.position, each.transform.position, 'position', each.name);
			near(got.rotation, each.transform.rotation, 'rotation', each.name);
		});
	}
});

describe('finding the clamp nearest a point', () => {
	for (const each of corpus.nearest) {
		it(each.name, () => {
			const { mount, distance } = nearestMount(each.point, corpus.chordSets[each.chords]);
			if (each.distance === null) {
				expect(Number.isFinite(distance)).toBe(false);
				return;
			}
			expect(mount.chord, `${each.name}: the wrong chord`).toBe(each.mount.chord);
			expect(Math.abs(mount.along - each.mount.along)).toBeLessThan(1e-3);
			expect(Math.abs(mount.roll - each.mount.roll)).toBeLessThan(1e-3);
			expect(Math.abs(distance - each.distance)).toBeLessThan(1e-3);
		});
	}
});

describe('the chords a light can be dragged onto', () => {
	it('are the catalogue piece’s own where there is one', () => {
		const truss = piece('f34-2m');
		expect(chordsFor(truss, null)).toHaveLength(4);
		// A corner is 290 mm of block, and a clamp on it would be a light bolted to a
		// joint — so nothing hangs off one.
		expect(chordsFor(piece('f34-corner'), null)).toHaveLength(0);
	});

	it('are one line off the bounds of anything else', () => {
		const chords = chordsFor(undefined, { x: 6, y: 0.4, z: 0.4 });

		expect(chords).toHaveLength(1);
		expect(chords[0].at.y).toBeCloseTo(-0.2);
	});

	it('are nothing at all for a piece nobody has measured', () => {
		expect(chordsFor(undefined, null)).toHaveLength(0);
	});
});

describe('a clamp on a bar', () => {
	// The round trip a drag rests on: sliding a light along a bar has to keep it on
	// the bar, which means the point a mount resolves to snaps back to that mount.
	it('comes back as the mount that made it', () => {
		const chords = piece('f34-3m')!.chords;
		for (let chord = 0; chord < 4; chord++) {
			for (const roll of [0, 90, 180, 270]) {
				const mount: Mount = { chord, along: 0.6, roll };
				const { mount: back, distance } = nearestMount(mountPoint(mount, chords), chords);
				expect(distance, `chord ${chord} roll ${roll}`).toBeLessThan(1e-3);
				expect(back.chord).toBe(chord);
				expect(back.roll).toBeCloseTo(roll);
				expect(back.along).toBeCloseTo(0.6);
			}
		}
	});
});
