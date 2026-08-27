/**
 * The programmer, as maths.
 *
 * Everything here is pure: what an entry's id is, which parameters a selection can
 * be given, what a store would write, and how one value turns into another. The
 * store in `stores/programmer.ts` does the talking; this decides what to say.
 */

import type {
	Cue,
	Fixture,
	FixtureType,
	ParameterCapture,
	ParameterKind,
	ParameterValue,
	ProgrammerValue
} from './generated/index.js';
import { kindLabel, parameterKey } from './patch.js';

// ── Entry ids ─────────────────────────────────────────────────────────────────

const FNV_PRIME = 1099511628211n;
const FNV_OFFSET = 14695981039346656037n;
const MASK = (1n << 64n) - 1n;

function fnv1a(text: string, seed: bigint): bigint {
	let hash = seed;
	for (let i = 0; i < text.length; i++) {
		hash = ((hash ^ BigInt(text.charCodeAt(i))) * FNV_PRIME) & MASK;
	}
	return hash;
}

/**
 * The id of the programmer entry for one parameter of one fixture.
 *
 * Derived rather than minted, and that is the whole reason it exists. Two consoles
 * grabbing the same fader write the same row and converge; minting an id each time
 * would leave two rows for one parameter, both replicated, disagreeing, and taking
 * turns to reach the output.
 *
 * A hash rather than the two ids joined with a slash, because the backend parses
 * every entity key as a UUID. Two 64-bit FNV-1a passes over the same string with
 * different seeds give the 128 bits that needs; it is a naming scheme, not a
 * security one, and a collision would mean two parameters of the same rig hashing
 * together — which is not something an operator can provoke.
 */
export function entryId(fixtureId: string, key: string): string {
	const source = `${fixtureId}/${key}`;
	const hi = fnv1a(source, FNV_OFFSET);
	const lo = fnv1a(source, FNV_OFFSET ^ MASK);
	const hex = (hi.toString(16).padStart(16, '0') + lo.toString(16).padStart(16, '0')).split('');
	// Version 8 (custom) and the RFC 4122 variant, so what comes out is a UUID and
	// says truthfully how it was made.
	hex[12] = '8';
	hex[16] = (parseInt(hex[16], 16) & 0x3 | 0x8).toString(16);
	const s = hex.join('');
	return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20)}`;
}

// ── What a selection can be given ─────────────────────────────────────────────

/** One row of the values panel: a parameter the selection can be set to. */
export type EditableParameter = {
	kind: ParameterKind;
	/** Its `live_values` key, which is also half of an entry id. */
	key: string;
	label: string;
	defaultValue: ParameterValue;
	/** How many of the selected fixtures actually have it. */
	count: number;
	/** True when some of the selection has it and some does not. */
	mixed: boolean;
};

/**
 * The parameters a selection can be programmed on, in the order its types list them.
 *
 * A union rather than an intersection: selecting a wash and a mover and pulling
 * Intensity should move both, and pulling Pan should move the one that can pan. The
 * `mixed` flag is what lets the panel say so instead of quietly doing half the job.
 *
 * Inputs are left out. A contact closure is a parameter a device writes and the show
 * reads, and there is nothing for an operator to set.
 */
export function editableParameters(
	types: FixtureType[],
	fixtures: Fixture[]
): EditableParameter[] {
	const byType = new Map(types.map((type) => [type.id, type]));
	const rows = new Map<string, EditableParameter>();

	for (const fixture of fixtures) {
		const type = byType.get(fixture.fixture_type_id);
		if (!type) continue;
		for (const parameter of type.parameters) {
			if (parameter.direction !== 'Output') continue;
			const key = parameterKey(parameter.kind);
			const existing = rows.get(key);
			if (existing) {
				existing.count += 1;
				continue;
			}
			rows.set(key, {
				kind: parameter.kind,
				key,
				label: kindLabel(parameter.kind),
				defaultValue: parameter.default_value,
				count: 1,
				mixed: false
			});
		}
	}

	return [...rows.values()].map((row) => ({ ...row, mixed: row.count < fixtures.length }));
}

/**
 * What a selection is showing for one parameter, or nothing when it disagrees.
 *
 * A readout of a mixed selection is a lie whichever fixture it picks, so the panel
 * is told that it is mixed and says so.
 */
export function commonValue(
	fixtures: Fixture[],
	key: string
): { value: ParameterValue | null; mixed: boolean } {
	let found: ParameterValue | null = null;
	let seen = false;
	for (const fixture of fixtures) {
		const value = fixture.live_values[key];
		if (value === undefined) continue;
		if (!seen) {
			found = value;
			seen = true;
			continue;
		}
		if (!sameValue(found, value)) return { value: found, mixed: true };
	}
	return { value: found, mixed: false };
}

export function sameValue(a: ParameterValue | null, b: ParameterValue | null): boolean {
	if (a === null || b === null) return a === b;
	if (a.type !== b.type) return false;
	if (a.type === 'Color' && b.type === 'Color') {
		return a.value.r === b.value.r && a.value.g === b.value.g && a.value.b === b.value.b;
	}
	return a.value === (b as { value: unknown }).value;
}

// ── Storing ───────────────────────────────────────────────────────────────────

const captureKey = (capture: { fixture_id: string; parameter_kind: ParameterKind }) =>
	`${capture.fixture_id}/${parameterKey(capture.parameter_kind)}`;

/**
 * What a cue's captures become once the programmer is stored into it.
 *
 * *Merge* keeps everything the cue already said and overwrites only the parameters
 * the programmer holds — the ordinary way a look is built up over several passes.
 * *Replace* makes the cue exactly what is in the programmer, which is what an
 * operator means when a cue has drifted and they want it to say this and nothing else.
 *
 * Times are left at zero, which the playback engine reads as "use the cue's". A
 * capture with its own time is a deliberate thing, and storing should not invent one.
 */
export function storeCaptures(
	existing: ParameterCapture[],
	entries: ProgrammerValue[],
	mode: 'merge' | 'replace',
	include: Set<string>
): ParameterCapture[] {
	const stored: ParameterCapture[] = entries
		.filter((entry) => include.has(entry.id))
		.map((entry) => ({
			fixture_id: entry.fixture_id,
			parameter_kind: entry.parameter_kind,
			value: entry.value,
			fade_in_ms: 0,
			fade_out_ms: 0,
			delay_in_ms: 0
		}));

	if (mode === 'replace') return stored;

	const taken = new Set(stored.map(captureKey));
	return [...existing.filter((capture) => !taken.has(captureKey(capture))), ...stored];
}

/**
 * A cue read back into the programmer, for editing it.
 *
 * The ids are derived the same way a live grab derives them, so loading a cue and
 * then touching one of its parameters patches the entry the cue put there rather
 * than adding a second one beside it.
 */
export function entriesFromCue(cue: Cue): ProgrammerValue[] {
	return cue.captures.map((capture) => ({
		id: entryId(capture.fixture_id, parameterKey(capture.parameter_kind)),
		fixture_id: capture.fixture_id,
		parameter_kind: capture.parameter_kind,
		value: capture.value,
		locked: false
	}));
}

// ── Values ────────────────────────────────────────────────────────────────────

/** A parameter value as a number, for anything that has one. */
export function asFloat(value: ParameterValue | null | undefined): number | null {
	if (!value) return null;
	if (value.type === 'Float' || value.type === 'Int') return value.value;
	if (value.type === 'Bool') return value.value ? 1 : 0;
	return null;
}

/** The same kind of value carrying a different number. */
export function withFloat(like: ParameterValue, n: number): ParameterValue {
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: clamp01(n) };
		case 'Int':
			return { type: 'Int', value: Math.round(n) };
		case 'Bool':
			return { type: 'Bool', value: n > 0.5 };
		default:
			return like;
	}
}

export const rgbToHex = (rgb: { r: number; g: number; b: number }): string =>
	`#${byte(rgb.r)}${byte(rgb.g)}${byte(rgb.b)}`;

/** A `#rrggbb` or `#rgb` colour as 0–1 channels, or null if it is neither. */
export function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
	const text = hex.trim().replace(/^#/, '');
	const full =
		text.length === 3
			? text
					.split('')
					.map((c) => c + c)
					.join('')
			: text;
	if (!/^[0-9a-fA-F]{6}$/.test(full)) return null;
	return {
		r: parseInt(full.slice(0, 2), 16) / 255,
		g: parseInt(full.slice(2, 4), 16) / 255,
		b: parseInt(full.slice(4, 6), 16) / 255
	};
}

/**
 * A value moved by a step, for an arrow key or a wheel.
 *
 * `delta` is a fraction of the parameter's whole range, so one key does the same
 * proportion of travel whether the parameter counts 0–1 or 0–255.
 */
export function nudge(value: ParameterValue, delta: number): ParameterValue {
	switch (value.type) {
		case 'Float':
			return { type: 'Float', value: clamp01(value.value + delta) };
		case 'Int':
			return { type: 'Int', value: Math.round(clamp(value.value + delta * 255, 0, 255)) };
		case 'Bool':
			return delta === 0 ? value : { type: 'Bool', value: delta > 0 };
		case 'Color': {
			const { r, g, b } = value.value;
			return {
				type: 'Color',
				value: { r: clamp01(r + delta), g: clamp01(g + delta), b: clamp01(b + delta) }
			};
		}
		default:
			return value;
	}
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const clamp01 = (v: number) => clamp(v, 0, 1);
const byte = (v: number) =>
	Math.round(clamp01(v) * 255)
		.toString(16)
		.padStart(2, '0');
