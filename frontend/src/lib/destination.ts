/**
 * Where a clip's sound goes, as the timeline panel reads and writes it.
 *
 * `AudioDestination` is a speaker zone — named for the room, mapped by each station to
 * its own channels (`$lib/zones.ts`) — or one OpenHaunt node's loudspeaker. The panel
 * offers the second only for nodes that said they have one — the `audio` object of
 * their `/info`, which the station keeps on the device row — so nothing here knows what
 * a node is beyond what the node said.
 */
import type {
	AudioDestination,
	DevicesState,
	NodeAudioMode,
	NodeAudioState,
	NodeReport,
	SpeakerZone
} from '$lib/generated/index.js';

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

/** Where one clip stands on one node's flash, for the line under Streamed/Stored. */
export type ClipOnNode =
	| { kind: 'stored' }
	| { kind: 'uploading'; fraction: number; sent: number; total: number }
	| { kind: 'waiting'; behind: boolean }
	| { kind: 'preparing' }
	| { kind: 'refused'; why: string }
	| { kind: 'unknown' };

/**
 * Where a stored clip is on a node, from what the driving station said about that node.
 *
 * By the sha of the region made for this clip on this node (`clips`), never the clip's
 * own asset: a clip is cut and encoded per node, so the asset is not what the node holds.
 * `stored` comes from the node's own listing, so it is the only state that means the
 * clip will play; an upload at 100% is still uploading until the node has said so.
 */
export function clipOnNode(state: NodeAudioState | undefined, clipId: string): ClipOnNode {
	if (!state) return { kind: 'unknown' };
	const sha = state.clips?.[clipId];
	if (!sha) return { kind: 'preparing' };
	if (state.stored.includes(sha)) return { kind: 'stored' };
	if (state.uploading === sha) {
		const total = state.upload_total ?? 0;
		const sent = Math.min(state.upload_sent ?? 0, total);
		return { kind: 'uploading', fraction: total > 0 ? sent / total : 0, sent, total };
	}
	if (state.fault) return { kind: 'refused', why: state.fault };
	return { kind: 'waiting', behind: !!state.uploading };
}

/** That, in words. */
export function clipOnNodeText(on: ClipOnNode): string {
	switch (on.kind) {
		case 'stored':
			return 'on the node';
		case 'uploading':
			return `uploading ${Math.floor(on.fraction * 100)}% (${bytesText(on.sent)} of ${bytesText(on.total)})`;
		case 'waiting':
			return on.behind ? 'waiting: another clip is uploading' : 'waiting to upload';
		case 'preparing':
			return 'being prepared for the node';
		case 'refused':
			return on.why;
		case 'unknown':
			return 'not reported: is the station driving the nodes up?';
	}
}
