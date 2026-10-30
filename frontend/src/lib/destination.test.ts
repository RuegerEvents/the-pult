import { describe, expect, it } from 'vitest';

import type { DevicesState, NodeAudioState, NodeReport, SpeakerZone } from '$lib/generated/index.js';
import {
	audioNodes,
	bytesText,
	clipOnNode,
	clipOnNodeText,
	destinationText,
	nodeOf,
	zoneOf
} from './destination.js';

const ZONES = [{ id: 'z2', name: 'Area 2', color: null, width: 1, channels: {}, nodes: [], notes: '' } as SpeakerZone];

function devices(): DevicesState {
	const base = {
		host: 'h',
		ip: '10.0.0.2',
		port: 80,
		fw: '0.2.0',
		protocol_version: '1',
		module_type: 0x0f01,
		module_name: 'Node V0',
		module_serial: '',
		module_rev: '',
		caps: [],
		description: null,
		effects: null,
		audio_status: null,
		pixels: null,
		head_fixtures: {},
		strip_objects: {},
		faults: [],
		is_mains: false,
		online: true,
		health: null,
		last_seen: '2026-10-20T00:00:00Z',
		net: []
	};
	const audio = {
		rate: 48000,
		voices: 4,
		outputs: [
			{ id: 'line', name: 'Line out', kind: 'line', channels: 2 },
			{ id: 'speaker', name: 'Speaker', kind: 'amp', channels: 1 }
		],
		clips: { formats: ['pcm16'], bytes_total: 100, bytes_free: 100 },
		stream: null
	};
	return {
		discovered: {
			b: { ...base, serial: 'b', name: 'Gate', audio, adopted_fixture_id: 'f1' },
			a: { ...base, serial: 'a', name: 'Area 2', audio, adopted_fixture_id: 'f2' },
			// No loudspeaker, and adopted: not offered.
			c: { ...base, serial: 'c', name: 'Relay', audio: null, adopted_fixture_id: 'f3' },
			// A loudspeaker, but nobody adopted it: not part of this show.
			d: { ...base, serial: 'd', name: 'Stray', audio, adopted_fixture_id: null }
		},
		broker_addr: null,
		active: true
	} as DevicesState;
}

describe('where a clip goes', () => {
	it('reads either kind of destination', () => {
		expect(zoneOf({ Zone: { zone: 'z2' } })).toBe('z2');
		expect(zoneOf({ Node: { serial: 'a', mode: 'Clip' } })).toBeNull();
		expect(nodeOf({ Node: { serial: 'a', mode: 'Stream' } })).toEqual({ serial: 'a', mode: 'Stream' });
		expect(nodeOf({ Zone: { zone: 'z2' } })).toBeNull();
	});

	it('offers only adopted nodes that have a loudspeaker, by name', () => {
		const nodes = audioNodes(devices());
		expect(nodes.map((n) => n.serial)).toEqual(['a', 'b']);
		expect(nodes[0].outputs).toBe('Line out and Speaker');
		expect(audioNodes(null)).toEqual([]);
	});

	it('believes the station driving the nodes over its own browse', () => {
		// This station's mDNS lost node b and never described node e; the leader drives both.
		const here = devices();
		here.discovered.b = { ...here.discovered.b!, online: false };
		const driven = (name: string) => ({ name, online: true, on_broker: true, audio: here.discovered.a!.audio, store: null, summary: null });
		const report = { id: 's1', driving: true, nodes: { b: driven('Gate'), e: driven('Crypt') }, clip_faults: {} } as NodeReport;
		const nodes = audioNodes(here, report);
		expect(nodes.map((n) => [n.serial, n.online])).toEqual([
			['a', true],
			['e', true],
			['b', true]
		]);
	});

	it('says where a clip goes in a few words', () => {
		const nodes = audioNodes(devices());
		expect(destinationText({ Zone: { zone: 'z2' } }, ZONES, nodes)).toBe('Area 2');
		expect(destinationText({ Zone: { zone: 'gone' } }, ZONES, nodes)).toBe('a deleted zone');
		expect(destinationText({ Node: { serial: 'b', mode: 'Clip' } }, ZONES, nodes)).toBe('Gate');
		expect(destinationText({ Node: { serial: 'b', mode: 'Stream' } }, ZONES, nodes)).toBe('Gate · stream');
		// A node the show names and this station has not found still says which.
		expect(destinationText({ Node: { serial: 'zz', mode: 'Clip' } }, ZONES, nodes)).toBe('zz');
	});

	it('reads a store size', () => {
		expect(bytesText(512)).toBe('512 B');
		expect(bytesText(20_480)).toBe('20 kB');
		expect(bytesText(10_264_576)).toBe('9.8 MB');
	});
});

describe('a stored clip on a node', () => {
	const state = (over: Partial<NodeAudioState> = {}): NodeAudioState => ({
		stored: [],
		bytes_total: 10_000_000,
		bytes_free: 10_000_000,
		uploading: null,
		upload_sent: 0,
		upload_total: 0,
		clips: { c1: 'sha1' },
		pending: 1,
		streaming: false,
		fault: null,
		...over
	});

	it('is read by the region made for it, not by the clip', () => {
		expect(clipOnNode(state({ stored: ['sha1'] }), 'c1')).toEqual({ kind: 'stored' });
		expect(clipOnNode(state({ stored: ['c1'] }), 'c1').kind).toBe('waiting');
	});

	it('says how far its upload has got', () => {
		const on = clipOnNode(state({ uploading: 'sha1', upload_sent: 1_572_864, upload_total: 3_145_728 }), 'c1');
		expect(on).toEqual({ kind: 'uploading', fraction: 0.5, sent: 1_572_864, total: 3_145_728 });
		expect(clipOnNodeText(on)).toBe('uploading 50% (1.5 MB of 3.0 MB)');
	});

	it('waits behind another clip, and says when the node refused', () => {
		expect(clipOnNodeText(clipOnNode(state({ uploading: 'other' }), 'c1'))).toBe('waiting: another clip is uploading');
		expect(clipOnNode(state({ fault: 'does not fit on the node (4096 kB)' }), 'c1')).toEqual({
			kind: 'refused',
			why: 'does not fit on the node (4096 kB)'
		});
	});

	it('is being prepared until a region is made, and unknown with no report', () => {
		expect(clipOnNode(state({ clips: {} }), 'c1').kind).toBe('preparing');
		expect(clipOnNode(undefined, 'c1').kind).toBe('unknown');
	});
});
