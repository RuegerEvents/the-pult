/**
 * What shape a fade has, and where that shape comes from.
 *
 * Three places can say: the capture, then the cue it is in, then the show's own
 * default for that sort of parameter. The station resolves this for real — it is
 * what builds the `RunningFade` a lamp is driven by — and this is the browser's copy
 * of the same three steps, for the one thing a browser needs them for: telling an
 * operator what a cue is *going* to do before they take it.
 *
 * Mirrored rather than asked for, like `parameterKey` in `patch.ts` beside it and for
 * the same reason: a cue editor that had to round-trip to the station to label a
 * dropdown would be a cue editor that flickers. Held to the schema by
 * `crates/pult-schema/src/types/show.rs`, whose own tests are the ones that matter —
 * this decides what a label says, and that decides what a light does.
 */

import type { Easing, FadeCurves, FadeGroup } from './generated/index.js';

/**
 * The group a parameter key belongs to.
 *
 * Indexed kinds arrive as `Gobo:1` and `Named:Fog output`; the group is decided by
 * the name before the colon, never by which one of them it is.
 */
export function fadeGroup(key: string): FadeGroup {
	switch (key.split(':')[0]) {
		case 'Intensity':
			return 'Intensity';
		case 'Pan':
		case 'Tilt':
			return 'Position';
		case 'ColorRgb':
		case 'ColorWheel':
		case 'ColorTemperature':
			return 'Color';
		case 'Zoom':
		case 'Focus':
		case 'Iris':
		case 'Shutter':
		case 'Strobe':
		case 'Gobo':
		case 'GoboIndex':
		case 'GoboRotation':
		case 'Prism':
		case 'Frost':
			return 'Beam';
		default:
			return 'Other';
	}
}

/** This show's curve for one group. */
export function curveForGroup(curves: FadeCurves, group: FadeGroup): Easing {
	switch (group) {
		case 'Intensity':
			return curves.intensity;
		case 'Position':
			return curves.position;
		case 'Color':
			return curves.color;
		case 'Beam':
			return curves.beam;
		case 'Other':
			return curves.other;
	}
}

/** And for one parameter, which is the question a panel actually asks. */
export function curveForKey(curves: FadeCurves, key: string): Easing {
	return curveForGroup(curves, fadeGroup(key));
}

/**
 * What a capture will actually fade on: its own curve, then its cue's, then the
 * show's. The same three steps, in the same order, that the fade *times* take.
 */
export function resolveEasing(
	curves: FadeCurves,
	capture: Easing | null,
	cue: Easing | null,
	key: string
): Easing {
	return capture ?? cue ?? curveForKey(curves, key);
}

/** What each curve is called where an operator has to pick one. */
export const CURVE_LABELS: Record<Easing, string> = {
	Linear: 'Linear',
	EaseIn: 'Ease in',
	EaseOut: 'Ease out',
	EaseInOut: 'Ease both',
	Step: 'Snap'
};

/** In the order a picker offers them: gentlest first, and the one that does not fade last. */
export const CURVES: Easing[] = ['Linear', 'EaseIn', 'EaseOut', 'EaseInOut', 'Step'];
