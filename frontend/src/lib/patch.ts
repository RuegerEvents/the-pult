import type { ParameterKind, ParameterValue, Fixture } from './generated/index.js';

/** The parameter kinds an operator can pick from. Raw channels are addressed separately. */
export const PARAMETER_KINDS = ['Intensity', 'ColorRgb', 'Pan', 'Tilt', 'GoboIndex'] as const;

/** A parameter kind as a string, including the tagged Raw variant. */
export function parameterKindLabel(kind: ParameterKind): string {
	return typeof kind === 'string' ? kind : `Raw:${kind.Raw}`;
}

/** The live_values map key for a parameter. Must match parameter_key in the backend. */
export function parameterKey(kind: ParameterKind): string {
	return parameterKindLabel(kind);
}

/** A sensible zero for a kind, used when a parameter's kind changes. */
export function defaultValueFor(kind: ParameterKind): ParameterValue {
	if (kind === 'ColorRgb') return { type: 'Color', value: { r: 0, g: 0, b: 0 } };
	if (kind === 'GoboIndex') return { type: 'Int', value: 0 };
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
		case 'Color': {
			const hex = (n: number) =>
				Math.round(Math.min(1, Math.max(0, n)) * 255)
					.toString(16)
					.padStart(2, '0');
			return `#${hex(value.value.r)}${hex(value.value.g)}${hex(value.value.b)}`;
		}
	}
}

/** The channel range a fixture occupies, as "1–4". */
export function channelRange(fixture: Fixture, channelCount: number): string {
	const last = fixture.dmx_address + Math.max(channelCount, 1) - 1;
	return last > fixture.dmx_address ? `${fixture.dmx_address}–${last}` : String(fixture.dmx_address);
}

/** Fixtures whose channels overlap another fixture in the same universe. */
export function clashingFixtures(fixtures: Fixture[], span: (f: Fixture) => number): Set<string> {
	const clashing = new Set<string>();
	for (const a of fixtures) {
		for (const b of fixtures) {
			if (a.id === b.id || a.universe !== b.universe) continue;
			const aEnd = a.dmx_address + Math.max(span(a), 1) - 1;
			const bEnd = b.dmx_address + Math.max(span(b), 1) - 1;
			if (a.dmx_address <= bEnd && b.dmx_address <= aEnd) {
				clashing.add(a.id);
				clashing.add(b.id);
			}
		}
	}
	return clashing;
}
