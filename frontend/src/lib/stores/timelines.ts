/**
 * What the timeline panel and the Inspector both need, kept once.
 *
 * The timeline used to be one panel holding all of this in its own component state,
 * which was fine while everything that edited a timeline was inside it. Now a clip is
 * edited in the Inspector, which may be a panel of its own on the other side of the
 * screen, so the station's LOCAL answers about sound, the detector's proposal and which
 * timeline is open live here and both read them.
 *
 * Nothing here is a second copy of the show. The rows are `collection('timelines')`,
 * the LOCAL values are `localPath`, both shared and reference-counted; what is this
 * module's own is only what a browser decides for itself — which timeline it is
 * looking at, and a proposal nobody has accepted.
 */

import { browser } from '$app/environment';
import { derived, get, readable, writable, type Readable } from 'svelte/store';

import type {
	AudioClip,
	AudioOutput,
	AudioStatus,
	Detected,
	DevicesState,
	NodeAudioState,
	NodeReport,
	TakeStatus,
	Timeline
} from '$lib/generated/index.js';
import { collection, localPath, showData } from '$lib/stores/show.js';
import { addToast } from '$lib/toasts.js';
import { consoleNow } from '$lib/ws/clock.js';
import { uuid } from '$lib/uuid.js';

export const timelines = collection('timelines');

/** This station's answer about each timeline's sound, by timeline id. */
export const audioStatus = localPath<Record<string, AudioStatus>>('audio_status');
/** The device this station mixes into: what a clip's channels are picked from. */
export const audioOutput = localPath<AudioOutput>('audio_output');
/** The OpenHaunt nodes this station can see, and what their loudspeakers can do. */
export const devices = localPath<DevicesState>('devices');
/** What this station has made of each node's clip store, where it drives them. */
const nodeAudioHere = localPath<Record<string, NodeAudioState>>('node_audio');

const nodeReports = collection('node_reports') as Readable<NodeReport[]>;
const stationRows = collection('stations') as Readable<{ id: string }[]>;

/**
 * The report of the station driving the nodes (`NodeReport`), or `null` when none is
 * driving. One whose station has gone is not believed. The station's own rule,
 * `node_faults` in `infra/audio/nodes.rs`, picks the same row.
 */
export const drivingReport: Readable<NodeReport | null> = derived([nodeReports, stationRows], ([$reports, $stations]) => {
	const live = new Set($stations.map((s) => s.id));
	const driving = $reports.filter((r) => r.driving && live.has(r.id)).sort((a, b) => (a.id < b.id ? 1 : -1));
	return driving[0] ?? null;
});

/** What the station driving the nodes has made of each one's clip store. */
export const nodeAudio: Readable<Record<string, NodeAudioState>> = derived(
	[nodeAudioHere, drivingReport],
	([$here, $report]) => {
		if (!$report) return $here ?? {};
		const out: Record<string, NodeAudioState> = {};
		for (const [serial, n] of Object.entries($report.nodes)) if (n?.store) out[serial] = n.store;
		return out;
	}
);
/**
 * What this station is recording, per timeline. LOCAL: a take in progress is in the
 * recording station's memory, so a page on another station sees "armed" and not this.
 */
export const takes = localPath<Record<string, TakeStatus>>('takes');

// ── Which timeline this browser is looking at ─────────────────────────────────

const OPEN_KEY = 'pult.timeline.open';

function remembered(): string | null {
	if (!browser) return null;
	try {
		return localStorage.getItem(OPEN_KEY);
	} catch {
		return null;
	}
}

/**
 * The timeline the panel shows. Per browser, like a zoom: two operators can be looking
 * at two scares. Remembered across a reload, and falls back to the first timeline when
 * the remembered one has gone.
 */
export const openTimelineId = writable<string | null>(remembered());

openTimelineId.subscribe((id) => {
	if (!browser) return;
	try {
		if (id) localStorage.setItem(OPEN_KEY, id);
		else localStorage.removeItem(OPEN_KEY);
	} catch {
		// Storage refused: the choice lasts as long as the page.
	}
});

export const openTimeline: Readable<Timeline | null> = derived(
	[timelines, openTimelineId],
	([$timelines, $id]) => ($timelines.find((t) => t.id === $id) ?? $timelines[0] ?? null) as Timeline | null
);

// ── The detector's proposal ───────────────────────────────────────────────────

/**
 * What *Detect beats* answered, per timeline, until somebody uses it or throws it away.
 * The lanes draw it over the grid and the Inspector offers to keep it, so it lives where
 * both can see it — and only in this browser, because it is a question, not the show.
 */
export const proposals = writable<Record<string, Detected>>({});

// ── The playhead ─────────────────────────────────────────────────────────────

/**
 * The console's clock, once an animation frame, while anything is reading it — the
 * readout in the transport and on a running card. `null` until this browser knows the
 * station's clock, so a readout shows a gap rather than a plausible wrong figure.
 */
export const now: Readable<number | null> = readable<number | null>(null, (set) => {
	if (!browser) return;
	let frame = requestAnimationFrame(function tick() {
		set(consoleNow());
		frame = requestAnimationFrame(tick);
	});
	return () => cancelAnimationFrame(frame);
});

/**
 * Where a timeline's playhead is at `nowMs` — the arithmetic over its SYNCED anchor the
 * station does too. A stopped timeline is where it stopped.
 */
export function positionOf(timeline: Timeline, nowMs: number | null): number | null {
	if (nowMs === null) return null;
	if (!timeline.running || nowMs <= timeline.anchor_ms) return timeline.position_at_anchor_ms;
	return timeline.position_at_anchor_ms + (nowMs - timeline.anchor_ms) * timeline.rate;
}

/**
 * Where a looping timeline is in its lap, and which lap — a position past the end of a
 * loop is never shown, because the station wraps it and so does every readout.
 */
export function lapOf(timeline: Timeline, position: number): { position: number; lap: number } {
	const end = timeline.end;
	if (!end || end.then !== 'Loop' || end.at_ms <= 0) return { position, lap: 0 };
	return { position: position % end.at_ms, lap: Math.floor(position / end.at_ms) };
}

// ── Acts ─────────────────────────────────────────────────────────────────────

/**
 * The moment a transport act is taken, or `null` with a toast. Every station runs the
 * same command from the same arguments and so anchors the same millisecond; a browser
 * that does not know the station's clock yet is not allowed to name one.
 */
export function stamp(): number | null {
	const at = consoleNow();
	if (at === null) addToast('This browser has not worked out the station clock yet');
	return at;
}

const proxy = (id: string) => showData().timelines.byId(id);

export async function play(id: string) {
	const at = stamp();
	if (at !== null) await proxy(id).play({ at });
}

export async function stop(id: string) {
	const at = stamp();
	if (at !== null) await proxy(id).stop({ at });
}

export async function locate(id: string, positionMs: number) {
	const at = stamp();
	if (at !== null) await proxy(id).locate({ positionMs: Math.max(0, Math.round(positionMs)), at });
}

export async function toggle(timeline: Timeline) {
	await (timeline.running ? stop(timeline.id) : playCounted(timeline));
}

// ── Counting in before a recording ────────────────────────────────────────────

const COUNT_IN_KEY = 'pult.timeline.countIn';

/**
 * Seconds counted down before a Play that will record, or 0 for none. This browser's,
 * kept beside the open timeline: how long somebody wants to get their hand to the
 * fader is theirs, not the show's.
 */
export const countInSeconds = writable<number>(
	browser ? Number(localStorage.getItem(COUNT_IN_KEY) ?? 0) || 0 : 0
);
countInSeconds.subscribe((n) => {
	if (browser) {
		try {
			localStorage.setItem(COUNT_IN_KEY, String(n));
		} catch {
			// a browser that will not store it counts in only until the page reloads
		}
	}
});

/**
 * Whole seconds left before a timeline played with its anchor in the future gets
 * there, or `null` when it is not counting in. Read off the anchor rather than kept by
 * a timer here, so every browser and every station shows the same count.
 */
export function countdownOf(timeline: Timeline, nowMs: number | null): number | null {
	if (!timeline.running || nowMs === null || nowMs >= timeline.anchor_ms) return null;
	return Math.ceil((timeline.anchor_ms - nowMs) / 1000);
}

/**
 * Play — with a count-in when the timeline is armed to record and one is set. The
 * count is the station's: Play goes out at once with its anchor that far ahead, so the
 * timeline is running and holding its position, and the sound that has to be ready on
 * the beat — a stream to a node, a deck on this station's card, a clip a node plays
 * from flash — is started during the count and lands on the anchor. Stop cancels it.
 */
export async function playCounted(timeline: Timeline) {
	const seconds = get(countInSeconds);
	if (!timeline.recording || seconds <= 0) return play(timeline.id);
	const at = stamp();
	if (at !== null) await proxy(timeline.id).play({ at: at + seconds * 1000 });
}

/** The row as it is now, which a write is built from. */
export const current = (id: string): Timeline | undefined =>
	(get(timelines) as Timeline[]).find((t) => t.id === id);

/**
 * Rewrite some of a timeline's columns. `clips`, `events`, `markers`, `grid` and
 * `tracks` are one column each, so every edit of one item rewrites its whole list.
 */
export async function writeColumns(id: string, change: Partial<Timeline>) {
	const target = proxy(id);
	for (const [field, value] of Object.entries(change)) {
		await (target as unknown as Record<string, { set(v: unknown): Promise<unknown> }>)[field].set(value);
	}
}

export async function setClip(timelineId: string, clipId: string, change: Partial<AudioClip>) {
	const timeline = current(timelineId);
	if (!timeline) return;
	await proxy(timelineId).clips.set(timeline.clips.map((c) => (c.id === clipId ? { ...c, ...change } : c)));
}

/** A new timeline, open at once. */
export async function createTimeline(name: string): Promise<string> {
	const id = uuid();
	await showData().timelines.create({
		id,
		name,
		clips: [],
		detected: null,
		source: 'Internal',
		grid: [],
		markers: [],
		events: [],
		tracks: [],
		flashes: null,
		pending: null,
		speed_master: null,
		// The leader, which is the rule outputs follow.
		node_id: null,
		end: null,
		running: false,
		anchor_ms: 0,
		position_at_anchor_ms: 0,
		rate: 1,
		recording: null,
		armed_by: null
	});
	// Shown at once. Without this the panel goes on showing the first timeline, and the
	// next thing somebody does — attaching a file, say — happens to a different timeline
	// from the one they just made.
	openTimelineId.set(id);
	return id;
}

// ── How tall a recordings row is ──────────────────────────────────────────────

const ROW_KEY = 'pult.timeline.takeRow';

/**
 * How tall each row of the Recordings lane is drawn, in pixels. This browser's, in
 * `localStorage` beside the zoom: a jaw's movement is read in a row's height, and how
 * much of the screen that is worth is a question about the screen, not about the show.
 */
export const takeRowHeight = writable<number>((browser && Number(localStorage.getItem(ROW_KEY))) || 18);
if (browser) takeRowHeight.subscribe((h) => localStorage.setItem(ROW_KEY, String(Math.round(h))));
