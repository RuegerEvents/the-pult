/**
 * The programmer, as maths.
 *
 * Everything here is pure: what an entry's id is, which parameters a selection can
 * be given, what a store would write, and how one value turns into another. The
 * store in `stores/programmer.ts` does the talking; this decides what to say.
 */

import type {
	Cue,
	Element,
	Fixture,
	FixtureType,
	ParameterCapture,
	ParameterKind,
	ParameterValue,
	Preset,
	PresetValue,
	ProgrammerValue
} from './generated/index.js';
import { fadeGroup } from './fade.js';
import type { FadeGroup, PhysicalRange } from './generated/index.js';
import { displayLabel, inCell, kindBase, modeHas, modeOf, parameterKey } from './patch.js';
import { consoleNow } from './ws/clock.js';
import type { Showing } from './stores/output.js';

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
	/** Its parameter key, which is also half of an entry id. */
	key: string;
	label: string;
	defaultValue: ParameterValue;
	/**
	 * The words a text parameter takes, where its type names them: a strip's Effect
	 * port says which effects the node draws. Empty is free text.
	 */
	choices: string[];
	/**
	 * What its 0..1 spans, where the type says: a strip's Speed is 0 to 10 Hz, and
	 * reads as hertz rather than as a fraction of its travel.
	 */
	physical: PhysicalRange | null;
	/** How many of the selected fixtures actually have it. */
	count: number;
	/**
	 * How many of them are patched in a mode that does not place it.
	 *
	 * A head in its basic mode has a zoom the light can do and the console cannot
	 * send. Showing that as an ordinary fader would be the panel lying about what
	 * moving it does; hiding the row would leave an operator unable to work out why
	 * the light will not zoom. So it is shown, and it says so.
	 */
	unreachable: number;
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
		const mode = modeOf(type, fixture.address);
		// A kind declared twice — a bar's master and its per-cell dimmers — is one row,
		// reachable if either way of setting it is.
		const here = new Set<string>();
		for (const parameter of type.parameters) {
			if (parameter.direction !== 'Output') continue;
			const key = parameterKey(parameter.kind);
			if (here.has(key)) continue;
			here.add(key);
			// A parameter the fixture's chosen mode does not place is one the console
			// cannot send. The row stays — it is still a thing the light can do, and an
			// operator who cannot see it cannot work out why the light will not zoom —
			// but it says so, and a selection where some fixtures can and some cannot
			// says that too.
			// A per-cell parameter is placed cell by cell; the first cell answers for it.
			const reachable = type.parameters.some(
				(p) =>
					p.direction === 'Output' &&
					parameterKey(p.kind) === key &&
					modeHas(mode, p.per_cell && type.cells > 0 ? `${key}@1` : key)
			);
			const existing = rows.get(key);
			if (existing) {
				existing.count += 1;
				existing.unreachable += reachable ? 0 : 1;
				continue;
			}
			rows.set(key, {
				kind: parameter.kind,
				key,
				label: displayLabel(parameter.kind),
				defaultValue: parameter.default_value,
				choices:
					parameter.default_value.type === 'Text' ? parameter.slots.map((slot) => slot.name) : [],
				physical: parameter.physical ?? null,
				count: 1,
				unreachable: reachable ? 0 : 1,
				mixed: false
			});
		}
	}

	return [...rows.values()].map((row) => ({ ...row, mixed: row.count < fixtures.length }));
}

/** One parameter of one fixture, as the programmer holds it: a cell's has its cell. */
export type Target = { fixtureId: string; kind: ParameterKind };

/**
 * Where setting one parameter of a selection lands: one target per element that has
 * it. A cell takes a per-cell parameter as itself; a fixture with cells selected whole
 * takes it on every cell; and a parameter that is the whole fixture's (a bar's master
 * dimmer) is set once per fixture however many of its cells are selected. A fixture
 * whose type is not here takes the kind as itself, the way the station's `__by` does.
 *
 * Where a type declares the kind both ways — a master beside a dimmer per cell, or a
 * one-color mode beside a pixel mode — the patched mode decides (`reaches`): a fixture
 * selected whole takes the whole one where its mode carries it and every cell
 * otherwise; a cell takes its own where its mode carries it and the whole one
 * otherwise, because in a one-color mode the bar's one color is all there is.
 *
 * The station's half is `pult_schema::types::group::targets`, which the command line
 * asks through `selection.targets`; `testdata/cell-targets.json` holds the two together.
 */
export function rowTargets(
	kind: ParameterKind,
	elements: Element[],
	typeOf: (fixtureId: string) => FixtureType | undefined,
	reaches: (fixtureId: string, key: string) => boolean = () => true
): Target[] {
	const base = kindBase(kind);
	const key = parameterKey(base);
	const out: Target[] = [];
	const seen = new Set<string>();
	const add = (fixtureId: string, k: ParameterKind) => {
		const id = `${fixtureId}/${parameterKey(k)}`;
		if (seen.has(id)) return;
		seen.add(id);
		out.push({ fixtureId, kind: k });
	};
	for (const { fixture, cell } of elements) {
		const type = typeOf(fixture);
		if (!type) {
			add(fixture, base);
			continue;
		}
		let whole = false;
		let perCell = false;
		for (const p of type.parameters) {
			if (p.direction !== 'Output' || parameterKey(p.kind) !== key) continue;
			if (p.per_cell && type.cells > 0) perCell = true;
			else whole = true;
		}
		if (cell !== null && perCell && (!whole || reaches(fixture, `${key}@${cell}`))) {
			add(fixture, inCell(base, cell));
		} else if (cell === null && whole && (!perCell || reaches(fixture, key))) {
			add(fixture, base);
		} else if (cell === null && perCell) {
			for (let c = 1; c <= type.cells; c++) add(fixture, inCell(base, c));
		} else if (whole) {
			add(fixture, base);
		}
	}
	return out;
}

/** What a set of targets is showing, or nothing when they disagree. */
export function commonValueOf(
	targets: Target[],
	showing: Showing
): { value: ParameterValue | null; mixed: boolean } {
	let found: ParameterValue | null = null;
	let seen = false;
	for (const target of targets) {
		const value = showing.value(target.fixtureId, parameterKey(target.kind));
		if (value === null) continue;
		if (!seen) {
			found = value;
			seen = true;
			continue;
		}
		if (!sameValue(found, value)) return { value: found, mixed: true };
	}
	return { value: found, mixed: false };
}

/**
 * What a selection is showing for one parameter, or nothing when it disagrees.
 *
 * A readout of a mixed selection is a lie whichever fixture it picks, so the panel
 * is told that it is mixed and says so.
 */
export function commonValue(
	fixtures: Fixture[],
	key: string,
	showing: Showing
): { value: ParameterValue | null; mixed: boolean } {
	let found: ParameterValue | null = null;
	let seen = false;
	for (const fixture of fixtures) {
		const value = showing.value(fixture.id, key);
		if (value === null) continue;
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
 * A preset dropped onto a cue: every value it holds, as a capture that *references*
 * it — reference first, literal beside it, the rule a stored programmer entry follows.
 *
 * A merge: what the cue held for other parameters stays, and a parameter the preset
 * also holds keeps its timing and easing, so dragging a palette onto a cue changes
 * what it says and not how it moves. One write of the whole list, so one Ctrl-Z.
 */
export function presetIntoCue(existing: ParameterCapture[], preset: Preset): ParameterCapture[] {
	const before = new Map(existing.map((capture) => [captureKey(capture), capture]));
	const stored: ParameterCapture[] = preset.values.map((v) => {
		const kept = before.get(captureKey(v));
		return {
			fixture_id: v.fixture_id,
			parameter_kind: v.parameter_kind,
			value: v.value,
			preset: preset.id,
			fade_in_ms: kept?.fade_in_ms ?? 0,
			fade_out_ms: kept?.fade_out_ms ?? 0,
			delay_in_ms: kept?.delay_in_ms ?? 0,
			effect: null,
			easing: kept?.easing ?? null
		};
	});
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
		// Back into the programmer, and anchored now: the operator is holding it
		// again rather than replaying it from where the cue put it.
		effect: capture.effect ? { ...capture.effect, t0: consoleNow() ?? Date.now() } : null,
		// And the reference comes back with it, so editing a cue and updating it does
		// not quietly turn every palette reference in it into a literal.
		preset: capture.preset ?? null,
		locked: false
	}));
}

// ── Presets ───────────────────────────────────────────────────────────────────

/**
 * What recalling a preset writes into the programmer.
 *
 * **Both**, always: the reference *and* the value it resolves to right now. The
 * reference is what makes an edit reach the cue later; the value is what plays when
 * the preset is gone, and what a station that never received the preset shows.
 *
 * `selection` narrows it. With nothing selected a preset applies to every fixture it
 * knows — recalling *warm* means warm — and with a selection it is the intersection,
 * which is what lets one look be applied to half a rig.
 */
export function applyPreset(preset: Preset, selection: string[]): ProgrammerValue[] {
	const wanted = selection.length > 0 ? new Set(selection) : null;
	return preset.values
		.filter((value) => !wanted || wanted.has(value.fixture_id))
		.map((value) => ({
			id: entryId(value.fixture_id, parameterKey(value.parameter_kind)),
			fixture_id: value.fixture_id,
			parameter_kind: value.parameter_kind,
			value: value.value,
			effect: null,
			preset: preset.id,
			locked: false
		}));
}

/**
 * How many of a preset's fixtures the current selection would actually reach, so a
 * button can say "4 of 6" rather than pretending it will do the whole thing.
 */
export function presetReach(preset: Preset, selection: string[]): { of: number; all: number } {
	const fixtures = [...new Set(preset.values.map((v) => v.fixture_id))];
	if (selection.length === 0) return { of: fixtures.length, all: fixtures.length };
	const wanted = new Set(selection);
	return { of: fixtures.filter((id) => wanted.has(id)).length, all: fixtures.length };
}

/**
 * What the programmer becomes, as a preset's values.
 *
 * An effect is left out: a preset is a look, and a shape is an instruction with an
 * anchor in it. Storing one would be storing a chase that every cue referencing the
 * preset would then restart at a moment nobody chose.
 */
export function presetValues(entries: ProgrammerValue[], include: Set<string>): PresetValue[] {
	return entries
		.filter((entry) => include.has(entry.id) && !entry.effect)
		.map((entry) => ({
			fixture_id: entry.fixture_id,
			parameter_kind: entry.parameter_kind,
			value: entry.value
		}));
}

/**
 * A preset's values with more merged into it, keyed the way everything else is.
 *
 * *Update from programmer* is a merge rather than a replace: a preset that also holds
 * a position should not lose it because somebody re-grabbed the color.
 */
export function mergePresetValues(existing: PresetValue[], incoming: PresetValue[]): PresetValue[] {
	const key = (v: PresetValue) => `${v.fixture_id}/${parameterKey(v.parameter_kind)}`;
	const taken = new Set(incoming.map(key));
	return [...existing.filter((v) => !taken.has(key(v))), ...incoming];
}

/**
 * Which fade groups a preset touches, derived from its keys and never stored.
 *
 * A preset that grows a color is a color preset from that moment, and nobody has to
 * reclassify anything — which is the whole reason the pool is flat and the tags are
 * computed. See `crates/pult-schema/src/types/preset.rs`.
 */
export function presetGroups(preset: Preset): FadeGroup[] {
	const found = new Set<FadeGroup>();
	for (const value of preset.values) found.add(fadeGroup(parameterKey(value.parameter_kind)));
	return (['Intensity', 'Position', 'Color', 'Beam', 'Other'] as FadeGroup[]).filter((g) =>
		found.has(g)
	);
}

/** The one-letter tags a pool button wears: I, P, C, B, O. */
export const GROUP_TAGS: Record<FadeGroup, string> = {
	Intensity: 'I',
	Position: 'P',
	Color: 'C',
	Beam: 'B',
	Other: 'O'
};

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

/** A `#rrggbb` or `#rgb` color as 0–1 channels, or null if it is neither. */
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
			const { r, g, b, overrides } = value.value;
			return {
				type: 'Color',
				value: { r: clamp01(r + delta), g: clamp01(g + delta), b: clamp01(b + delta), overrides }
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
