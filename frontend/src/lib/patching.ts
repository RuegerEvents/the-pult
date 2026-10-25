/**
 * Patching several lights at once, and drawing a universe: the arithmetic the patch
 * panel, its universe map and the *Add fixtures* dialog share.
 *
 * Kept apart from `patch.ts`, which mirrors what the station works out about one
 * fixture. None of this is the station's: where the next run of four spots goes, what
 * they are called and which channels a universe has free are the browser's proposals,
 * written as ordinary addresses once somebody presses *Patch*.
 */

import type { Fixture } from '$lib/generated/index.js';
import { dmxAddress } from '$lib/patch.js';

/** The channels in a DMX universe. */
export const CHANNELS = 512;

type Span = (fixture: Fixture) => number;

/** What occupies a universe, first break only — the same rule `clashingFixtures` reads. */
export function occupied(fixtures: Fixture[], universe: number, span: Span): { id: string; start: number; end: number }[] {
	return fixtures.flatMap((fixture) => {
		const dmx = dmxAddress(fixture.address);
		if (!dmx || dmx.universe !== universe) return [];
		return [{ id: fixture.id, start: dmx.address, end: dmx.address + Math.max(span(fixture), 1) - 1 }];
	});
}

/**
 * The first address from `from` with `need` channels free in a row, or `null` where
 * the universe has no such gap. A gap, not the end of the last fixture: a universe
 * somebody unpatched the middle of has room in the middle.
 */
export function freeRun(fixtures: Fixture[], universe: number, need: number, span: Span, from = 1): number | null {
	if (need > CHANNELS) return null;
	const taken = occupied(fixtures, universe, span).sort((a, b) => a.start - b.start);
	let at = Math.max(1, from);
	for (const block of taken) {
		if (block.end < at) continue;
		if (block.start - at >= need) return at;
		at = Math.max(at, block.end + 1);
	}
	return at + need - 1 <= CHANNELS ? at : null;
}

/**
 * Where `count` fixtures of `width` channels go, one after another from `start`. A
 * fixture that would run past channel 512 starts the next universe at 1 instead of
 * being split, which no DMX fixture can be.
 */
export function addressRun(universe: number, start: number, width: number, count: number): { universe: number; address: number }[] {
	const out: { universe: number; address: number }[] = [];
	let u = universe;
	let at = Math.max(1, start);
	const w = Math.max(1, width);
	for (let i = 0; i < count; i++) {
		if (at + w - 1 > CHANNELS) {
			u += 1;
			at = 1;
		}
		out.push({ universe: u, address: at });
		at += w;
	}
	return out;
}

/**
 * Names for `count` new fixtures. `{n}` is the number, counted from `first`; a name
 * with no `{n}` gets the number on the end when there is more than one, so four
 * "Spot" are "Spot 1" to "Spot 4" rather than four lights nobody can tell apart.
 */
export function namesFor(pattern: string, count: number, first = 1): string[] {
	const base = pattern.trim();
	return Array.from({ length: count }, (_, i) => {
		const n = String(first + i);
		if (base.includes('{n}')) return base.replaceAll('{n}', n);
		return count === 1 ? base : `${base} ${n}`;
	});
}

/** The first number after the highest one already ending a name with this stem. */
export function nextNumber(fixtures: Fixture[], pattern: string): number {
	const stem = pattern.includes('{n}') ? pattern.slice(0, pattern.indexOf('{n}')).trim() : pattern.trim();
	if (!stem) return 1;
	let highest = 0;
	for (const fixture of fixtures) {
		const name = fixture.name.trim();
		if (!name.startsWith(stem)) continue;
		const tail = Number(name.slice(stem.length).trim());
		if (Number.isInteger(tail) && tail > highest) highest = tail;
	}
	return highest + 1;
}

/** A universe's channels, each with the fixtures on it: none, one, or a clash. */
export function universeCells(fixtures: Fixture[], universe: number, span: Span): string[][] {
	const cells: string[][] = Array.from({ length: CHANNELS }, () => []);
	for (const block of occupied(fixtures, universe, span)) {
		for (let c = block.start; c <= Math.min(block.end, CHANNELS); c++) cells[c - 1].push(block.id);
	}
	return cells;
}

/** The universes any fixture is in, in order. */
export function universesInUse(fixtures: Fixture[]): number[] {
	const set = new Set<number>();
	for (const fixture of fixtures) {
		const dmx = dmxAddress(fixture.address);
		if (dmx) set.add(dmx.universe);
	}
	return [...set].sort((a, b) => a - b);
}

/**
 * A fixture moved by `delta` channels, kept wholly inside the universe: dragged past
 * either end it stops there rather than wrapping, which would be a light landing
 * somewhere nobody dragged it.
 */
export function shifted(address: number, width: number, delta: number): number {
	return Math.min(Math.max(1, address + delta), CHANNELS - Math.max(1, width) + 1);
}

/** The list order: by universe and address, then the fixtures with no DMX address, by name. */
export function patchOrder(fixtures: Fixture[]): Fixture[] {
	return [...fixtures].sort((a, b) => {
		const da = dmxAddress(a.address);
		const db = dmxAddress(b.address);
		if (da && db) return da.universe - db.universe || da.address - db.address || a.name.localeCompare(b.name);
		if (da) return -1;
		if (db) return 1;
		return a.name.localeCompare(b.name);
	});
}
