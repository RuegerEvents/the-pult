/**
 * Surfaces: a page of tiles for running a show from a phone.
 *
 * The schema says what a tile points at (`types::surface`); this is everything the
 * page works out from that — what a tile is called when nobody named it, what color
 * a readout is at a value, where a timeline is, and the edits the editor makes to a
 * tile list. Pure, so the editor and the phone view agree by calling one function,
 * and so it can be tested without a station.
 */

import type {
	ColorStop,
	Fixture,
	FixtureType,
	Flow,
	FlowNode,
	ParameterKind,
	ParameterValue,
	Sequence,
	SurfaceTile,
	TileTarget,
	Timeline,
	Variable,
	VariableOp
} from './generated/index.js';
import { currentValue, variableLabel, variableOpLabel } from './flow.js';
import { displayLabel, formatValue, parameterKey } from './patch.js';
import { rgbToHex } from './programmer.js';
import type { PultWsClient } from './ws/client.js';
import { offer } from './stores/learn.js';
import type { DataRoot } from './ws/data.js';
import { uuid } from '$lib/uuid.js';

/** What a surface's tiles can point at: the show, as far as a surface reads it. */
export type SurfaceShow = {
	variables: Variable[];
	timelines: Timeline[];
	sequences: Sequence[];
	flows: Flow[];
	flowNodes: FlowNode[];
	fixtures: Fixture[];
	types: FixtureType[];
};

export type TileKind = TileTarget['kind'];

/** Every kind, in the order the editor offers them. */
export const TILE_KINDS: { kind: TileKind; label: string }[] = [
	{ kind: 'Heading', label: 'Heading' },
	{ kind: 'VariableReadout', label: 'Variable — show it (and press it)' },
	{ kind: 'VariableSetter', label: 'Variable — change it' },
	{ kind: 'FlowButton', label: 'Flow button' },
	{ kind: 'Timeline', label: 'Timeline play / stop' },
	{ kind: 'SequenceGo', label: 'Sequence Go' },
	{ kind: 'SequenceOff', label: 'Sequence off' },
	{ kind: 'Parameter', label: 'Fixture parameter — show it' }
];

/** Whether pressing the tile does something, as against only showing something. */
export const isAction = (target: TileTarget): boolean =>
	(target.kind === 'VariableReadout' && !!target.press) ||
	target.kind === 'FlowButton' ||
	target.kind === 'VariableSetter' ||
	target.kind === 'Timeline' ||
	target.kind === 'SequenceGo' ||
	target.kind === 'SequenceOff';

// ── Names ─────────────────────────────────────────────────────────────────────

/** A flow's button nodes, named after their flow and their own name where they have one. */
export function flowButtons(show: Pick<SurfaceShow, 'flows' | 'flowNodes'>): { id: string; name: string }[] {
	const out: { id: string; name: string }[] = [];
	for (const flow of show.flows) {
		const buttons = show.flowNodes.filter((n) => n.flow_id === flow.id && n.kind === 'Button');
		// A button with a name of its own is called that; one without is told apart
		// from its siblings by number.
		buttons.forEach((node, i) =>
			out.push({
				id: node.id,
				name: node.name
					? `${flow.name} · ${node.name}`
					: buttons.length === 1
						? flow.name
						: `${flow.name} · ${i + 1}`
			})
		);
	}
	return out;
}

/** The parameter a fixture has of a kind, if it has one, with its direction. */
function definitionOf(show: Pick<SurfaceShow, 'fixtures' | 'types'>, fixtureId: string, kind: ParameterKind) {
	const fixture = show.fixtures.find((f) => f.id === fixtureId);
	const type = fixture && show.types.find((t) => t.id === fixture.fixture_type_id);
	const key = parameterKey(kind);
	return type?.parameters.find((p) => parameterKey(p.kind) === key) ?? null;
}

/**
 * What a target is called, for a tile nobody gave a label to — or `null` when the
 * thing it points at is gone, which the tile then says rather than going blank.
 */
export function targetName(target: TileTarget, show: SurfaceShow): string | null {
	switch (target.kind) {
		case 'Heading':
			return '';
		case 'FlowButton':
			return flowButtons(show).find((b) => b.id === target.node_id)?.name ?? null;
		case 'VariableReadout':
			return show.variables.find((v) => v.id === target.variable_id)?.name ?? null;
		case 'VariableSetter': {
			const v = show.variables.find((v) => v.id === target.variable_id);
			return v ? `${v.name}: ${opWords(target.op)}` : null;
		}
		case 'Timeline':
			return show.timelines.find((t) => t.id === target.timeline_id)?.name ?? null;
		case 'SequenceGo': {
			const s = show.sequences.find((s) => s.id === target.sequence_id);
			return s ? `${s.name} Go` : null;
		}
		case 'SequenceOff': {
			const s = show.sequences.find((s) => s.id === target.sequence_id);
			return s ? `${s.name} off` : null;
		}
		case 'Parameter': {
			const fixture = show.fixtures.find((f) => f.id === target.fixture_id);
			return fixture ? `${fixture.name} ${displayLabel(target.parameter)}` : null;
		}
	}
}

/** `+1`, `−1` and `reset` read better on a button than `add 1`. */
export function opWords(op: VariableOp): string {
	if (typeof op === 'object' && 'Add' in op) {
		return op.Add >= 0 ? `+${op.Add}` : `−${-op.Add}`;
	}
	return variableOpLabel(op);
}

/** What a tile says: its own label, the name of its target, or that the target is gone. */
export function tileLabel(tile: SurfaceTile, show: SurfaceShow): string {
	if (tile.label.trim()) return tile.label;
	return targetName(tile.target, show) ?? 'missing';
}

/** Whether the thing a tile points at still exists. A heading always does. */
export const targetExists = (target: TileTarget, show: SurfaceShow): boolean =>
	targetName(target, show) !== null;

// ── Colors ───────────────────────────────────────────────────────────────────

/** What a readout shows when nothing says otherwise: the tile's color for on, gray for off. */
export const OFF_COLOR = '#3a3a3a';
export const ON_COLOR = '#22c55e';

/**
 * The color a readout is at a value: the highest stop at or below it.
 *
 * Below every stop is the lowest stop's color rather than nothing — a count that
 * someone set to −1 by mistake should still read as the color of "empty", not as a
 * tile with no state.
 */
export function stopColor(
	stops: ColorStop[],
	value: number,
	on: string = ON_COLOR,
	off: string = OFF_COLOR
): string {
	if (stops.length === 0) return value !== 0 ? on : off;
	const sorted = [...stops].sort((a, b) => a.at - b.at);
	let chosen = sorted[0];
	for (const stop of sorted) if (value >= stop.at) chosen = stop;
	return chosen.color;
}

/** What a variable readout shows: its value in words and its color. */
export function variableReading(
	target: Extract<TileTarget, { kind: 'VariableReadout' }>,
	show: Pick<SurfaceShow, 'variables'>,
	tileColor: string | null
): { text: string; color: string } | null {
	const variable = show.variables.find((v) => v.id === target.variable_id);
	if (!variable) return null;
	return {
		text: variableLabel(variable),
		color: stopColor(target.stops, currentValue(variable), tileColor ?? ON_COLOR)
	};
}

/**
 * What a parameter readout shows. An input is read from what the device reported,
 * because nothing drives it; an output from the evaluated rig, which is `null` until
 * the page knows the station's clock.
 */
export function parameterReading(
	target: Extract<TileTarget, { kind: 'Parameter' }>,
	show: Pick<SurfaceShow, 'fixtures' | 'types'>,
	evaluated: (fixtureId: string, key: string) => ParameterValue | null,
	tileColor: string | null = null
): { text: string; color: string | null } | null {
	const fixture = show.fixtures.find((f) => f.id === target.fixture_id);
	if (!fixture) return null;
	const key = parameterKey(target.parameter);
	const definition = definitionOf(show, fixture.id, target.parameter);
	const value =
		definition?.direction === 'Input'
			? (fixture.sensed_values[key] ?? null)
			: evaluated(fixture.id, key);
	if (!value) return { text: '–', color: null };
	return { text: formatValue(value), color: valueColor(value, tileColor) };
}

/**
 * A value's own color where it has one: a color is itself, a level is a gray, and a
 * flag on is the tile's color — a relay driving the red lamp reads red when it is on.
 */
export function valueColor(value: ParameterValue, on: string | null = null): string | null {
	switch (value.type) {
		case 'Color':
			return rgbToHex(value.value);
		case 'Bool':
			return value.value ? (on ?? ON_COLOR) : OFF_COLOR;
		case 'Float': {
			const level = Math.round(Math.min(1, Math.max(0, value.value)) * 255);
			const hex = level.toString(16).padStart(2, '0');
			return `#${hex}${hex}${hex}`;
		}
		default:
			return null;
	}
}

/** Black or white, whichever reads on a background. */
export function inkOn(background: string): string {
	const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(background);
	if (!m) return '#fff';
	const [r, g, b] = m.slice(1).map((h) => parseInt(h, 16) / 255);
	return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.55 ? '#111' : '#fff';
}

// ── Timelines ─────────────────────────────────────────────────────────────────

/**
 * Where a timeline is at a console millisecond — the arithmetic `TimelinePanel` does,
 * with the end applied, or `null` when the page cannot place itself on the clock.
 */
export function timelinePosition(timeline: Timeline, now: number | null): number | null {
	if (!timeline.running) return timeline.position_at_anchor_ms;
	if (now === null) return null;
	if (now <= timeline.anchor_ms) return timeline.position_at_anchor_ms;
	const raw = timeline.position_at_anchor_ms + (now - timeline.anchor_ms) * timeline.rate;
	const end = timeline.end;
	if (!end || end.at_ms <= 0) return raw;
	if (end.then === 'Loop') return raw % end.at_ms;
	return Math.min(raw, end.at_ms);
}

// ── Editing a tile list ───────────────────────────────────────────────────────

/** A tile pointing at the first thing of its kind the show has, or `null` if it has none. */
export function newTile(kind: TileKind, show: SurfaceShow, id: string = uuid()): SurfaceTile | null {
	const target = defaultTarget(kind, show);
	if (!target) return null;
	return {
		id,
		label: kind === 'Heading' ? 'Heading' : '',
		color: null,
		size: kind === 'Heading' || kind === 'Timeline' ? 'Full' : 'Half',
		target
	};
}

export function defaultTarget(kind: TileKind, show: SurfaceShow): TileTarget | null {
	switch (kind) {
		case 'Heading':
			return { kind };
		case 'FlowButton': {
			const button = flowButtons(show)[0];
			return button ? { kind, node_id: button.id } : null;
		}
		case 'VariableReadout': {
			const v = show.variables[0];
			return v ? { kind, variable_id: v.id, stops: [], press: null } : null;
		}
		case 'VariableSetter': {
			const v = show.variables[0];
			if (!v) return null;
			return { kind, variable_id: v.id, op: v.kind === 'Flag' ? 'Toggle' : 'Reset' };
		}
		case 'Timeline': {
			const t = show.timelines[0];
			return t ? { kind, timeline_id: t.id, from_start: false } : null;
		}
		case 'SequenceGo':
		case 'SequenceOff': {
			const s = show.sequences[0];
			return s ? { kind, sequence_id: s.id } : null;
		}
		case 'Parameter': {
			for (const fixture of show.fixtures) {
				const type = show.types.find((t) => t.id === fixture.fixture_type_id);
				const first = type?.parameters[0];
				if (first) return { kind, fixture_id: fixture.id, parameter: first.kind };
			}
			return null;
		}
	}
}

/** The list with one tile moved from one place to another. Out of range moves nothing. */
export function moveTile(tiles: SurfaceTile[], from: number, to: number): SurfaceTile[] {
	if (from < 0 || from >= tiles.length || to < 0 || to >= tiles.length || from === to) return tiles;
	const next = [...tiles];
	const [moved] = next.splice(from, 1);
	next.splice(to, 0, moved);
	return next;
}

/** The list with the tile of this id replaced by `patch` applied to it. */
export function patchTile(tiles: SurfaceTile[], id: string, patch: Partial<SurfaceTile>): SurfaceTile[] {
	return tiles.map((t) => (t.id === id ? { ...t, ...patch } : t));
}

export const removeTile = (tiles: SurfaceTile[], id: string): SurfaceTile[] =>
	tiles.filter((t) => t.id !== id);

// ── The link ──────────────────────────────────────────────────────────────────

/**
 * Where a phone opens a surface: this station's own address, so a phone on the
 * same network reaches it, with `?port=` carried over because that is how a second
 * station on one machine is named.
 */
export function surfaceUrl(location: { origin: string; search: string }, id: string): string {
	const port = new URLSearchParams(location.search).get('port');
	const query = port && /^\d+$/.test(port) ? `?port=${port}` : '';
	return `${location.origin}/surface/${encodeURIComponent(id)}${query}`;
}

/** The surface id a path names, or `null` for `/surface` on its own. */
export function surfaceIdOf(pathname: string): string | null {
	const m = /^\/surface\/([^/]+)\/?$/.exec(pathname);
	return m ? decodeURIComponent(m[1]) : null;
}

/** Whether a path is a phone surface rather than the workspace. */
export const isSurfacePath = (pathname: string): boolean => /^\/surface(\/|$)/.test(pathname);

const LOOPBACK = /^(localhost|127\.\d+\.\d+\.\d+|\[::1\])$/;

/**
 * The addresses a phone could reach this station on: the IPv4 addresses of its
 * interfaces that are up and are not loopback or link-local, wired ones first.
 *
 * Read from the station's own inventory (`station_networks`), because the station row's
 * `http_addr` is what the listener *bound* — `0.0.0.0` on a station told nothing —
 * and a phone cannot dial that.
 */
export function phoneHosts(
	interfaces: { name: string; addresses: string[]; up: boolean; loopback: boolean }[]
): { name: string; address: string }[] {
	const out: { name: string; address: string }[] = [];
	for (const nic of interfaces) {
		if (!nic.up || nic.loopback) continue;
		for (const address of nic.addresses) {
			if (!/^\d+\.\d+\.\d+\.\d+$/.test(address) || address.startsWith('169.254.')) continue;
			out.push({ name: nic.name, address });
		}
	}
	// A VPN's tunnel has an address and is never what a phone at the venue is on.
	const rank = (name: string) => (/^(utun|tun|tap|wg|ppp)/.test(name) ? 1 : 0);
	return out.sort((a, b) => rank(a.name) - rank(b.name));
}

/**
 * Where a *phone* should be sent to reach the page this one came from.
 *
 * A desk opened at `localhost` would hand a phone an address that is the phone
 * itself, so a loopback origin is swapped for one of the station's own addresses at
 * the port it serves on — which already names the real port, so `?port=` goes with
 * it. Any other origin is already one the network can reach, and is kept.
 */
export function phoneLocation(
	location: { origin: string; hostname: string; search: string },
	host: string | null,
	port: number | null
): { origin: string; search: string } {
	if (!LOOPBACK.test(location.hostname) || !host || !port) return location;
	return { origin: `http://${host}:${port}`, search: '' };
}

/**
 * Press what a tile points at, the way the panel that owns it already does: a flow
 * button's own `press`, a variable through the unattributed `variable.*` RPCs, a
 * timeline or a sequence with the console millisecond it was taken at.
 *
 * One function for every surface that presses — a phone's tiles and a wing key bound
 * to the same target — so there is no second way of doing any of them. `at` is the
 * console millisecond, or `null` when this page cannot place itself on the station's
 * clock, which refuses rather than anchoring an act wrong.
 */
export async function pressTarget(
	target: TileTarget,
	show: Pick<SurfaceShow, 'timelines'>,
	data: DataRoot,
	client: PultWsClient,
	at: number | null
): Promise<void> {
	const stamp = (): number => {
		if (at === null) throw new Error('This page has not worked out the station clock yet.');
		return at;
	};
	// A key being learned can take this press for its own.
	offer({ kind: 'Press', target });
	switch (target.kind) {
		case 'FlowButton':
			await data.flow_nodes.byId(target.node_id).press();
			return;
		case 'VariableReadout':
		case 'VariableSetter': {
			const op = target.kind === 'VariableSetter' ? target.op : target.press;
			if (!op) return;
			const variableId = target.variable_id;
			if (op === 'Toggle') await client.call('variable.toggle', { variableId });
			else if (op === 'Reset') await client.call('variable.reset', { variableId });
			else if ('Set' in op) await client.call('variable.set', { variableId, value: op.Set });
			else await client.call('variable.add', { variableId, by: op.Add });
			return;
		}
		case 'Timeline': {
			const timeline = show.timelines.find((t) => t.id === target.timeline_id);
			if (!timeline) return;
			const when = stamp();
			const proxy = data.timelines.byId(timeline.id);
			// A toggle either way: running, it stops; stopped, it plays — from 0:00 when
			// the tile says from the beginning. (Always restarting was tried, and took
			// away the phone's only Stop.)
			if (timeline.running) await proxy.stop({ at: when });
			else await proxy.play(target.from_start ? { at: when, fromMs: 0 } : { at: when });
			return;
		}
		case 'SequenceGo':
			await data.sequences.byId(target.sequence_id).goNext({ at: stamp() });
			return;
		case 'SequenceOff':
			await data.sequences.byId(target.sequence_id).off({ at: stamp() });
			return;
	}
}

/**
 * Whether what a tile points at is live right now: a running timeline, a sequence
 * with a cue up, a variable that holds something. What a wing key lights for.
 */
export function targetLive(target: TileTarget, show: SurfaceShow): boolean {
	switch (target.kind) {
		case 'Timeline':
			return show.timelines.some((t) => t.id === target.timeline_id && t.running);
		case 'SequenceGo':
		case 'SequenceOff':
			return show.sequences.some((s) => s.id === target.sequence_id && s.active_cue_index !== null);
		case 'VariableSetter':
		case 'VariableReadout': {
			const variable = show.variables.find((v) => v.id === target.variable_id);
			return variable !== undefined && currentValue(variable) !== 0;
		}
		default:
			return false;
	}
}

/**
 * Everything a key could press, as one list with names: every sequence's Go and Off,
 * every timeline from where it is and from the beginning, every flow button, every variable
 * toggled or reset. What a wing key's binding is chosen from.
 */
export function pressableTargets(show: SurfaceShow): { label: string; target: TileTarget }[] {
	const out: { label: string; target: TileTarget }[] = [];
	for (const s of show.sequences) {
		out.push({ label: `Go · ${s.name}`, target: { kind: 'SequenceGo', sequence_id: s.id } });
		out.push({ label: `Off · ${s.name}`, target: { kind: 'SequenceOff', sequence_id: s.id } });
	}
	for (const t of show.timelines) {
		out.push({ label: `Play/stop · ${t.name}`, target: { kind: 'Timeline', timeline_id: t.id, from_start: false } });
		out.push({ label: `Start from the beginning · ${t.name}`, target: { kind: 'Timeline', timeline_id: t.id, from_start: true } });
	}
	for (const b of flowButtons(show)) {
		out.push({ label: `Press · ${b.name}`, target: { kind: 'FlowButton', node_id: b.id } });
	}
	for (const v of show.variables) {
		if (v.kind === 'Flag') out.push({ label: `Toggle · ${v.name}`, target: { kind: 'VariableSetter', variable_id: v.id, op: 'Toggle' } });
		out.push({ label: `Reset · ${v.name}`, target: { kind: 'VariableSetter', variable_id: v.id, op: 'Reset' } });
	}
	return out;
}
