/**
 * A color parameter's emitters, as the fixture type editor shows and edits them.
 *
 * The emitter list *is* the channel layout of a type with no modes — a byte per
 * emitter, `implicitWidth` in `patch.ts` — so choosing RGBW over RGB here is what
 * gives a hand-made head its fourth channel. The presets come from the station
 * (`fixtureTypes.library`), so the editor, the patch dialog's generic types and a
 * plugin all offer the same mixes.
 */

import type { Emitter, EmitterPreset, ParameterDefinition, Vec3 } from '$lib/generated/index.js';

/** The three a color with no emitters mixes across: what `emitters_of` fills in. */
export const PLAIN_RGB = ['Red', 'Green', 'Blue'];

/** The emitters a color parameter actually drives, filled in where it names none. */
export function namesOf(parameter: ParameterDefinition): string[] {
	return parameter.emitters.length ? parameter.emitters.map((e) => e.name) : PLAIN_RGB;
}

/**
 * The preset these emitters are, if any: the same names in the same order. Order
 * matters because it is the byte order on the wire — RGBW and WRGB are two layouts.
 */
export function presetOf(parameter: ParameterDefinition, presets: EmitterPreset[]): EmitterPreset | null {
	const names = namesOf(parameter);
	return (
		presets.find((p) => p.emitters.length === names.length && p.emitters.every((e, i) => e.name === names[i])) ??
		null
	);
}

const clamp = (n: number) => Math.min(1, Math.max(0, n));

/** A swatch for an emitter's color; gray for one nothing measured. */
export function hexOf(rgb: Vec3 | null): string {
	if (!rgb) return '#808080';
	const byte = (n: number) =>
		Math.round(clamp(n) * 255)
			.toString(16)
			.padStart(2, '0');
	return `#${byte(rgb.x)}${byte(rgb.y)}${byte(rgb.z)}`;
}

/** An `<input type="color">` value back into the 0..1 the mixer reads. */
export function vecOf(hex: string): Vec3 | null {
	const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
	if (!m) return null;
	const n = (s: string) => Math.round((parseInt(s, 16) / 255) * 1000) / 1000;
	return { x: n(m[1]), y: n(m[2]), z: n(m[3]) };
}

/** A fresh emitter for "+ Emitter": a white die, which mixes as the neutral part. */
export function anEmitter(taken: string[]): Emitter {
	let name = 'White';
	for (let i = 2; taken.includes(name); i++) name = `White ${i}`;
	return { name, rgb: { x: 1, y: 1, z: 1 }, subtractive: false };
}

/** A copy of a preset's emitters, so editing a custom list never edits the preset. */
export const copied = (emitters: Emitter[]): Emitter[] => emitters.map((e) => ({ ...e, rgb: e.rgb ? { ...e.rgb } : null }));
