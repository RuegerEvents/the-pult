/**
 * What an OpenHaunt node's ports and heads are called, read for the Devices panel and
 * the adoption dialog.
 *
 * The station keeps the node and the show the same (`infra/devices/names.rs`); this is
 * only the reading side: which fixture and parameter key a port is in the show, what
 * somebody called it, and what the module calls it when nobody has.
 */
import type { DiscoveredDevice, Fixture, FixtureType, ParameterKind } from '$lib/generated/index.js';
import { displayLabel, parameterKey } from '$lib/patch.js';

/** What a node keeps of a name: 32 bytes with the terminator. */
export const NAME_MAX = 31;

/** Whether a name is longer than a node keeps, counted in bytes: an umlaut is two. */
export function tooLong(name: string): boolean {
	return new TextEncoder().encode(name.trim()).length > NAME_MAX;
}

/** One port of a node, as a row: what it is, and where it is in the show. */
export interface PortRow {
	port: number;
	head: number;
	/** What the module calls it. */
	name: string;
	access: 'readonly' | 'readwrite' | null;
	dataType: string | null;
	unit: string | null;
	/** The fixture it is a parameter of, once the node is adopted. */
	fixture: Fixture | null;
	key: string | null;
	kind: ParameterKind | null;
	/** What somebody called it in the show, or empty. */
	label: string;
}

function serialOf(fixture: Fixture): string | null {
	return 'OpenHaunt' in fixture.address ? fixture.address.OpenHaunt.serial : null;
}

function headOf(fixture: Fixture): number {
	return 'OpenHaunt' in fixture.address ? (fixture.address.OpenHaunt.head ?? 0) : 0;
}

/** Every fixture patched to one node. */
export function fixturesOf(serial: string, fixtures: Fixture[]): Fixture[] {
	return fixtures.filter((f) => serialOf(f) === serial);
}

/** Port number → the fixture and parameter it is in the show. */
function patchedPorts(
	serial: string,
	fixtures: Fixture[],
	types: FixtureType[]
): Map<number, { fixture: Fixture; key: string; kind: ParameterKind; head: number }> {
	const ports = new Map<number, { fixture: Fixture; key: string; kind: ParameterKind; head: number }>();
	for (const fixture of fixturesOf(serial, fixtures)) {
		const type = types.find((t) => t.id === fixture.fixture_type_id);
		for (const parameter of type?.parameters ?? []) {
			const index = parameter.binding?.Port.index;
			if (index === undefined) continue;
			ports.set(index, { fixture, key: parameterKey(parameter.kind), kind: parameter.kind, head: headOf(fixture) });
		}
	}
	return ports;
}

/**
 * The node's ports, from what it said it is, or — for a node that is off and has said
 * nothing since this station started — from the show, so it can be named while it is
 * off.
 */
export function portRows(device: DiscoveredDevice, fixtures: Fixture[], types: FixtureType[]): PortRow[] {
	const patched = patchedPorts(device.serial, fixtures, types);
	const labelOf = (fixture: Fixture | null, key: string | null) =>
		(fixture && key ? fixture.parameter_labels?.[key] : undefined) ?? '';
	if (device.description?.ports?.length) {
		return device.description.ports.map((p) => {
			const at = patched.get(p.port);
			return {
				port: p.port,
				head: p.head ?? 0,
				name: p.name,
				access: p.access,
				dataType: p.dataType,
				unit: p.unit,
				fixture: at?.fixture ?? null,
				key: at?.key ?? null,
				kind: at?.kind ?? null,
				label: labelOf(at?.fixture ?? null, at?.key ?? null)
			};
		});
	}
	return [...patched.entries()]
		.sort(([a], [b]) => a - b)
		.map(([port, at]) => ({
			port,
			head: at.head,
			name: displayLabel(at.kind),
			access: null,
			dataType: null,
			unit: null,
			fixture: at.fixture,
			key: at.key,
			kind: at.kind,
			label: labelOf(at.fixture, at.key)
		}));
}

/** A fixture's labels with one changed: empty removes it. */
export function withLabel(fixture: Fixture, key: string, label: string): { [key in string]?: string } {
	const next = { ...(fixture.parameter_labels ?? {}) };
	const trimmed = label.trim();
	if (trimmed) next[key] = trimmed;
	else delete next[key];
	return next;
}

/** What the module calls a head: "Strip 1". */
export function headName(device: DiscoveredDevice, head: number): string {
	return device.description?.heads?.find((h) => h.head === head)?.name ?? `Head ${head}`;
}

/**
 * What somebody called a head, read off its fixture's name: nothing, where the fixture
 * is still called after its node — `<node> Strip 1`, as adoption names it.
 */
export function headLabel(fixture: Fixture, moduleName: string): string {
	const name = fixture.name.trim();
	return name === moduleName || name.endsWith(` ${moduleName}`) ? '' : name;
}

/** The names an adoption starts from, in node terms. */
export interface AdoptionNames {
	name: string;
	heads: Record<number, string>;
	ports: Record<number, string>;
}

/** What the node itself says: its name and its labels. */
export function namesOnNode(device: DiscoveredDevice): AdoptionNames {
	const heads: Record<number, string> = {};
	const ports: Record<number, string> = {};
	for (const [h, l] of Object.entries(device.labels?.heads ?? {})) if (l) heads[Number(h)] = l;
	for (const [p, l] of Object.entries(device.labels?.ports ?? {})) if (l) ports[Number(p)] = l;
	return { name: device.name, heads, ports };
}

/**
 * What the show calls a node that is gone, in node terms, for the node that takes its
 * place: its own fixture's name, each head's, and each port's label by number.
 */
export function namesInShow(
	old: DiscoveredDevice,
	fixtures: Fixture[],
	types: FixtureType[]
): AdoptionNames {
	const names: AdoptionNames = { name: old.name, heads: {}, ports: {} };
	for (const fixture of fixturesOf(old.serial, fixtures)) {
		const head = headOf(fixture);
		if (head === 0) names.name = fixture.name;
		else {
			const type = types.find((t) => t.id === fixture.fixture_type_id);
			const label = headLabel(fixture, type?.name ?? `Head ${head}`);
			if (label) names.heads[head] = label;
		}
	}
	for (const row of portRows({ ...old, description: null }, fixtures, types)) {
		if (row.label) names.ports[row.port] = row.label;
	}
	return names;
}
