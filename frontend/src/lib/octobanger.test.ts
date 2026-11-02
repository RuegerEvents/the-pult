import { describe, expect, it } from 'vitest';

import type { Fixture, FixtureType } from '$lib/generated/index.js';
import { DISCARD, clashes, defaultEdge, fillFrom, portId, switchOutputs, triggerInputs } from './octobanger.js';

function v0(id: string, name: string): { fixture: Fixture; type: FixtureType } {
	const relay = (n: number) => ({
		kind: { Switch: n },
		direction: 'Output',
		binding: { Port: { index: n } },
		default_value: { type: 'Bool', value: false },
		physical: null,
		slots: [],
		feature_group: null,
		emitters: [],
		per_cell: false
	});
	const type = {
		id: `type-${id}`,
		parameters: [
			...[0, 1, 2, 3].map(relay),
			{ ...relay(4), kind: { Contact: 4 }, direction: 'Input' },
			{ ...relay(5), kind: 'Intensity', default_value: { type: 'Float', value: 0 } }
		]
	} as unknown as FixtureType;
	const fixture = { id, name, fixture_type_id: type.id } as unknown as Fixture;
	return { fixture, type };
}

const a = v0('a', 'Area 1');
const b = v0('b', 'Area 1b');
const fixtures = [a.fixture, b.fixture];
const types = [a.type, b.type];

describe('octobanger', () => {
	it('offers the outputs that switch and nothing else', () => {
		const outputs = switchOutputs(fixtures, types);
		expect(outputs.map((o) => o.key)).toEqual(['Switch:0', 'Switch:1', 'Switch:2', 'Switch:3', 'Switch:0', 'Switch:1', 'Switch:2', 'Switch:3']);
		expect(triggerInputs(fixtures, types).map((o) => o.key)).toEqual(['Contact:4', 'Contact:4']);
	});

	it('puts eight channels onto two four-relay nodes', () => {
		const outputs = switchOutputs(fixtures, types);
		const used = [1, 2, 3, 4, 5, 6, 7, 8];
		const empty = Object.fromEntries(used.map((c) => [c, null]));
		const first = fillFrom(empty, used, outputs, 'a');
		expect(first[4]).toBe('a|Switch:3');
		expect(first[5]).toBeNull();
		const both = fillFrom(first, used, outputs, 'b');
		expect(both[5]).toBe('b|Switch:0');
		expect(both[8]).toBe('b|Switch:3');
	});

	it('fills past a discarded channel rather than into it', () => {
		const outputs = switchOutputs(fixtures, types);
		const filled = fillFrom({ 1: DISCARD, 2: null }, [1, 2], outputs, 'a');
		expect(filled).toEqual({ 1: DISCARD, 2: 'a|Switch:0' });
	});

	it('fills around what is already given out', () => {
		const outputs = switchOutputs(fixtures, types);
		const assigned = { 1: null, 2: portId({ fixtureId: 'a', key: 'Switch:0' }), 3: null };
		const filled = fillFrom(assigned, [1, 2, 3], outputs, 'a');
		expect(filled).toEqual({ 1: 'a|Switch:1', 2: 'a|Switch:0', 3: 'a|Switch:2' });
	});

	it('says which channels share an output', () => {
		expect([...clashes({ 1: 'a|Switch:0', 2: 'a|Switch:0', 3: 'a|Switch:1', 4: null })]).toEqual([1, 2]);
		expect(clashes({ 1: DISCARD, 2: DISCARD }).size, 'two discarded channels share nothing').toBe(0);
	});

	it('fires a contact to ground on the way down', () => {
		expect(defaultEdge('Hi')).toBe('FallingEdge');
		expect(defaultEdge('Low')).toBe('RisingEdge');
	});
});
