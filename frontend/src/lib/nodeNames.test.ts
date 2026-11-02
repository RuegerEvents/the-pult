import { describe, expect, it } from 'vitest';
import type { DiscoveredDevice, Fixture, FixtureType } from './generated/index.js';
import { headLabel, namesInShow, namesOnNode, portRows, tooLong, withLabel } from './nodeNames.js';
import { parameterName } from './patch.js';

const contact = (n: number) => ({
	kind: { Contact: n },
	direction: 'Input',
	binding: { Port: { index: n } },
	default_value: { type: 'Bool', value: false }
});

const types = [
	{ id: 'in8', name: 'Digital Inputs', parameters: [contact(0), contact(1)] },
	{ id: 'strip', name: 'Strip 1', parameters: [] }
] as unknown as FixtureType[];

const fixture = (id: string, serial: string, head: number | null, name: string, labels: Record<string, string> | null) =>
	({
		id,
		name,
		fixture_type_id: head ? 'strip' : 'in8',
		address: { OpenHaunt: { serial, universe: null, head, strip: null, redundant_stream: null } },
		parameter_labels: labels
	}) as unknown as Fixture;

const device = (serial: string, described: boolean) =>
	({
		serial,
		name: `Node ${serial}`,
		description: described
			? {
					ports: [
						{ port: 0, name: 'Input 1', access: 'readonly', dataType: 'boolean', unit: null, head: null },
						{ port: 1, name: 'Input 2', access: 'readonly', dataType: 'boolean', unit: null, head: null }
					],
					dmx: null,
					heads: [{ head: 1, name: 'Strip 1', kind: 'pixel-strip' }]
				}
			: null,
		labels: { ports: { 1: 'Relay Horn' }, heads: { 1: 'LED Streifen Tor' } }
	}) as unknown as DiscoveredDevice;

describe('the names of a node', () => {
	const fixtures = [
		fixture('own', 'a1', null, 'Hof', { 'Contact:0': 'Ausgang PIR' }),
		fixture('s1', 'a1', 1, 'Hof Strip 1', null)
	];

	it('reads a port row with its fixture, key and label', () => {
		const rows = portRows(device('a1', true), fixtures, types);
		expect(rows.map((r) => [r.port, r.name, r.key, r.label])).toEqual([
			[0, 'Input 1', 'Contact:0', 'Ausgang PIR'],
			[1, 'Input 2', 'Contact:1', '']
		]);
	});

	it('reads the ports out of the show for a node that has said nothing', () => {
		const rows = portRows(device('a1', false), fixtures, types);
		expect(rows.map((r) => [r.port, r.name, r.label])).toEqual([
			[0, 'Contact 0', 'Ausgang PIR'],
			[1, 'Contact 1', '']
		]);
	});

	it('names a parameter by its label, and by its kind where there is none', () => {
		expect(parameterName(fixtures[0], { Contact: 0 })).toBe('Ausgang PIR');
		expect(parameterName(fixtures[0], { Contact: 1 })).toBe('Contact 1');
		expect(parameterName(null, { Contact: 0 })).toBe('Contact 0');
		expect(parameterName(fixture('x', 'a1', null, 'x', { 'Contact:0': '  ' }), { Contact: 0 })).toBe('Contact 0');
	});

	it('clears a label that is emptied', () => {
		expect(withLabel(fixtures[0], 'Contact:0', ' ')).toEqual({});
		expect(withLabel(fixtures[0], 'Contact:1', ' Tor ')).toEqual({ 'Contact:0': 'Ausgang PIR', 'Contact:1': 'Tor' });
	});

	it('counts a name in bytes, as the node does', () => {
		expect(tooLong('a'.repeat(31))).toBe(false);
		expect(tooLong('Ä'.repeat(16))).toBe(true);
	});

	it('reads a strip still called after its node as unnamed', () => {
		expect(headLabel(fixtures[1], 'Strip 1')).toBe('');
		expect(headLabel({ ...fixtures[1], name: 'LED Streifen Tor' }, 'Strip 1')).toBe('LED Streifen Tor');
	});

	it('starts an adoption from what the node says, or from the node it replaces', () => {
		expect(namesOnNode(device('b2', true))).toEqual({
			name: 'Node b2',
			heads: { 1: 'LED Streifen Tor' },
			ports: { 1: 'Relay Horn' }
		});
		expect(namesInShow(device('a1', false), fixtures, types)).toEqual({
			name: 'Hof',
			heads: {},
			ports: { 0: 'Ausgang PIR' }
		});
	});
});
