/**
 * An OctoBanger prop, as the import dialog works on it.
 *
 * The station reads the file and plans the import (`infra/interop/octobanger.rs`);
 * what is here is only the part a person does in between — which output each channel
 * goes to — kept out of the component so it can be tested.
 */

import type { Fixture, FixtureType, ParameterKind } from '$lib/generated/index.js';
import { displayLabel, parameterKey } from '$lib/patch.js';

/** What `POST /api/import/obc/read` says about one controller. */
export type Described = {
	path: string;
	controller: {
		name: string;
		comment: string;
		channels: { name: string; comment: string; activeLow: boolean }[];
		endMs: number;
		pinMapping: string | { Other: number };
		trigger: 'Low' | 'Hi' | { Other: number };
		timingOffset: string | { Other: number };
		resetDelayS: number;
		bootDelayS: number;
		media: {
			kind: 'None' | 'Mp3ScareOnly' | 'Mp3AmbientScare' | 'Video' | { Other: number };
			ambient: string | null;
			scare: string | null;
			volume: number;
			delayCs: number;
		};
	};
	channels: {
		number: number;
		used: boolean;
		spans: [number, number][];
		pin: string | null;
		placeholder: string | null;
	}[];
	triggerPin: string | null;
	sounds: {
		role: 'scare' | 'ambient';
		name: string;
		found: boolean;
		durationMs: number | null;
		silent: boolean | null;
		error: string | null;
	}[];
	notes: string[];
};

/** A parameter of a fixture, as something a channel or the trigger can be pointed at. */
export type Port = {
	fixtureId: string;
	key: string;
	kind: ParameterKind;
	/** "Area 1 · Switch 0". */
	label: string;
	fixture: string;
};

/** The id a port is chosen by in a `<select>`. */
export function portId(port: Pick<Port, 'fixtureId' | 'key'>): string {
	return `${port.fixtureId}|${port.key}`;
}

function ports(
	fixtures: Fixture[],
	types: FixtureType[],
	keep: (p: FixtureType['parameters'][number]) => boolean
): Port[] {
	const out: Port[] = [];
	for (const fixture of [...fixtures].sort((a, b) => a.name.localeCompare(b.name))) {
		const type = types.find((t) => t.id === fixture.fixture_type_id);
		for (const parameter of type?.parameters ?? []) {
			if (parameter.per_cell || !keep(parameter)) continue;
			out.push({
				fixtureId: fixture.id,
				key: parameterKey(parameter.kind),
				kind: parameter.kind,
				label: `${fixture.name} · ${displayLabel(parameter.kind)}`,
				fixture: fixture.name
			});
		}
	}
	return out;
}

/**
 * Every output a channel can drive: one that switches, which is what an OctoBanger
 * channel is. The station refuses anything else, so the dialog never offers it.
 */
export function switchOutputs(fixtures: Fixture[], types: FixtureType[]): Port[] {
	return ports(fixtures, types, (p) => p.direction === 'Output' && p.default_value.type === 'Bool');
}

/** Every input a trigger can come from. */
export function triggerInputs(fixtures: Fixture[], types: FixtureType[]): Port[] {
	return ports(fixtures, types, (p) => p.direction === 'Input');
}

/**
 * Give the used channels that have no output yet the outputs of one fixture, in order,
 * skipping any output already given to another channel. Which is how an eight-channel
 * prop goes onto two four-relay nodes: fill from the first, then from the second.
 */
export function fillFrom(
	assigned: Record<number, string | null>,
	used: number[],
	outputs: Port[],
	fixtureId: string
): Record<number, string | null> {
	const taken = new Set(Object.values(assigned).filter((v): v is string => !!v));
	const free = outputs.filter((o) => o.fixtureId === fixtureId && !taken.has(portId(o)));
	const next = { ...assigned };
	for (const channel of used) {
		if (next[channel]) continue;
		const port = free.shift();
		if (!port) break;
		next[channel] = portId(port);
	}
	return next;
}

/** Channels that more than one channel points at: refused by the station, so said here. */
export function clashes(assigned: Record<number, string | null>): Set<number> {
	const seen = new Map<string, number[]>();
	for (const [channel, port] of Object.entries(assigned)) {
		if (!port) continue;
		seen.set(port, [...(seen.get(port) ?? []), Number(channel)]);
	}
	return new Set([...seen.values()].filter((c) => c.length > 1).flat());
}

/**
 * Which edge sets the scare off, from what the file says the input rests at: one that
 * rests low (a PIR) fires going up, one resting on a pull-up (a contact to ground)
 * fires going down. A sensor wired the other way on the node is why it can be changed.
 */
export function defaultEdge(trigger: Described['controller']['trigger']): 'RisingEdge' | 'FallingEdge' {
	return trigger === 'Hi' ? 'FallingEdge' : 'RisingEdge';
}

/** "Low (PIR or + trigger)", the firmware's own words for it. */
export function triggerText(trigger: Described['controller']['trigger']): string {
	if (trigger === 'Low') return 'rests low: a PIR or a + trigger';
	if (trigger === 'Hi') return 'rests high: a contact to ground';
	return `level ${trigger.Other}`;
}
