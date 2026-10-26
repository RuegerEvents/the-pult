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
 * Whether this tab listens, and how loud, is kept per browser like `stores/view.ts` —
 * not in the show, which has no business knowing which tablet has its speaker up. A
 * page that was listening when it reloaded is *armed* rather than listening: a browser
 * starts sound only from a tap (Safari is strict about it), so the toggle says so and
 * the next tap carries on.
 */

import { browser } from '$app/environment';
import { get, writable } from 'svelte/store';

import { decodeChunk, placeChunk } from '../monitor.js';
import { consoleNow } from '../ws/clock.js';
import { backendOrigin, wsUrlFrom } from '../ws/endpoint.js';

const KEY = 'pult.monitor';

export type MonitorSettings = {
	/** The timeline this tab listens to, or `null` for not listening. */
	timelineId: string | null;
	/** 0–1, on this tab's speaker only. */
	level: number;
};

/** `loading` is listening while the station is still fetching or decoding a file the
 *  timeline plays, which is left out of the sound until it is ready. */
export type MonitorState = 'off' | 'armed' | 'listening' | 'loading' | 'no-clock';

function read(): MonitorSettings {
	const fallback: MonitorSettings = { timelineId: null, level: 0.8 };
	if (!browser) return fallback;
	try {
		return { ...fallback, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') };
	} catch {
		return fallback;
	}
}

export const monitorSettings = writable<MonitorSettings>(read());
monitorSettings.subscribe((s) => {
	if (browser) localStorage.setItem(KEY, JSON.stringify(s));
});

/** What this tab is doing about it. Armed until a tap starts the sound. */
export const monitorState = writable<MonitorState>(read().timelineId ? 'armed' : 'off');

let context: AudioContext | null = null;
let gain: GainNode | null = null;
let socket: WebSocket | null = null;
let lastEnd: number | null = null;
let retry: ReturnType<typeof setTimeout> | null = null;

/** Listen to a timeline, from a tap, or stop when it is the one already listened to. */
export function toggleListening(timelineId: string): void {
	const { timelineId: current } = get(monitorSettings);
	const state = get(monitorState);
	if (current === timelineId && (state === 'listening' || state === 'loading')) {
		stopListening();
		return;
	}
	monitorSettings.update((s) => ({ ...s, timelineId }));
	start(timelineId);
}

export function stopListening(): void {
	monitorSettings.update((s) => ({ ...s, timelineId: null }));
	close();
	monitorState.set('off');
}

export function setMonitorLevel(level: number): void {
	monitorSettings.update((s) => ({ ...s, level }));
	if (gain) gain.gain.value = level;
}

/** Called from the tap itself, which is what lets the sound start at all. */
function start(timelineId: string): void {
	close();
	context ??= new AudioContext({ latencyHint: 'interactive' });
	void context.resume();
	gain = context.createGain();
	gain.gain.value = get(monitorSettings).level;
	gain.connect(context.destination);
	connect(timelineId);
}

function connect(timelineId: string): void {
	const url = wsUrlFrom(backendOrigin(location), `/ws/monitor?timeline=${encodeURIComponent(timelineId)}`);
	const ws = new WebSocket(url);
	ws.binaryType = 'arraybuffer';
	ws.onopen = () => monitorState.set('listening');
	ws.onmessage = (e) => {
		if (e.data instanceof ArrayBuffer) play(e.data);
	};
	ws.onclose = () => {
		if (socket !== ws) return;
		socket = null;
		// Still meant to be listening: the station restarted, or the link dropped.
		if (get(monitorSettings).timelineId === timelineId) {
			retry = setTimeout(() => connect(timelineId), 2_000);
		}
	};
	socket = ws;
}

function close(): void {
	if (retry) clearTimeout(retry);
	retry = null;
	const ws = socket;
	socket = null;
	ws?.close();
	gain?.disconnect();
	gain = null;
	lastEnd = null;
}

function play(bytes: ArrayBuffer): void {
	if (!context || !gain) return;
	const chunk = decodeChunk(bytes);
	if (!chunk) return;
	const now = consoleNow();
	// No estimate of the station clock yet: nothing to place a chunk against, so a
	// gap rather than a sound in the wrong place.
	if (now === null) {
		monitorState.set('no-clock');
		return;
	}
	const state = chunk.loading ? 'loading' : 'listening';
	if (get(monitorState) !== state) monitorState.set(state);
	const wanted = context.currentTime + (chunk.atMs - now) / 1000;
	const at = placeChunk(wanted, lastEnd, context.currentTime);
	if (at === null) return;
	const buffer = context.createBuffer(chunk.channels.length, chunk.frames, chunk.sampleRate);
	chunk.channels.forEach((samples, c) => buffer.copyToChannel(samples, c));
	const source = context.createBufferSource();
	source.buffer = buffer;
	source.connect(gain);
	source.start(at);
	lastEnd = at + buffer.duration;
}
