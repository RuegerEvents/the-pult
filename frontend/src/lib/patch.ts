import type {
	DmxBreak,
	DmxMode,
	Fixture,
	FixtureAddress,
	FixtureType,
	ParameterBinding,
	ParameterDirection,
	ParameterKind,
	ParameterValue
} from './generated/index.js';

/**
 * The parameter kinds an operator can pick from, as selector labels.
 *
 * `Switch`, `Contact` and `Raw` carry a number in the schema. The operator never
 * types that number twice: it follows the channel or port the parameter is bound to.
 *
 * `Raw` is on the list because a fixture type built by hand for a light nobody has
 * written a profile for is mostly raw channels — "channel 5 does something, I do not
 * care what, put it on a fader". Leaving it off meant that light could not be
 * patched at all without editing JSON.
 *
 * `Named` is on it for the mirror-image reason: a device described a port this
 * console has no word for, and an operator building a type by hand should be able to
 * say the same thing. Picking it asks for the name, which is the whole identity of
 * the parameter and what its `live_values` key is built from.
 */
export const PARAMETER_KINDS = [
	'Intensity',
	'ColorRgb',
	'Pan',
	'Tilt',
	'GoboIndex',
	'Zoom',
	'Focus',
	'Iris',
	'Shutter',
	'Strobe',
	'ColorTemperature',
	'Gobo',
	'GoboRotation',
	'ColorWheel',
	'Prism',
	'Frost',
	'Switch',
	'Contact',
	'Temperature',
	'Humidity',
	'AirQuality',
	'Text',
	'Raw',
	'Named'
] as const;

/** The kinds that need something typed in beside the picker. */
export const KIND_NEEDS_NAME = 'Named';

/**
 * The selector label for a kind: the numbered kinds drop their number, and a
 * named one shows the name the device gave it.
 */
export function kindLabel(kind: ParameterKind): string {
	if (typeof kind === 'string') return kind;
	if ('Named' in kind) return kind.Named;
	return Object.keys(kind)[0];
}

/**
 * Turn a selector label back into a kind, numbering it after the port it sits on.
 *
 * `name` is only read for `Named`, and only that kind can be wrong without it: a
 * parameter named nothing has no `live_values` key and no label, so an empty one
 * falls back to something visible rather than to a blank row.
 */
export function kindFromLabel(label: string, portIndex: number, name?: string): ParameterKind {
	if (label === 'Switch') return { Switch: portIndex };
	if (label === 'Contact') return { Contact: portIndex };
	if (label === 'Raw') return { Raw: portIndex };
	if (label === 'Named') return { Named: name?.trim() || 'Unnamed' };
	return label as ParameterKind;
}

/**
 * The label a kind picks in the selector, as opposed to what it shows an operator.
 *
 * `kindLabel` answers "what is this called" and gives a named parameter its own
 * name, which is right everywhere it is read. A `<select>` needs the other answer:
 * which of the fixed options is this one.
 */
export function kindOption(kind: ParameterKind): string {
	if (typeof kind === 'string') return kind;
	return Object.keys(kind)[0];
}

/** Which way a kind usually flows. A sensor reads; a relay is driven. */
export function defaultDirectionFor(kind: ParameterKind): ParameterDirection {
	const label = kindLabel(kind);
	return ['Contact', 'Temperature', 'Humidity', 'AirQuality'].includes(label)
		? 'Input'
		: 'Output';
}

/** A parameter kind as a string, including the tagged variants. */
export function parameterKindLabel(kind: ParameterKind): string {
	if (typeof kind === 'string') return kind;
	if ('Raw' in kind) return `Raw:${kind.Raw}`;
	if ('Switch' in kind) return `Switch:${kind.Switch}`;
	if ('Named' in kind) return `Named:${kind.Named}`;
	if ('Gobo' in kind) return `Gobo:${kind.Gobo}`;
	if ('GoboRotation' in kind) return `GoboRotation:${kind.GoboRotation}`;
	if ('ColorWheel' in kind) return `ColorWheel:${kind.ColorWheel}`;
	if ('Prism' in kind) return `Prism:${kind.Prism}`;
	if ('Frost' in kind) return `Frost:${kind.Frost}`;
	return `Contact:${kind.Contact}`;
}

/** The live_values map key for a parameter. Must match parameter_key in the backend. */
export function parameterKey(kind: ParameterKind): string {
	return parameterKindLabel(kind);
}

/**
 * A sensible zero for a kind, used when a parameter's kind changes.
 *
 * A `Named` kind says nothing about its shape — the data type that decided it
 * lives on the device's description, and the default that came with it is already
 * on the parameter. A level is the least surprising thing to fall back to.
 */
export function defaultValueFor(kind: ParameterKind): ParameterValue {
	if (kind === 'ColorRgb') return { type: 'Color', value: { r: 0, g: 0, b: 0, overrides: {} } };
	if (kind === 'GoboIndex') return { type: 'Int', value: 0 };
	if (
		typeof kind === 'object' &&
		('Gobo' in kind || 'ColorWheel' in kind || 'Prism' in kind || 'Frost' in kind)
	) {
		// A wheel picks a slot by number, not a level.
		return { type: 'Int', value: 0 };
	}
	if (kind === 'Text') return { type: 'Text', value: '' };
	if (typeof kind === 'object' && ('Switch' in kind || 'Contact' in kind)) {
		return { type: 'Bool', value: false };
	}
	return { type: 'Float', value: 0 };
}

/** A parameter value as something short enough to sit in a table cell. */
export function formatValue(value: ParameterValue | undefined): string {
	if (!value) return '–';
	switch (value.type) {
		case 'Float':
			return `${Math.round(value.value * 100)}%`;
		case 'Int':
			return String(value.value);
		case 'Bool':
			return value.value ? 'on' : 'off';
		case 'Text':
			return value.value || '–';
		case 'Color': {
			const hex = (n: number) =>
				Math.round(Math.min(1, Math.max(0, n)) * 255)
					.toString(16)
					.padStart(2, '0');
			return `#${hex(value.value.r)}${hex(value.value.g)}${hex(value.value.b)}`;
		}
	}
}

// ── Addresses ─────────────────────────────────────────────────────────────────

/**
 * The mode every fixture is in until somebody says otherwise.
 *
 * Mirrors `DEFAULT_MODE` in `crates/pult-schema/src/types/fixture.rs`, and is the name
 * the station's implicit mode takes — so a fixture patched here and a fixture read out
 * of an old showfile name the same thing.
 */
export const DEFAULT_MODE = 'Default';

/** Every DMX break a fixture occupies. Empty for anything not on a DMX line. */
export function dmxBreaks(address: FixtureAddress): DmxBreak[] {
	// `?? []` rather than a bare read: these two functions are on the drawing path of
	// every panel, and an address from a peer running an older build would otherwise
	// take a whole view down rather than showing one fixture oddly.
	return 'Dmx' in address ? (address.Dmx.breaks ?? []) : [];
}

/**
 * Universe and start address of a fixture's first break.
 *
 * The first, and named so, because a fixture with a separate dimmer break has two —
 * and every caller that means "where is it patched" means this one. Anything that
 * has to see all of them asks {@link dmxBreaks}.
 */
export function dmxAddress(address: FixtureAddress): { universe: number; address: number } | null {
	return dmxBreaks(address)[0] ?? null;
}

/** Which of its type's modes a fixture is patched in. */
export function fixtureMode(address: FixtureAddress): string | null {
	return 'Dmx' in address ? address.Dmx.mode : null;
}

export const isDmx = (fixture: Fixture) => dmxAddress(fixture.address) !== null;

/** How a fixture is addressed, for a table cell: "1 / 10" or a node serial. */
export function addressLabel(fixture: Fixture): string {
	const address = fixture.address;
	if ('Dmx' in address) {
		// One span per break, so a fixture in two universes reads as being in two.
		return address.Dmx.breaks.map((b) => `${b.universe} / ${b.address}`).join(' + ') || '–';
	}
	const node = address.OpenHaunt;
	return node.universe === null ? node.serial : `${node.serial} · universe ${node.universe}`;
}

/** The DMX channel a parameter occupies, if it occupies one at all. */
export function bindingChannel(binding: ParameterBinding | null): number | null {
	if (!binding) return null;
	return 'Dmx' in binding ? binding.Dmx.channel : null;
}

// ── Modes ─────────────────────────────────────────────────────────────────────

/**
 * The mode a fixture is patched in, resolved against its type.
 *
 * Mirrors `FixtureType::mode` in the schema, and the rules are the same one for the
 * same reason: a show patched against a GDTF file that has since been revised names a
 * mode the new file dropped, and the first mode is a better answer than nothing.
 *
 * A type with no modes of its own has one all the same — the implicit `Default` the
 * station computes from the parameters. The browser does *not* recompute that: it
 * shows the mode name and lets the station lay the channels out, because two
 * derivations of one layout is exactly the drift this codebase spends its effort
 * avoiding. So a type with no `dmx_modes` answers `null` here, and callers read
 * `channel_count` as they always have.
 */
export function modeOf(fixtureType: FixtureType | undefined, address: FixtureAddress): DmxMode | null {
	const modes = fixtureType?.dmx_modes ?? [];
	if (modes.length === 0) return null;
	const name = fixtureMode(address);
	return modes.find((m) => m.name === name) ?? modes[0];
}

/** How many channels a fixture occupies in each of its breaks. */
export function footprint(fixtureType: FixtureType | undefined, address: FixtureAddress): number[] {
	const mode = modeOf(fixtureType, address);
	if (mode) return mode.breaks;
	return [Math.max(fixtureType?.channel_count ?? 0, 0)];
}

/**
 * Whether a mode places a parameter at all.
 *
 * What the programmer greys a control out on: a head in its basic mode has no zoom,
 * and offering one that goes nowhere is worse than saying it is not there. A type with
 * no modes places everything, which is what it always did.
 */
export function modeHas(mode: DmxMode | null, parameterKey: string): boolean {
	if (!mode) return true;
	return mode.channels.some((channel) => channel.parameter_key === parameterKey);
}

/**
 * The parameters a cue captures that a mode would drop.
 *
 * Answered before the mode is changed, so the patch panel can say what will stop
 * working rather than have the operator find out on stage.
 */
export function droppedByMode(mode: DmxMode, captured: string[]): string[] {
	return captured.filter((key) => !modeHas(mode, key));
}

/** The channel range a fixture occupies, as "1–4". Empty for anything not on DMX. */
export function channelRange(fixture: Fixture, channelCount: number): string {
	const breaks = dmxBreaks(fixture.address);
	if (breaks.length === 0) return '';
	const first = breaks[0];
	const last = first.address + Math.max(channelCount, 1) - 1;
	return last > first.address ? `${first.address}–${last}` : String(first.address);
}

/**
 * Fixtures whose channels overlap another fixture in the same universe.
 *
 * Only DMX fixtures can clash. Two relays on two nodes are not fighting over
 * anything, and a node has no address to compare.
 *
 * Bucketed by universe and swept in address order rather than compared pairwise.
 * The patch table recomputes this whenever anything about a fixture changes,
 * including a level moving at 40 Hz during a fade, and comparing every fixture
 * against every other one made a large rig cost real milliseconds per frame.
 *
 * The sweep works because each bucket is sorted by start address: a fixture
 * overlaps something earlier exactly when it starts at or before the furthest end
 * seen so far, and that furthest fixture is necessarily one of the things it
 * overlaps. A fixture that clashes with nothing earlier becomes the new furthest,
 * so it is still there to be named by whatever overlaps it later.
 */
export function clashingFixtures(fixtures: Fixture[], span: (f: Fixture) => number): Set<string> {
	const clashing = new Set<string>();

	const universes = new Map<number, { id: string; start: number; end: number }[]>();
	for (const fixture of fixtures) {
		const dmx = dmxAddress(fixture.address);
		if (!dmx) continue;
		const start = dmx.address;
		const entry = { id: fixture.id, start, end: start + Math.max(span(fixture), 1) - 1 };
		const bucket = universes.get(dmx.universe);
		if (bucket) bucket.push(entry);
		else universes.set(dmx.universe, [entry]);
	}

	for (const bucket of universes.values()) {
		bucket.sort((a, b) => a.start - b.start || a.end - b.end);
		let furthest: { id: string; end: number } | null = null;
		for (const entry of bucket) {
			if (furthest && entry.start <= furthest.end) {
				clashing.add(entry.id);
				clashing.add(furthest.id);
			}
			if (!furthest || entry.end > furthest.end) {
				furthest = { id: entry.id, end: entry.end };
			}
		}
	}
	return clashing;
}

/** The address after the last DMX fixture in a universe, so patching is one click. */
export function nextFreeAddress(
	fixtures: Fixture[],
	universe: number,
	span: (f: Fixture) => number
): number {
	const used = fixtures
		.map((f) => ({ fixture: f, dmx: dmxAddress(f.address) }))
		.filter((f) => f.dmx?.universe === universe);
	if (used.length === 0) return 1;
	return Math.max(...used.map((f) => f.dmx!.address + Math.max(span(f.fixture), 1))) || 1;
}
