/**
 * A grandMA3 command wing, as the page sees it: MA's names for its controls, how an
 * encoder slot reads, and what a control may be learned as.
 *
 * Small, because the station runs the whole wing (`infra/wing_control/`) — the keypad,
 * the encoders, the faders, the lights. Its arithmetic is in `keys.rs` there and only
 * there; a second copy here drifted the first time it was tried. What is left is what
 * a page needs to *show* the wing and to bind one of its controls.
 *
 * Names come from MA's own control map, which the station reads out of a local
 * grandMA3 install and hands over through `wing.map`.
 */

import type { BindingAction, ParameterKind, ParameterValue, SurfaceControl } from './generated/index.js';

// ── MA's map, as the station hands it over ────────────────────────────────────

/** One control in MA's table. Only the fields this console reads. */
export type WingControl = {
	label: string;
	Code?: string;
	Type?: string;
	Key?: string;
	ExecutorIndex?: string;
	SpecialExecutor?: string;
};

/** One LED and where its channels sit in the 253-byte frame. */
export type WingLed = {
	label: string;
	Code?: string;
	R?: number;
	G?: number;
	B?: number;
	ExecutorIndex?: string;
};

export type WingMap = {
	module: string;
	hardkeys: Record<string, WingControl>;
	faders: Record<string, WingControl>;
	encoders: Record<string, WingControl>;
	leds: WingLed[];
};

/** How many digital inputs the wing has, on its rear panel. */
export const DIGITAL_INPUTS = 7;

/** One position on the encoder bar: a parameter, or one channel of a color. */
export type EncoderSlot = {
	kind: ParameterKind;
	key: string;
	label: string;
	component?: 'r' | 'g' | 'b';
	defaultValue: ParameterValue;
};

/** What an encoder slot reads as, for the bar: a percentage, a step, on or off. */
export function slotReading(value: ParameterValue | null, component?: 'r' | 'g' | 'b'): string {
	if (!value) return '—';
	switch (value.type) {
		case 'Float':
			return `${Math.round(value.value * 100)}%`;
		case 'Int':
			return String(value.value);
		case 'Bool':
			return value.value ? 'On' : 'Off';
		case 'Color':
			return component ? `${Math.round(value.value[component] * 100)}%` : '';
		default:
			return '';
	}
}

// ── Learning ──────────────────────────────────────────────────────────────────

/**
 * What a control can be bound to from something the operator touched, or `null` when
 * the two do not fit: a fader or an encoder moves a parameter and a fader may be a
 * sequence's level; a key presses something, or flashes a parameter it was shown.
 */
export function learnable(control: SurfaceControl, action: BindingAction): BindingAction | null {
	switch (control.kind) {
		case 'Fader':
			return action.kind === 'Parameter' || action.kind === 'Master' ? action : null;
		case 'Encoder':
			return action.kind === 'Parameter' ? action : null;
		case 'Key':
		case 'Digital':
			if (action.kind === 'Parameter') {
				return { kind: 'Flash', fixture_id: action.fixture_id, parameter: action.parameter };
			}
			return action.kind === 'Press' ? action : null;
	}
}

