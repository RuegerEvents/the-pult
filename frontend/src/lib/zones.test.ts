import { describe, expect, it } from 'vitest';

import type { SpeakerZone } from '$lib/generated/index.js';
import { channelName, freshZoneName, mappingOn, needsMapping, toggleChannel, unmappedOn, widthName, zoneProblem } from './zones.js';

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
		const z = zone({ channels: { [BOOTH]: [4] } });
		const stations = [
			{ id: BOOTH, name: 'booth' },
			{ id: SPARE, name: 'spare' }
		];
		expect(mappingOn(z, BOOTH)).toEqual([4]);
		expect(mappingOn(z, SPARE)).toBeNull();
		expect(unmappedOn(z, stations)).toEqual(['spare']);
		expect(zoneProblem(z, SPARE, 'spare')).toBe('not mapped on spare');
		expect(zoneProblem(z, BOOTH, 'booth')).toBeNull();
	});

	it('refuses a stereo zone mapped to one channel, as the station does', () => {
		const z = zone({ width: 2, channels: { [BOOTH]: [1] } });
		expect(zoneProblem(z, BOOTH, 'booth')).toBe('stereo, mapped to 1 channel on booth');
	});

	it('does not ask a station to map a zone of node speakers alone', () => {
		const z = zone({ nodes: [{ serial: 'a2', mode: 'Clip' }] });
		expect(needsMapping(z)).toBe(false);
		expect(unmappedOn(z, [{ id: BOOTH, name: 'booth' }])).toEqual([]);
		expect(zoneProblem(z, BOOTH, 'booth')).toBeNull();
	});

	it('moves a mono zone to the channel clicked', () => {
		const z = zone({ channels: { [BOOTH]: [4] } });
		expect(toggleChannel(z, BOOTH, 6)).toEqual({ [BOOTH]: [6] });
		// Clicking the one it has takes it off, and the station is no longer named.
		expect(toggleChannel(z, BOOTH, 4)).toEqual({});
	});

	it('never lets a stereo zone grow past two channels', () => {
		let z = zone({ width: 2 });
		z = { ...z, channels: toggleChannel(z, BOOTH, 1) };
		z = { ...z, channels: toggleChannel(z, BOOTH, 2) };
		expect(z.channels[BOOTH]).toEqual([1, 2]);
		z = { ...z, channels: toggleChannel(z, BOOTH, 5) };
		expect(z.channels[BOOTH]).toEqual([2, 5]);
		// Another station's mapping is left alone.
		z = { ...z, channels: { ...z.channels, [SPARE]: [7, 8] } };
		expect(toggleChannel(z, BOOTH, 2)[SPARE]).toEqual([7, 8]);
	});

	it('finds a name nobody has', () => {
		expect(freshZoneName([zone({ name: 'Zone 2' })])).toBe('Zone 3');
		expect(freshZoneName([])).toBe('Zone 1');
	});
});
