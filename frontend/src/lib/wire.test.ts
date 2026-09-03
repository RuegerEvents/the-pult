import { describe, expect, it } from 'vitest';

import type { OutputMessage, OutputView } from '$lib/generated/index.js';
import { channelWeight, HISTORY, Wire, wireKey } from './wire.js';

const NODE = '11111111-1111-1111-1111-111111111111';
const OUTPUT = '22222222-2222-2222-2222-222222222222';

const said = (what: string): OutputMessage => ({
	at_ms: 1,
	to: 'abc port 1',
	what,
	detail: '{}'
});

const push = (messages: OutputMessage[], dropped = 0): OutputView => ({
	node_id: NODE,
	output_id: OUTPUT,
	focus: null,
	at_ms: 1000,
	sections: [
		{ title: 'To the nodes', note: null, body: { shape: 'messages', of: { messages, dropped } } }
	]
});

const key = wireKey(NODE, OUTPUT);
const messagesIn = (wire: Wire) => {
	const body = wire.view(key)?.sections[0].body;
	if (body?.shape !== 'messages') throw new Error('not a messages section');
	return body.of;
};

describe('what a browser keeps of what is on the wire', () => {
	it('turns drained batches back into a log', () => {
		const wire = new Wire();
		wire.take(push([said('value')]));
		wire.take(push([said('fades'), said('traces')]));

		expect(messagesIn(wire).messages.map((m) => m.what)).toEqual(['value', 'fades', 'traces']);
	});

	it('keeps the newest and says how much it let go of', () => {
		const wire = new Wire();
		wire.take(push(Array.from({ length: HISTORY + 5 }, (_, n) => said(`n${n}`))));

		const kept = messagesIn(wire);
		expect(kept.messages).toHaveLength(HISTORY);
		expect(kept.messages[0].what).toBe('n5');
		expect(kept.dropped).toBe(5);
	});

	it('adds what the station threw away to what the reader did', () => {
		const wire = new Wire();
		wire.take(push([said('a')], 3));
		wire.take(push([said('b')], 4));

		expect(messagesIn(wire).dropped).toBe(7);
	});

	it('holds the last view, because a settled rig sends nothing at all', () => {
		const wire = new Wire();
		wire.take(push([said('a')]));
		expect(wire.view(key)?.at_ms).toBe(1000);
	});

	it('lets go of an output nobody is watching, history and all', () => {
		const wire = new Wire();
		wire.take(push([said('a')]));
		wire.forget(key);
		expect(wire.view(key)).toBeUndefined();

		wire.take(push([said('b')]));
		expect(messagesIn(wire).messages.map((m) => m.what)).toEqual(['b']);
	});
});

describe('reading a sheet as a shape rather than as numbers', () => {
	it('mutes a channel at zero and lifts the rest with its value', () => {
		expect(channelWeight(0)).toBe(0);
		expect(channelWeight(255)).toBe(1);
		expect(channelWeight(1)).toBeGreaterThan(0.3);
		expect(channelWeight(128)).toBeLessThan(channelWeight(200));
	});
});
