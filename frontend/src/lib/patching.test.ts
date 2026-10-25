import { describe, expect, it } from 'vitest';

import type { Fixture } from '$lib/generated/index.js';
import { addressRun, freeRun, namesFor, nextNumber, patchOrder, shifted, universeCells, universesInUse } from './patching.js';

function at(id: string, universe: number, address: number, name = id): Fixture {
	return { id, name, address: { Dmx: { mode: 'Default', breaks: [{ universe, address }] } } } as unknown as Fixture;
}
function node(id: string, name = id): Fixture {
	return { id, name, address: { OpenHaunt: { serial: 'f674', head: 0 } } } as unknown as Fixture;
}
const eight = () => 8;

describe('freeRun', () => {
	it('finds a gap in the middle, not only after the last fixture', () => {
		const rig = [at('a', 1, 1), at('b', 1, 41)];
		expect(freeRun(rig, 1, 16, eight)).toBe(9);
		expect(freeRun(rig, 1, 40, eight)).toBe(49);
	});
	it('is null where the universe has no such run', () => {
		expect(freeRun([at('a', 1, 1)], 1, 510, eight)).toBeNull();
		expect(freeRun([], 1, 513, eight)).toBeNull();
	});
	it('ignores other universes', () => {
		expect(freeRun([at('a', 2, 1)], 1, 8, eight)).toBe(1);
	});
});

describe('addressRun', () => {
	it('puts them one after another', () => {
		expect(addressRun(1, 1, 8, 3)).toEqual([
			{ universe: 1, address: 1 },
			{ universe: 1, address: 9 },
			{ universe: 1, address: 17 }
		]);
	});
	it('starts the next universe rather than splitting a fixture over the end', () => {
		expect(addressRun(1, 497, 8, 3)).toEqual([
			{ universe: 1, address: 497 },
			{ universe: 1, address: 505 },
			{ universe: 2, address: 1 }
		]);
	});
});

describe('namesFor', () => {
	it('numbers {n} from where it is told', () => {
		expect(namesFor('Spot {n} SR', 2, 5)).toEqual(['Spot 5 SR', 'Spot 6 SR']);
	});
	it('puts a number on the end of several, and leaves one alone', () => {
		expect(namesFor('Par', 3)).toEqual(['Par 1', 'Par 2', 'Par 3']);
		expect(namesFor('Gate par', 1)).toEqual(['Gate par']);
	});
	it('carries on after what is already patched', () => {
		expect(nextNumber([at('a', 1, 1, 'Spot 1'), at('b', 1, 9, 'Spot 4'), at('c', 1, 17, 'Spotlight')], 'Spot {n}')).toBe(5);
		expect(nextNumber([], 'Spot')).toBe(1);
	});
});

describe('universeCells', () => {
	it('marks each channel with what is on it, two where they clash', () => {
		const cells = universeCells([at('a', 1, 1), at('b', 1, 5)], 1, eight);
		expect(cells[0]).toEqual(['a']);
		expect(cells[5]).toEqual(['a', 'b']);
		expect(cells[11]).toEqual(['b']); // channel 12, the last of b
		expect(cells[12]).toEqual([]);
	});
});

describe('the rest', () => {
	it('keeps a dragged fixture inside the universe', () => {
		expect(shifted(10, 8, -50)).toBe(1);
		expect(shifted(500, 8, 20)).toBe(505);
		expect(shifted(10, 8, 5)).toBe(15);
	});
	it('lists by universe and address, and the nodes after', () => {
		const order = patchOrder([node('n'), at('b', 2, 1), at('a', 1, 9), at('c', 1, 1)]).map((f) => f.id);
		expect(order).toEqual(['c', 'a', 'b', 'n']);
		expect(universesInUse([at('b', 3, 1), node('n'), at('a', 1, 1)])).toEqual([1, 3]);
	});
});
