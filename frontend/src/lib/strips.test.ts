import { describe, expect, it } from 'vitest';
import type { DiscoveredDevice, Fixture, PixelStrip } from './generated/index.js';
import { stripRows, stripSummary } from './strips.js';

const strip = (mode: 'effect' | 'sacn', count = 3): PixelStrip => ({
	output: 0,
	mode,
	count,
	order: 'GRB',
	universe: 5,
	address: 1,
	hold_ms: 10000,
	pitch_m: 0.1
});

function fixture(id: string, address: Fixture['address'], extra: Partial<Fixture> = {}): Fixture {
	return {
		id,
		name: id,
		fixture_type_id: 't',
		address,
		position: null,
		parent: null,
		mount: null,
		layer: null,
		class: null,
		focus: null,
		fixture_number: null,
		unit_number: null,
		sensed_values: {},
		live_effects: {},
		live_fades: {},
		home_values: {},
		...extra
	} as Fixture;
}

const device = {
	serial: 'n1',
	strip_objects: { 1: 'obj1' },
	pixels: { strips: [{ head: 1, mode: 'effect' }] }
} as unknown as DiscoveredDevice;

describe('a node’s strips', () => {
	it('lists each strip head with its cells in order along it', () => {
		const head = fixture('h1', { OpenHaunt: { serial: 'n1', universe: null, head: 1, strip: strip('sacn') } });
		const node = fixture('h0', { OpenHaunt: { serial: 'n1', universe: null, head: null, strip: null } });
		const cell = (n: number, universe: number) =>
			fixture(`c${n}`, { Dmx: { mode: 'Default', breaks: [{ universe, address: 1 + 3 * n }] } }, {
				parent: 'obj1',
				unit_number: n + 1
			});
		const groups = [{ id: 'g1', query: { clauses: [{ term: { kind: 'Hanging', object: 'obj1' } }] } }];
		const rows = stripRows(device, [cell(2, 6), node, head, cell(0, 5), cell(1, 5)], groups);
		expect(rows).toHaveLength(1);
		expect(rows[0].cells.map((c) => c.id)).toEqual(['c0', 'c1', 'c2']);
		expect(rows[0].groupId).toBe('g1');
		expect(rows[0].running?.mode).toBe('effect');
		expect(stripSummary(rows[0].strip, rows[0].cells)).toBe('3 GRB pixels in universes 5–6');
	});

	it('says who draws a strip in effect mode', () => {
		expect(stripSummary(strip('effect', 60), [])).toBe('60 GRB pixels, drawn by the node');
	});
});
