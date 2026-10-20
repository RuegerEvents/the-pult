/**
 * Speaker zones and the stations that map them, as every audio panel needs them.
 *
 * The zones are a collection like any other, shared and reference-counted. What is
 * this module's own is the **names a station gives its sound card** — read out of its
 * preferences, which are a fact about the machine and travel with nobody's show — and
 * the one act a zone needs that is more than a field write: giving the first clip of
 * a show somewhere to go.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import type { AudioDestination, SpeakerZone, Station, Timeline } from '$lib/generated/index.js';
import { readPreferences, writePreferences, type AudioPrefs } from '$lib/preferences.js';
import { collection, showData } from '$lib/stores/show.js';
import type { DeviceNames } from '$lib/zones.js';

export const zones = collection('speaker_zones');
export const stations = collection('stations');
/** What each station plays through and what it calls its channels, by station id. */
export const stationAudio = collection('station_audio');

/** The stations, as a picker names them. */
export const stationNames: Readable<{ id: string; name: string }[]> = derived(stations, ($stations) =>
	$stations.map((s: Station) => ({ id: s.id, name: s.hostname })).sort((a, b) => a.name.localeCompare(b.name))
);

/** Which station plays a timeline: the one it names, or whoever leads. */
export function playerOf(timeline: Timeline | undefined, rows: Station[]): Station | undefined {
	if (!timeline) return undefined;
	if (timeline.node_id) return rows.find((s) => s.id === timeline.node_id);
	return rows.find((s) => s.is_leader);
}

// ── This station's names for its card ─────────────────────────────────────────

/** This station's `[audio]` preferences, once read. */
export const audioPrefs = writable<AudioPrefs | null>(null);

let asked = false;
/** Read them, once per page; a write answers with what was stored, which is kept. */
export function loadAudioPrefs(): void {
	if (asked) return;
	asked = true;
	void readPreferences().then((p) => audioPrefs.set(p?.audio ?? {}));
}

/** The names this station gives a device, by the device's own name. */
export function namesFor(prefs: AudioPrefs | null, device: string | null | undefined): DeviceNames | undefined {
	return device ? prefs?.names?.[device] : undefined;
}

/**
 * Change this station's names for one device. The whole `[audio]` section is written,
 * the way the device pickers write it, because *absent* is a value there.
 */
export async function nameDevice(device: string, names: DeviceNames): Promise<boolean> {
	const now = get(audioPrefs) ?? {};
	const next: AudioPrefs = { ...now, names: { ...(now.names ?? {}), [device]: names } };
	const stored = await writePreferences({ audio: next });
	if (!stored) return false;
	audioPrefs.set(stored.audio);
	return true;
}

// ── Making one ────────────────────────────────────────────────────────────────

/** A new zone, mapped on `station` to `channels` where both are given. */
export async function createZone(
	name: string,
	width: number,
	station?: string | null,
	channels?: number[]
): Promise<string> {
	const id = crypto.randomUUID();
	await showData().speaker_zones.create({
		id,
		name,
		color: null,
		width,
		channels: station && channels && channels.length > 0 ? { [station]: channels } : {},
		nodes: [],
		notes: ''
	} as SpeakerZone);
	return id;
}

/**
 * Where a clip goes when it is first dropped on a timeline: the show's first zone, or —
 * in a show that has none yet — a stereo "Main" made for it, on this station's first
 * two channels. Made inside the caller's gesture, so adding the clip is one Ctrl-Z.
 */
export async function firstDestination(station: string | null): Promise<AudioDestination> {
	const first = get(zones)[0];
	if (first) return { Zone: { zone: first.id } };
	return { Zone: { zone: await createZone('Main', 2, station, [1, 2]) } };
}
