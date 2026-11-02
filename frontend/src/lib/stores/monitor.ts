/**
 * Listening to a timeline from this page, while programming.
 *
 * The tablet in the maze hears what the timeline is playing, in step with the playhead
 * it draws: the station renders the sound a lead ahead of the show clock and sends it on
 * `/ws/monitor` (see `infra/audio/monitor.rs`), and this plays each chunk at the show
 * millisecond it says it belongs to, by the clock estimate the page already keeps
 * (`ws/clock.ts`). **Never a show output and never a clock**: what this page does
 * reaches nothing but its own speaker, and a tab that sleeps loses its sound and
 * nothing else.
 *
 * **To as many timelines as it likes**: the loop and the trigger together, each its own
 * stream on `/ws/monitor`, placed by its own chunks' clock and mixed on this tab's one
 * speaker at one level. It used to be one timeline, and listening to the trigger
 * silently stopped the loop.
 *
 * Whether this tab listens, and how loud, is kept per browser like `stores/view.ts` —
 * not in the show, which has no business knowing which tablet has its speaker up. A
 * page that was listening when it reloaded is *armed* rather than listening: a browser
 * starts sound only from a tap (Safari is strict about it), so the toggle says so and
 * the next tap carries on — **any** tap or key on the page, not only the Listen button,
 * because a station restarting reloads every page and the sound has to come back
 * without anybody finding the timeline panel first.
 */

import { browser } from '$app/environment';
import { get, writable } from 'svelte/store';

import { decodeChunk, placeChunk } from '../monitor.js';
import { consoleNow } from '../ws/clock.js';
import { backendOrigin, wsUrlFrom } from '../ws/endpoint.js';

const KEY = 'pult.monitor';

export type MonitorSettings = {
	/** The timelines this tab listens to. */
	timelineIds: string[];
	/** 0–1, on this tab's speaker only. */
	level: number;
};

/** `loading` is listening while the station is still fetching or decoding a file the
 *  timeline plays, which is left out of the sound until it is ready. */
export type MonitorState = 'off' | 'armed' | 'listening' | 'loading' | 'no-clock';

function read(): MonitorSettings {
	const fallback: MonitorSettings = { timelineIds: [], level: 0.8 };
	if (!browser) return fallback;
	try {
		const stored = JSON.parse(localStorage.getItem(KEY) ?? '{}');
		// A tab that listened to one timeline before there could be several.
		const ids: string[] = Array.isArray(stored.timelineIds)
			? stored.timelineIds
			: stored.timelineId
				? [stored.timelineId]
				: [];
		return { timelineIds: ids, level: typeof stored.level === 'number' ? stored.level : fallback.level };
	} catch {
		return fallback;
	}
}

export const monitorSettings = writable<MonitorSettings>(read());
monitorSettings.subscribe((s) => {
	if (browser) localStorage.setItem(KEY, JSON.stringify(s));
});

/** What this tab is doing about each timeline. Armed until a tap starts the sound. */
export const monitorStates = writable<Record<string, MonitorState>>(
	Object.fromEntries(read().timelineIds.map((id) => [id, 'armed' as MonitorState]))
);

function setState(timelineId: string, state: MonitorState) {
	monitorStates.update((all) => (all[timelineId] === state ? all : { ...all, [timelineId]: state }));
}

// Armed at load: the first gesture anywhere on the page is the tap that may start it.
if (browser && read().timelineIds.length > 0) {
	const resume = () => {
		window.removeEventListener('pointerdown', resume, true);
		window.removeEventListener('keydown', resume, true);
		const states = get(monitorStates);
		for (const id of get(monitorSettings).timelineIds) if (states[id] === 'armed') start(id);
	};
	window.addEventListener('pointerdown', resume, true);
	window.addEventListener('keydown', resume, true);
}

/** One timeline being listened to: its socket, and where its last chunk ends. */
type Stream = { socket: WebSocket | null; retry: ReturnType<typeof setTimeout> | null; lastEnd: number | null };

let context: AudioContext | null = null;
let gain: GainNode | null = null;
const streams = new Map<string, Stream>();

/** Listen to a timeline, from a tap, or stop when it is already being listened to. */
export function toggleListening(timelineId: string): void {
	const state = get(monitorStates)[timelineId];
	if (state === 'listening' || state === 'loading') {
		stopListening(timelineId);
		return;
	}
	monitorSettings.update((s) => ({
		...s,
		timelineIds: s.timelineIds.includes(timelineId) ? s.timelineIds : [...s.timelineIds, timelineId]
	}));
	start(timelineId);
}

export function stopListening(timelineId: string): void {
	monitorSettings.update((s) => ({ ...s, timelineIds: s.timelineIds.filter((id) => id !== timelineId) }));
	close(timelineId);
	setState(timelineId, 'off');
}

export function setMonitorLevel(level: number): void {
	monitorSettings.update((s) => ({ ...s, level }));
	if (gain) gain.gain.value = level;
}

/** Called from a tap, which is what lets the sound start at all. */
function start(timelineId: string): void {
	close(timelineId);
	context ??= new AudioContext({ latencyHint: 'interactive' });
	void context.resume();
	if (!gain) {
		gain = context.createGain();
		gain.gain.value = get(monitorSettings).level;
		gain.connect(context.destination);
	}
	const stream: Stream = { socket: null, retry: null, lastEnd: null };
	streams.set(timelineId, stream);
	connect(timelineId, stream);
}

function connect(timelineId: string, stream: Stream): void {
	const url = wsUrlFrom(backendOrigin(location), `/ws/monitor?timeline=${encodeURIComponent(timelineId)}`);
	const ws = new WebSocket(url);
	ws.binaryType = 'arraybuffer';
	ws.onopen = () => setState(timelineId, 'listening');
	ws.onmessage = (e) => {
		if (e.data instanceof ArrayBuffer) play(timelineId, stream, e.data);
	};
	ws.onclose = () => {
		if (stream.socket !== ws) return;
		stream.socket = null;
		// Still meant to be listening: the station restarted, or the link dropped.
		if (streams.get(timelineId) === stream && get(monitorSettings).timelineIds.includes(timelineId)) {
			stream.retry = setTimeout(() => connect(timelineId, stream), 2_000);
		}
	};
	stream.socket = ws;
}

function close(timelineId: string): void {
	const stream = streams.get(timelineId);
	if (!stream) return;
	streams.delete(timelineId);
	if (stream.retry) clearTimeout(stream.retry);
	const ws = stream.socket;
	stream.socket = null;
	ws?.close();
	// The speaker goes when nothing is left on it.
	if (streams.size === 0) {
		gain?.disconnect();
		gain = null;
	}
}

function play(timelineId: string, stream: Stream, bytes: ArrayBuffer): void {
	if (!context || !gain) return;
	const chunk = decodeChunk(bytes);
	if (!chunk) return;
	const now = consoleNow();
	// No estimate of the station clock yet: nothing to place a chunk against, so a
	// gap rather than a sound in the wrong place.
	if (now === null) {
		setState(timelineId, 'no-clock');
		return;
	}
	setState(timelineId, chunk.loading ? 'loading' : 'listening');
	const wanted = context.currentTime + (chunk.atMs - now) / 1000;
	const at = placeChunk(wanted, stream.lastEnd, context.currentTime);
	if (at === null) return;
	const buffer = context.createBuffer(chunk.channels.length, chunk.frames, chunk.sampleRate);
	chunk.channels.forEach((samples, c) => buffer.copyToChannel(samples, c));
	const source = context.createBufferSource();
	source.buffer = buffer;
	source.connect(gain);
	source.start(at);
	stream.lastEnd = at + buffer.duration;
}
