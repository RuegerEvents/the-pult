/**
 * Fixture types for tests to build on.
 *
 * A `FixtureType` carries what a real fixture definition turns out to contain —
 * physical data, a geometry tree, modes, where the file came from — and a test that
 * cares about one field of it should not have to spell the other twelve. So this is
 * the empty one, and a test says only what it is about.
 *
 * Not imported by anything the browser ships: it exists for `*.test.ts`.
 */
import type { FixturePhysical, FixtureType, ParameterDefinition } from './generated/index.js';

export const NOTHING_PHYSICAL: FixturePhysical = {
	weight_kg: null,
	power_w: null,
	dimensions_m: null,
	connectors: [],
	leg_height_m: null,
	operating_temperature: null,
	beam_angle_deg: null
};

/** A type with nothing in it. Spread it and name the fields the test is about. */
export function aFixtureType(over: Partial<FixtureType> = {}): FixtureType {
	return {
		id: 'type',
		name: 'Type',
		manufacturer: '',
		short_name: '',
		long_name: '',
		description: '',
		channel_count: 0,
		parameters: [],
		dmx_modes: [],
		physical: NOTHING_PHYSICAL,
		geometry: [],
		source: 'Manual',
		...over
	};
}

/**
 * A parameter with only the two fields that have no sensible default: what it is and
 * where it rests. Everything else — direction, binding, physical range, slots, feature
 * group, emitters — is the empty answer a hand-made type gives.
 */
export function aParameter(
	over: Partial<ParameterDefinition> & Pick<ParameterDefinition, 'kind' | 'default_value'>
): ParameterDefinition {
	return {
		direction: 'Output',
		binding: null,
		physical: null,
		slots: [],
		feature_group: null,
		emitters: [],
		...over
	};
}
