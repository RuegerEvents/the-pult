/**
 * Where a clip's sound goes, as the timeline panel reads and writes it.
 *
 * `AudioDestination` is a speaker zone — named for the room, mapped by each station to
 * its own channels (`$lib/zones.ts`) — or one OpenHaunt node's loudspeaker. The panel
 * offers the second only for nodes that said they have one — the `audio` object of
 * their `/info`, which the station keeps on the device row — so nothing here knows what
 * a node is beyond what the node said.
 */
import type { AudioDestination, DevicesState, NodeAudioMode, NodeReport, SpeakerZone } from '$lib/generated/index.js';

/** The zone a destination names, or `null` for a node. */
export function zoneOf(destination: AudioDestination): string | null {
	return 'Zone' in destination ? destination.Zone.zone : null;
}

/** The node a destination names, or `null` for a zone. */
export function nodeOf(destination: AudioDestination): { serial: string; mode: NodeAudioMode } | null {
	return 'Node' in destination ? destination.Node : null;
}

/** One node a clip can go to. */
export interface AudioNode {
	serial: string;
	name: string;
	/** "Line out and Speaker", the way an operator reads a back panel. */
	outputs: string;
	online: boolean;
}

/**
 * The adopted nodes with a loudspeaker, by name.
 *
 * The station driving the nodes is the one that knows: its `node_reports` row, where
 * there is one, says which nodes there are and whether each is on the network, and
 * this station's own view only fills in what the report does not name. A station that
 * is not driving them has nothing but its own mDNS browse, which said "offline" or "not
 * found" about a node the leader was playing through (found at the gig, 2026-10-25).
 */
export function audioNodes(devices: DevicesState | null, report: NodeReport | null = null): AudioNode[] {
	const out = new Map<string, AudioNode>();
	for (const d of Object.values(devices?.discovered ?? {})) {
		if (!d || !d.audio || !d.adopted_fixture_id) continue;
		out.set(d.serial, {
			serial: d.serial,
			name: d.name,
			outputs: d.audio.outputs.map((o) => o.name).join(' and '),
			online: d.online
		});
	}
	for (const [serial, n] of Object.entries(report?.nodes ?? {})) {
		if (!n) continue;
		out.set(serial, {
			serial,
			name: n.name || out.get(serial)?.name || serial,
			outputs: (n.audio?.outputs ?? []).map((o) => o.name).join(' and ') || (out.get(serial)?.outputs ?? ''),
			online: n.online
		});
	}
	return [...out.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** A destination in a few words, for a clip's chip and its lane. */
export function destinationText(destination: AudioDestination, zones: SpeakerZone[], nodes: AudioNode[]): string {
	const zone = zoneOf(destination);
	if (zone !== null) return zones.find((z) => z.id === zone)?.name ?? 'a deleted zone';
	const node = nodeOf(destination);
	if (!node) return '—';
	const name = nodes.find((n) => n.serial === node.serial)?.name ?? node.serial;
	return `${name}${node.mode === 'Stream' ? ' · stream' : ''}`;
}

/** Bytes, the way a store's free space is read. */
export function bytesText(bytes: number): string {
	if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	if (bytes >= 1024) return `${Math.round(bytes / 1024)} kB`;
	return `${bytes} B`;
}
