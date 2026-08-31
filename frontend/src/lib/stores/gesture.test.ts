import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';

import type { DataRoot } from '$lib/ws/data.js';
import { initShowStores } from './show.js';
import { asOneGesture, beginGesture, currentGesture, endGesture, nudging } from './gesture.js';

/** What the socket was last told. The store's only visible effect. */
let told: (string | null)[] = [];

initShowStores({} as unknown as DataRoot, {
	duringGesture: (id: string | null) => told.push(id)
} as never);

beforeEach(() => {
	told = [];
	vi.useFakeTimers();
});

afterEach(() => {
	// The module holds the open gesture, so leaving one behind would hand it to the
	// next test. Close it and let the tail expire before the clock goes back.
	endGesture();
	vi.runAllTimers();
	vi.useRealTimers();
});

describe('marking a run of writes as one act', () => {
	it('tells the socket an id when one begins', () => {
		beginGesture();
		expect(currentGesture()).toBeTruthy();
		expect(told).toEqual([currentGesture()]);
	});

	it('gives each act its own id', () => {
		beginGesture();
		const first = currentGesture();
		beginGesture();
		expect(currentGesture()).not.toBe(first);
	});

	/// The whole reason the close is deferred: the programmer stages a move and
	/// writes it on the next frame, so an act that ended with the pointer would
	/// leave its own last writes outside it.
	it('stays open for a moment after the pointer comes up', () => {
		beginGesture();
		const id = currentGesture();
		endGesture();
		expect(currentGesture()).toBe(id);

		vi.advanceTimersByTime(500);
		expect(currentGesture()).toBeNull();
		expect(told.at(-1)).toBeNull();
	});

	/// Closing late is only safe because beginning replaces the id outright. Without
	/// this, a quick second drag would be folded into the first.
	it('a new act cancels the close of the one before', () => {
		beginGesture();
		endGesture();
		beginGesture();
		const second = currentGesture();

		vi.advanceTimersByTime(500);
		expect(currentGesture(), 'the pending close was not for this one').toBe(second);
	});

	it('ending when nothing is open does nothing', () => {
		endGesture();
		vi.advanceTimersByTime(500);
		expect(told).toEqual([]);
	});
});

describe('a control that repeats without a pointer', () => {
	it('holds one act open across a run of steps', () => {
		nudging();
		const id = currentGesture();
		for (let i = 0; i < 20; i++) {
			vi.advanceTimersByTime(50);
			nudging();
		}
		expect(currentGesture(), 'still the same key being held').toBe(id);
	});

	it('and ends it when the stepping stops', () => {
		nudging();
		vi.advanceTimersByTime(500);
		expect(currentGesture()).toBeNull();
	});
});

describe('the drag action', () => {
	function fakeNode() {
		const handlers = new Map<string, EventListener>();
		return {
			node: {
				addEventListener: (type: string, fn: EventListener) => handlers.set(type, fn),
				removeEventListener: (type: string) => handlers.delete(type)
			} as unknown as Element,
			fire: (type: string) => handlers.get(type)?.(new Event(type)),
			types: () => [...handlers.keys()]
		};
	}

	it('opens on the pointer going down and closes on it coming up', () => {
		const { node, fire } = fakeNode();
		asOneGesture(node);

		fire('pointerdown');
		expect(currentGesture()).toBeTruthy();
		fire('pointerup');
		vi.advanceTimersByTime(500);
		expect(currentGesture()).toBeNull();
	});

	/// A drag cancelled by the phone ringing is still over, and a gesture left open
	/// would swallow whatever the operator did next.
	it('closes on a cancelled pointer too', () => {
		const { node, fire } = fakeNode();
		asOneGesture(node);

		fire('pointerdown');
		fire('pointercancel');
		vi.advanceTimersByTime(500);
		expect(currentGesture()).toBeNull();
	});

	it('lets go of the element when the component does', () => {
		const { node, types, fire } = fakeNode();
		const action = asOneGesture(node);
		expect(types()).toHaveLength(3);
		action.destroy();
		expect(types()).toHaveLength(0);
		fire('pointerdown');
		expect(currentGesture()).toBeNull();
	});
});
