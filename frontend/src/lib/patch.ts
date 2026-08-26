import type {
	Fixture,
	FixtureAddress,
	ParameterBinding,
	ParameterDirection,
	ParameterKind,
	ParameterValue
} from './generated/index.js';

/**
 * The parameter kinds an operator can pick from, as selector labels.
 *
 * `Switch` and `Contact` carry a port number in the schema. The operator never
 * types that number twice: it follows the port the parameter is bound to. `Raw`
 * is left out — a raw channel is addressed by its binding, not chosen by name.
 */
export const PARAMETER_KINDS = [
	'Intensity',
	'ColorRgb',
	'Pan',
	'Tilt',
	'GoboIndex',
	'Switch',
	'Contact',
	'Temperature',
	'Humidity',
	'AirQuality',
	'Text'
] as const;

/** The selector label for a kind: the numbered kinds drop their number. */
export function kindLabel(kind: ParameterKind): string {
	return typeof kind === 'string' ? kind : Object.keys(kind)[0];
}

/** Turn a selector label back into a kind, numbering it after the port it sits on. */
export function kindFromLabel(label: string, portIndex: number): ParameterKind {
	if (label === 'Switch') return { Switch: portIndex };
	if (label === 'Contact') return { Contact: portIndex };
	return label as ParameterKind;
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
	return `Contact:${kind.Contact}`;
}

/** The live_values map key for a parameter. Must match parameter_key in the backend. */
export function parameterKey(kind: ParameterKind): string {
	return parameterKindLabel(kind);
}

/** A sensible zero for a kind, used when a parameter's kind changes. */
export function defaultValueFor(kind: ParameterKind): ParameterValue {
	if (kind === 'ColorRgb') return { type: 'Color', value: { r: 0, g: 0, b: 0 } };
	if (kind === 'GoboIndex') return { type: 'Int', value: 0 };
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

/** Universe and start address, for the fixtures that live on a DMX line. */
export function dmxAddress(address: FixtureAddress): { universe: number; address: number } | null {
	return 'Dmx' in address ? address.Dmx : null;
}

export const isDmx = (fixture: Fixture) => dmxAddress(fixture.address) !== null;

/** How a fixture is addressed, for a table cell: "1 / 10" or a node serial. */
export function addressLabel(fixture: Fixture): string {
	const address = fixture.address;
	if ('Dmx' in address) return `${address.Dmx.universe} / ${address.Dmx.address}`;
	const node = address.OpenHaunt;
	return node.universe === null ? node.serial : `${node.serial} · universe ${node.universe}`;
}

/** The DMX channel a parameter occupies, if it occupies one at all. */
export function bindingChannel(binding: ParameterBinding): number | null {
	return 'Dmx' in binding ? binding.Dmx.channel : null;
}

/** The channel range a fixture occupies, as "1–4". Empty for anything not on DMX. */
export function channelRange(fixture: Fixture, channelCount: number): string {
	const dmx = dmxAddress(fixture.address);
	if (!dmx) return '';
	const last = dmx.address + Math.max(channelCount, 1) - 1;
	return last > dmx.address ? `${dmx.address}–${last}` : String(dmx.address);
}

/**
 * Fixtures whose channels overlap another fixture in the same universe.
 *
 * Only DMX fixtures can clash. Two relays on two nodes are not fighting over
 * anything, and a node has no address to compare.
 */
export function clashingFixtures(fixtures: Fixture[], span: (f: Fixture) => number): Set<string> {
	const clashing = new Set<string>();
	const addressed = fixtures
		.map((f) => ({ fixture: f, dmx: dmxAddress(f.address) }))
		.filter((f): f is { fixture: Fixture; dmx: { universe: number; address: number } } => !!f.dmx);

	for (const a of addressed) {
		for (const b of addressed) {
			if (a.fixture.id === b.fixture.id || a.dmx.universe !== b.dmx.universe) continue;
			const aEnd = a.dmx.address + Math.max(span(a.fixture), 1) - 1;
			const bEnd = b.dmx.address + Math.max(span(b.fixture), 1) - 1;
			if (a.dmx.address <= bEnd && b.dmx.address <= aEnd) {
				clashing.add(a.fixture.id);
				clashing.add(b.fixture.id);
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
