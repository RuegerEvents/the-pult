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

/** The channels a station maps a zone to, or `null` where it has not. */
export function mappingOn(zone: SpeakerZone, station: string): number[] | null {
	const channels = zone.channels[station];
	return channels && channels.length > 0 ? channels : null;
}

/**
 * Does a station have to map this zone? A zone that is only node loudspeakers is not
 * any station's to mix — the same rule the station applies.
 */
export function needsMapping(zone: SpeakerZone): boolean {
	return Object.keys(zone.channels).length > 0 || zone.nodes.length === 0;
}

/** What is wrong with a zone on one station, in the station's words, or `null`. */
export function zoneProblem(zone: SpeakerZone, station: string, stationName: string): string | null {
	if (!needsMapping(zone)) return null;
	const channels = mappingOn(zone, station);
	if (!channels) return `not mapped on ${stationName}`;
	if (channels.length !== zone.width) {
		return `${widthName(zone.width)}, mapped to ${channels.length} channel${channels.length === 1 ? '' : 's'} on ${stationName}`;
	}
	return null;
}

/** The stations, by name, a zone still has to be mapped on. */
export function unmappedOn(zone: SpeakerZone, stations: { id: string; name: string }[]): string[] {
	if (!needsMapping(zone)) return [];
	return stations.filter((s) => !mappingOn(zone, s.id)).map((s) => s.name);
}

/**
 * A click on the matrix: channel `channel` of station `station` into or out of the zone.
 *
 * A mono zone *moves* to the clicked channel, because one channel is all it can be. A
 * wider zone toggles the channel, and a click past its width drops the oldest one it
 * had, so the mapping never silently becomes wider than the zone says it is.
 */
export function toggleChannel(zone: SpeakerZone, station: string, channel: number): SpeakerZone['channels'] {
	const now = zone.channels[station] ?? [];
	let next: number[];
	if (now.includes(channel)) next = now.filter((c) => c !== channel);
	else if (zone.width <= 1) next = [channel];
	else next = [...now, channel].slice(-zone.width);
	const map: SpeakerZone['channels'] = { ...zone.channels };
	if (next.length === 0) delete map[station];
	else map[station] = next;
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
