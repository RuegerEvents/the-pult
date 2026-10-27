/**
 * Speaker zones, as the panels read and edit them.
 *
 * A zone is the show's — "Area 2", "IBC row" — and each station maps it to its own
 * output channels, on the zone row itself (`SpeakerZone::channels`, keyed by station).
 * What *plays* a zone is the station's own resolution (`types::zone::station_channels`
 * in the schema), and its answer reaches the page as `clip_channels` and `clip_faults`
 * on the playing station's status row. What is worked out here is only what a panel
 * shows *before* anything plays: which stations have mapped a zone, and what a click on
 * the matrix makes the mapping.
 *
 * Channel and device names are the station's (`[audio.names]` in its preferences),
 * because which speaker channel 4 is depends on how that machine is wired.
 */
import type { SpeakerZone } from '$lib/generated/index.js';

/** "mono", "stereo", "4 channels" — the schema's own words. */
export function widthName(width: number): string {
	if (width === 1) return 'mono';
	if (width === 2) return 'stereo';
	return `${width} channels`;
}

/** What one station calls its sound card and the card's channels. */
export type DeviceNames = { alias?: string | null; channels?: string[] };

/** A channel by its name on this station, or by its number. */
export function channelName(names: DeviceNames | undefined, channel: number): string {
	const name = names?.channels?.[channel - 1]?.trim();
	return name ? name : `ch ${channel}`;
}

/**
 * Where each of a zone's channels comes out on a station: one list of outputs per zone
 * channel (left, right, …). A station still running the old code writes one number
 * per zone channel, and that is read as a list of one.
 */
export function sidesOf(zone: SpeakerZone, station: string): number[][] {
	const raw = (zone.channels[station] ?? []) as (number[] | number)[];
	return raw.map((side) => (Array.isArray(side) ? side : [side]));
}

/** The sides a station maps a zone to, or `null` where it maps none of them anywhere. */
export function mappingOn(zone: SpeakerZone, station: string): number[][] | null {
	const sides = sidesOf(zone, station);
	return sides.some((outputs) => outputs.length > 0) ? sides : null;
}

/** What a zone's channel is called: L and R for a stereo zone, M for a mono one, a number past that. */
export function sideName(width: number, side: number): string {
	if (width === 1) return 'M';
	if (width === 2) return side === 0 ? 'L' : 'R';
	return String(side + 1);
}

/** Which of a zone's channels an output carries on a station, in its side order. */
export function sidesAt(zone: SpeakerZone, station: string, output: number): number[] {
	return sidesOf(zone, station).flatMap((outputs, side) => (outputs.includes(output) ? [side] : []));
}

/**
 * Where a zone comes out on a station, in words: "ch 4, ch 5" for a mono zone, "L: FOH L ·
 * R: FOH R" for a stereo one, or `null` where it is not mapped there.
 */
export function mappingText(zone: SpeakerZone, station: string, names: DeviceNames | undefined): string | null {
	const sides = mappingOn(zone, station);
	if (!sides) return null;
	const width = Math.max(1, zone.width);
	const said = (outputs: number[] | undefined) =>
		outputs?.length ? outputs.map((c) => channelName(names, c)).join(', ') : 'nowhere';
	if (width === 1) return said(sides[0]);
	return Array.from({ length: width }, (_, i) => `${sideName(width, i)}: ${said(sides[i])}`).join(' · ');
}

/**
 * Does a station have to map this zone? A zone that is only node loudspeakers is not
 * any station's to mix — the same rule the station applies.
 */
export function needsMapping(zone: SpeakerZone): boolean {
	return Object.keys(zone.channels).length > 0 || zone.nodes.length === 0;
}

/** What is wrong with a zone on one station, in the station's words, or `null`. `SpeakerZone::mapped_on`. */
export function zoneProblem(zone: SpeakerZone, station: string, stationName: string): string | null {
	if (!needsMapping(zone)) return null;
	const sides = mappingOn(zone, station);
	if (!sides) return `not mapped on ${stationName}`;
	const missing = Array.from({ length: zone.width }, (_, i) => i).filter((i) => !(sides[i]?.length));
	if (missing.length > 0) {
		const names = missing.map((i) => (zone.width === 2 ? (i === 0 ? 'left' : 'right') : `channel ${i + 1}`));
		return `${widthName(zone.width)}, ${names.join(' and ')} goes nowhere on ${stationName}`;
	}
	return null;
}

/** The stations, by name, a zone still has to be mapped on. */
export function unmappedOn(zone: SpeakerZone, stations: { id: string; name: string }[]): string[] {
	if (!needsMapping(zone)) return [];
	return stations.filter((s) => zoneProblem(zone, s.id, s.name) !== null).map((s) => s.name);
}

/**
 * A click on the matrix: what output `output` of station `station` carries of the zone
 * next.
 *
 * A mono zone's output is on or off. A wider one goes round its channels and back to
 * nothing — off, L, R, off for stereo — so each side is set on its own, and any number
 * of outputs can carry the same side: a bed on all eight Dante channels is eight clicks.
 */
export function toggleChannel(zone: SpeakerZone, station: string, output: number): SpeakerZone['channels'] {
	const width = Math.max(1, zone.width);
	const sides = Array.from({ length: width }, (_, i) => [...(sidesOf(zone, station)[i] ?? [])]);
	const carried = sides.findIndex((outputs) => outputs.includes(output));
	for (const outputs of sides) {
		const at = outputs.indexOf(output);
		if (at >= 0) outputs.splice(at, 1);
	}
	const next = carried < 0 ? 0 : carried + 1;
	if (next < width) sides[next] = [...sides[next], output].sort((a, b) => a - b);
	const map: SpeakerZone['channels'] = { ...zone.channels };
	if (sides.every((outputs) => outputs.length === 0)) delete map[station];
	else map[station] = sides;
	return map;
}

/** A new zone's name that no other zone has. */
export function freshZoneName(zones: SpeakerZone[], base = 'Zone'): string {
	const names = new Set(zones.map((z) => z.name));
	for (let n = zones.length + 1; ; n++) {
		const name = `${base} ${n}`;
		if (!names.has(name)) return name;
	}
}
