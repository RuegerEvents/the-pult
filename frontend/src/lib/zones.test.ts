import { describe, expect, it } from 'vitest';

import type { SpeakerZone } from '$lib/generated/index.js';
import {
	channelName,
	freshZoneName,
	mappingOn,
	needsMapping,
	nodeMode,
	nodeSidesOf,
	sidesAt,
	sidesOf,
	toggleChannel,
	toggleNodeChannel,
	unmappedOn,
	widthName,
	zoneProblem
} from './zones.js';

const BOOTH = 'booth-id';
const SPARE = 'spare-id';

function zone(over: Partial<SpeakerZone> = {}): SpeakerZone {
	return { id: 'z', name: 'Area 2', color: null, width: 1, channels: {}, nodes: [], notes: '', ...over } as SpeakerZone;
}

describe('speaker zones', () => {
	it('names a width the way the schema does', () => {
		expect(widthName(1)).toBe('mono');
		expect(widthName(2)).toBe('stereo');
		expect(widthName(6)).toBe('6 channels');
	});

	it('reads a channel by the name its station gave it, or by its number', () => {
		const names = { alias: 'Garden Dante', channels: ['FOH L', 'FOH R', '', 'Area 2 top'] };
		expect(channelName(names, 1)).toBe('FOH L');
		expect(channelName(names, 3)).toBe('ch 3');
		expect(channelName(names, 4)).toBe('Area 2 top');
		expect(channelName(undefined, 9)).toBe('ch 9');
	});

	it('says which stations have not mapped a zone', () => {
		const z = zone({ channels: { [BOOTH]: [[4]] } });
		const stations = [
			{ id: BOOTH, name: 'booth' },
			{ id: SPARE, name: 'spare' }
		];
		expect(mappingOn(z, BOOTH)).toEqual([[4]]);
		expect(mappingOn(z, SPARE)).toBeNull();
		expect(unmappedOn(z, stations)).toEqual(['spare']);
		expect(zoneProblem(z, SPARE, 'spare')).toBe('not mapped on spare');
		expect(zoneProblem(z, BOOTH, 'booth')).toBeNull();
	});

	it('says which side of a stereo zone goes nowhere, as the station does', () => {
		const z = zone({ width: 2, channels: { [BOOTH]: [[1], []] } });
		expect(zoneProblem(z, BOOTH, 'booth')).toBe('stereo, right goes nowhere on booth');
	});

	it('reads a map written by a station still on one output per channel', () => {
		const z = zone({ width: 2, channels: { [BOOTH]: [1, 2] as unknown as number[][] } });
		expect(sidesOf(z, BOOTH)).toEqual([[1], [2]]);
		expect(zoneProblem(z, BOOTH, 'booth')).toBeNull();
	});

	it('does not ask a station to map a zone of node speakers alone', () => {
		const z = zone({ nodes: [{ serial: 'a2', mode: 'Clip', channels: [] }] });
		expect(needsMapping(z)).toBe(false);
		expect(unmappedOn(z, [{ id: BOOTH, name: 'booth' }])).toEqual([]);
		expect(zoneProblem(z, BOOTH, 'booth')).toBeNull();
	});

	it('puts a mono zone on as many outputs as are clicked, and takes one off again', () => {
		let z = zone({ channels: { [BOOTH]: [[4]] } });
		z = { ...z, channels: toggleChannel(z, BOOTH, 6) };
		expect(z.channels[BOOTH]).toEqual([[4, 6]]);
		z = { ...z, channels: toggleChannel(z, BOOTH, 4) };
		expect(z.channels[BOOTH]).toEqual([[6]]);
		// The last one off, and the station is no longer named.
		expect(toggleChannel(z, BOOTH, 6)).toEqual({});
	});

	it('goes round off, left, right on a stereo zone, each side on any number of outputs', () => {
		let z = zone({ width: 2 });
		for (const output of [1, 3, 5, 7]) z = { ...z, channels: toggleChannel(z, BOOTH, output) };
		for (const output of [2, 4, 6, 8]) {
			z = { ...z, channels: toggleChannel(z, BOOTH, output) };
			z = { ...z, channels: toggleChannel(z, BOOTH, output) };
		}
		expect(z.channels[BOOTH]).toEqual([[1, 3, 5, 7], [2, 4, 6, 8]]);
		expect(sidesAt(z, BOOTH, 4)).toEqual([1]);
		// A third click on a right takes it off altogether.
		expect(toggleChannel(z, BOOTH, 8)[BOOTH]).toEqual([[1, 3, 5, 7], [2, 4, 6]]);
		// Another station's mapping is left alone.
		z = { ...z, channels: { ...z.channels, [SPARE]: [[7], [8]] } };
		expect(toggleChannel(z, BOOTH, 2)[SPARE]).toEqual([[7], [8]]);
	});

	it('finds a name nobody has', () => {
		expect(freshZoneName([zone({ name: 'Zone 2' })])).toBe('Zone 3');
		expect(freshZoneName([])).toBe('Zone 1');
	});
});

describe('a node loudspeaker in the matrix', () => {
	const zone = (over: Partial<SpeakerZone>): SpeakerZone => ({
		id: 'z',
		name: 'Area 2',
		color: null,
		width: 1,
		channels: {},
		nodes: [],
		notes: '',
		folder: null,
		...over
	});

	it('reads a member from before the matrix as the whole node', () => {
		expect(nodeSidesOf(zone({ nodes: [{ serial: 'n', mode: 'Stream', channels: [] }] }), 'n')).toEqual([[1, 2]]);
		expect(nodeSidesOf(zone({ width: 2, nodes: [{ serial: 'n', mode: 'Stream', channels: [] }] }), 'n')).toEqual([[1], [2]]);
	});

	it('joins the zone on the first click, streamed, and leaves it when it carries nothing', () => {
		let z = zone({});
		z = { ...z, nodes: toggleNodeChannel(z, 'n', 1, 'Stream') };
		expect(z.nodes).toEqual([{ serial: 'n', mode: 'Stream', channels: [[1]] }]);
		z = { ...z, nodes: toggleNodeChannel(z, 'n', 1, 'Stream') };
		expect(z.nodes).toEqual([]);
	});

	it('goes round a stereo zone side by side, and keeps how the node is reached', () => {
		let z = zone({ width: 2, nodes: [{ serial: 'n', mode: 'Clip', channels: [[1], [2]] }] });
		z = { ...z, nodes: toggleNodeChannel(z, 'n', 1, 'Stream') };
		expect(z.nodes).toEqual([{ serial: 'n', mode: 'Clip', channels: [[], [1, 2]] }]);
		expect(nodeMode([z], 'n')).toBe('Clip');
	});
});
