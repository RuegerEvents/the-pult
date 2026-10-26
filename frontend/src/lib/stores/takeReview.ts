/**
 * A take that has stopped, waiting for somebody to say what to keep.
 *
 * The station stores every pass as an asset the moment the timeline stops and lists it
 * on the timeline's PERSISTED `pending` (see `PendingTake` in the schema); nothing is
 * merged until a person answers. **The page that armed the recording opens the review
 * by itself** — it says who it is when it arms (`armedBy`), with an id it keeps for as
 * long as the tab is open — and every other page offers it in the top bar, so a take
 * armed from a tablet that has since gone to sleep can still be answered from the desk.
 *
 * Merge and Discard are station calls (`take.merge`, `take.discard`), each one gesture
 * under this operator's name, so one Ctrl-Z takes a Merge back and puts the take up for
 * review again.
 */

import { browser } from '$app/environment';
import { derived, get, writable, type Readable } from 'svelte/store';

import type { PendingTake, Timeline } from '$lib/generated/index.js';
import { uuid } from '$lib/uuid.js';
import { collection, showClient } from './show.js';

const PAGE_KEY = 'pult.page';

/**
 * This page, by an id it gave itself. Per tab and kept across a reload of it
 * (`sessionStorage`), so arming, reloading and stopping still opens the review here.
 */
export const pageId: string = (() => {
	if (!browser) return '';
	const known = sessionStorage.getItem(PAGE_KEY);
	if (known) return known;
	const id = uuid();
	sessionStorage.setItem(PAGE_KEY, id);
	return id;
})();

/** A timeline with a take waiting, and whether this page armed it. */
export type Waiting = { timeline: Timeline; pending: PendingTake; mine: boolean };

const timelines = collection('timelines') as Readable<Timeline[]>;

export const waiting: Readable<Waiting[]> = derived(timelines, ($timelines) =>
	$timelines
		.filter((t) => !!t.pending && t.pending.passes.length > 0)
		.map((t) => ({ timeline: t, pending: t.pending!, mine: !!pageId && t.pending!.armed_by === pageId }))
);

/** The timeline whose take the review is open on, or `null`. */
export const reviewing = writable<string | null>(null);

/** What a review lists: per parameter, whether it has a take and how many flashes. */
export type ReviewRow = {
	fixtureId: string;
	key: string;
	/** The passes that recorded it as a take. */
	takes: number;
	/** Where its earliest take starts, for reading the list in the order it happened. */
	fromMs: number;
	flashes: number;
};

/** Every parameter a pending take touched, in the order they were first touched. */
export function reviewRows(pending: PendingTake): ReviewRow[] {
	const rows = new Map<string, ReviewRow>();
	const row = (fixtureId: string, key: string, at: number) => {
		const id = `${fixtureId}/${key}`;
		let r = rows.get(id);
		if (!r) {
			r = { fixtureId, key, takes: 0, fromMs: at, flashes: 0 };
			rows.set(id, r);
		}
		r.fromMs = Math.min(r.fromMs, at);
		return r;
	};
	for (const pass of pending.passes) {
		for (const k of pass.keys) row(k.fixture_id, k.key, k.from_ms).takes += 1;
		for (const f of pass.flashes) row(f.fixture_id, f.key, f.at_ms).flashes += 1;
	}
	return [...rows.values()].sort((a, b) => a.fromMs - b.fromMs || a.key.localeCompare(b.key));
}

/** Keep these: `[fixtureId, key]` per parameter, a take's and a flash's chosen apart. */
export async function mergeTake(timelineId: string, takes: [string, string][], flashes: [string, string][]): Promise<void> {
	await showClient().call('take.merge', { timelineId, takes, flashes });
}

export async function discardTake(timelineId: string): Promise<void> {
	await showClient().call('take.discard', { timelineId });
}

/**
 * Open the review on this page when a take this page armed starts waiting. Once per
 * take: closing it with *Later* does not bring it back until another pass arrives.
 * Called once, from the layout.
 */
export function startTakeReview(): void {
	if (!browser) return;
	const seen = new Map<string, number>();
	waiting.subscribe(($waiting) => {
		for (const w of $waiting) {
			const passes = w.pending.passes.length;
			if (!w.mine || (seen.get(w.timeline.id) ?? 0) >= passes) continue;
			seen.set(w.timeline.id, passes);
			if (get(reviewing) === null) reviewing.set(w.timeline.id);
		}
		// Answered somewhere else: the review open here has nothing left to show.
		const open = get(reviewing);
		if (open !== null && !$waiting.some((w) => w.timeline.id === open)) reviewing.set(null);
	});
}
