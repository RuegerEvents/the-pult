/**
 * A grandMA3 command wing, as arithmetic: what a key types, what an encoder turns,
 * what a fader position is worth, and which keys belong to which fader.
 *
 * Pure, and the reason it is pure is that every rule here was decided once and has to
 * stay decided: `stores/wing.ts` is the part with hands, which reads the show, writes
 * the programmer and lights the desk. The wing itself is on a station; the station
 * relays what is pressed to the page driving it (`infra/wing.rs`), and this page does
 * all the programming, because the selection, the command line and the gestures are
 * all this page's.
 *
 * Names come from MA's own control map, which the station reads out of a local
 * grandMA3 install and hands over through `wing.map` — so nothing here carries MA's
 * table, only the handful of codes that mean something to this console.
 */

import { fadeGroup } from './fade.js';
import type { BindingAction, FadeGroup, ParameterKind, ParameterValue, SurfaceControl } from './generated/index.js';
import { displayLabel, parameterKey } from './patch.js';
import type { EditableParameter } from './programmer.js';

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

/** What a pressed, moved or turned control reports. The station's `pult_wing::Event`. */
export type WingEvent =
	| { kind: 'key'; index: number; down: boolean }
	| { kind: 'fader'; index: number; value: number }
	| { kind: 'encoder'; index: number; delta: number }
	| { kind: 'digital'; bits: number };

/** How many digital inputs the wing has, on its rear panel. */
export const DIGITAL_INPUTS = 7;

/**
 * Which digital inputs went down or came up between two readings, lowest first. An
 * input reads as pressed while its bit is set.
 */
export function digitalEdges(before: number, after: number): { index: number; down: boolean }[] {
	const out: { index: number; down: boolean }[] = [];
	for (let index = 0; index < DIGITAL_INPUTS; index++) {
		const was = (before >> index) & 1;
		const now = (after >> index) & 1;
		if (was !== now) out.push({ index, down: now === 1 });
	}
	return out;
}

/** A fader's full scale: the wing's ADC is 12-bit, and the top is one past 4095. */
export const FADER_FULL = 4096;
/** The LED frame. */
export const LED_CHANNELS = 253;
/** How many encoders sit under the screen, each an attribute of the bar. */
export const ENCODERS = 5;

export const faderLevel = (raw: number): number => Math.min(1, Math.max(0, raw / FADER_FULL));
export const faderRaw = (level: number): number => Math.round(Math.min(1, Math.max(0, level)) * FADER_FULL);

// ── The keypad ────────────────────────────────────────────────────────────────

/**
 * What a key types into the command line, by MA's code.
 *
 * The legends already are this console's grammar wherever a word exists for them —
 * `fixture 1 thru 5 at 50`, `group 3`, `store sequence 1 cue 4` — so a key types its
 * word, and only the keys that have one are here. A key with no word does nothing
 * unless something is bound to it.
 */
const KEYPAD: Record<string, string> = {
	NUM0: '0',
	NUM1: '1',
	NUM2: '2',
	NUM3: '3',
	NUM4: '4',
	NUM5: '5',
	NUM6: '6',
	NUM7: '7',
	NUM8: '8',
	NUM9: '9',
	DOT: '.',
	PLUS: '+',
	MINUS: '-',
	THRU: 'thru',
	AT: 'at',
	FULL: 'full',
	FIXTURE: 'fixture',
	CHANNEL: 'fixture',
	GROUP: 'group',
	PRESET: 'preset',
	CUE: 'cue',
	SEQUENCE: 'sequence',
	STORE: 'store',
	UPDATE: 'update',
	DELETE: 'delete',
	HELP: 'help',
	OFF: 'off'
};

/** The word a key types, or `undefined` for a key that types nothing. */
export const keypadWord = (code: string | undefined): string | undefined =>
	code === undefined ? undefined : KEYPAD[code];

/** Every code that types something, for lighting the keypad. */
export const KEYPAD_CODES = new Set(Object.keys(KEYPAD));

/** What the keys that are not words do. */
export type KeyVerb =
	| 'please'
	| 'clear'
	| 'escape'
	| 'oops'
	| 'go'
	| 'goBack'
	| 'pageUp'
	| 'pageDown'
	| 'learn';

const VERBS: Record<string, KeyVerb> = {
	PLEASE: 'please',
	CLEAR: 'clear',
	ESC: 'escape',
	UNDO: 'oops',
	GO: 'go',
	DEF_GO: 'go',
	GOBACK: 'goBack',
	DEF_GOBACK: 'goBack',
	UP: 'pageUp',
	DOWN: 'pageDown',
	LEARN: 'learn'
};

export const keyVerb = (code: string | undefined): KeyVerb | undefined =>
	code === undefined ? undefined : VERBS[code];

const isDigit = (c: string) => c >= '0' && c <= '9';

/**
 * The line with one more key typed onto it.
 *
 * Digits run together, so `1` `2` is twelve; everything else is a word of its own,
 * and that includes the dot — `fixture 1 thru 20 . 1 thru 4` cuts cells, where a dot
 * between two digits would make one number of them.
 */
export function typed(line: string, word: string): string {
	const trimmed = line.trimEnd();
	if (trimmed === '') return word;
	const last = trimmed[trimmed.length - 1];
	if (word.length === 1 && isDigit(word) && isDigit(last)) return trimmed + word;
	return `${trimmed} ${word}`;
}

/** MA's Oops on a line being typed: the last word goes, a run of digits being one. */
export function oops(line: string): string {
	const words = line.trim().split(/\s+/).filter(Boolean);
	words.pop();
	return words.join(' ');
}

// ── Faders and their keys ─────────────────────────────────────────────────────

/** Fader index → MA's executor number, `"201"`… */
export function faderExecutors(map: WingMap | null): Map<number, string> {
	const out = new Map<number, string>();
	for (const [index, control] of Object.entries(map?.faders ?? {})) {
		if (control.ExecutorIndex) out.set(Number(index), control.ExecutorIndex);
	}
	return out;
}

/** What a hardkey is, in relation to a fader. */
export type FaderKey =
	| { role: 'touch'; fader: number }
	/** The key of the fader's own executor: Go, for a fader that is a master. */
	| { role: 'go'; fader: number }
	/** The key of the executor a hundred below it in the same column: Off. */
	| { role: 'off'; fader: number };

/**
 * Which hardkeys belong to which fader.
 *
 * MA models touching a fader as a hardkey with the code `FADER` and the fader's own
 * executor, which is how a surface gets grab and release without inferring them from
 * motion. The `EXEC` key of the same executor is the one under the fader, and the
 * `EXEC` key a hundred below it is the one above, in the same column.
 */
export function faderKeys(map: WingMap | null): Map<number, FaderKey> {
	const byExecutor = new Map<string, number>();
	for (const [fader, executor] of faderExecutors(map)) byExecutor.set(executor, fader);
	const out = new Map<number, FaderKey>();
	for (const [index, control] of Object.entries(map?.hardkeys ?? {})) {
		const executor = control.ExecutorIndex;
		if (!executor) continue;
		if (control.Code === 'FADER' && byExecutor.has(executor)) {
			out.set(Number(index), { role: 'touch', fader: byExecutor.get(executor)! });
		} else if (control.Code === 'EXEC') {
			if (byExecutor.has(executor)) {
				out.set(Number(index), { role: 'go', fader: byExecutor.get(executor)! });
			}
			const above = String(Number(executor) + 100);
			if (byExecutor.has(above)) {
				out.set(Number(index), { role: 'off', fader: byExecutor.get(above)! });
			}
		}
	}
	return out;
}

// ── The encoders ──────────────────────────────────────────────────────────────

/** Which of the five an encoder is, and whether it is the fine half of it. */
export type EncoderRole = { slot: number; fine: boolean };

/**
 * The five dual encoders under the screen: MA names them `Outside1`…`Inside5`. The
 * inner knob is coarse and the outer ring fine, for the same parameter, as MA's own
 * manual has it — an operator who has stood at an MA reaches for the inner knob to get
 * somewhere and the ring to land.
 */
export function encoderRole(map: WingMap | null, index: number): EncoderRole | undefined {
	const type = map?.encoders[String(index)]?.Type;
	const match = type?.match(/^(Inside|Outside)([1-5])$/);
	if (!match) return undefined;
	return { slot: Number(match[2]) - 1, fine: match[1] === 'Outside' };
}

/** One position on the encoder bar: a parameter, or one channel of a color. */
export type EncoderSlot = {
	kind: ParameterKind;
	key: string;
	label: string;
	component?: 'r' | 'g' | 'b';
	defaultValue: ParameterValue;
};

const GROUP_ORDER: FadeGroup[] = ['Intensity', 'Position', 'Color', 'Beam', 'Other'];

/**
 * What the encoders can reach for this selection, in the order they page through it.
 *
 * Intensity first, then position, color, beam and the rest — the order an operator
 * builds a look in — and the selection's own order within each. A color is three
 * slots, red, green and blue, because a knob turns one number. A line of text has no
 * number to turn and is left out.
 */
export function encoderSlots(rows: EditableParameter[]): EncoderSlot[] {
	const sorted = [...rows].sort(
		(a, b) => GROUP_ORDER.indexOf(fadeGroup(a.key)) - GROUP_ORDER.indexOf(fadeGroup(b.key))
	);
	const out: EncoderSlot[] = [];
	for (const row of sorted) {
		const base = { kind: row.kind, key: row.key, defaultValue: row.defaultValue };
		if (row.defaultValue.type === 'Text') continue;
		if (row.defaultValue.type === 'Color') {
			out.push({ ...base, label: 'Red', component: 'r' });
			out.push({ ...base, label: 'Green', component: 'g' });
			out.push({ ...base, label: 'Blue', component: 'b' });
		} else {
			out.push({ ...base, label: row.label || displayLabel(row.kind) });
		}
	}
	return out;
}

/** The slots on one page of five, padded with nothing past the end. */
export function encoderPage(slots: EncoderSlot[], page: number): (EncoderSlot | null)[] {
	return Array.from({ length: ENCODERS }, (_, i) => slots[page * ENCODERS + i] ?? null);
}

export const pageCount = (slots: EncoderSlot[]): number => Math.max(1, Math.ceil(slots.length / ENCODERS));

/** One click of the inner knob is 2% of the travel; of the outer ring a tenth of that, MA's ratio. */
export const COARSE = 0.02;
export const FINE = 0.002;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/**
 * A value after `clicks` of an encoder.
 *
 * A float moves by a fraction of its travel; a whole number by one step a click
 * however it is turned, because the steps of a gobo wheel are the steps; a switch goes
 * on turned up and off turned down; one channel of a color moves alone.
 */
export function turned(
	value: ParameterValue,
	clicks: number,
	fine: boolean,
	component?: 'r' | 'g' | 'b'
): ParameterValue {
	const step = (fine ? FINE : COARSE) * clicks;
	switch (value.type) {
		case 'Float':
			return { type: 'Float', value: clamp01(value.value + step) };
		case 'Int':
			return { type: 'Int', value: Math.max(0, value.value + Math.sign(clicks) * Math.max(1, Math.abs(clicks))) };
		case 'Bool':
			return clicks === 0 ? value : { type: 'Bool', value: clicks > 0 };
		case 'Color': {
			if (!component) return value;
			return {
				type: 'Color',
				value: { ...value.value, [component]: clamp01(value.value[component] + step) }
			};
		}
		default:
			return value;
	}
}

// ── Typing a value: the press of a large encoder ──────────────────────────────

/**
 * Which of the five large encoders a key is the press of, or `undefined`. MA's table
 * names an inner and an outer press for each; on this wing only the inner knob can be
 * pressed, but both are read so a wing whose ring presses too is not left with a key
 * that means nothing.
 */
export function encoderPress(code: string | undefined): number | undefined {
	const match = code?.match(/^ENCODER_(INSIDE|OUTSIDE)([1-5])$/);
	return match ? Number(match[2]) - 1 : undefined;
}

/**
 * The calculator's text after one keypad word: digits and one dot, as many as a
 * value needs. Any other word types nothing — the calculator is a number, and `at`
 * going into it would be a value nobody can read.
 */
export function calculatorTyped(text: string, word: string): string {
	if (/^\d$/.test(word)) return text.length < 8 ? text + word : text;
	if (word === '.') return text.includes('.') ? text : (text === '' ? '0' : text) + '.';
	return text;
}

/**
 * What the typed text sets an attribute to, read in the units its bar shows: a
 * percentage for a level or one channel of a color, a step for a wheel, zero or not
 * for a switch. `null` when there is nothing to set, or nothing the text could mean.
 */
export function typedValue(
	text: string,
	like: ParameterValue,
	component?: 'r' | 'g' | 'b'
): ParameterValue | null {
	if (text === '' || text === '.') return null;
	const n = Number(text);
	if (!Number.isFinite(n)) return null;
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: clamp01(n / 100) };
		case 'Int':
			return { type: 'Int', value: Math.max(0, Math.round(n)) };
		case 'Bool':
			return { type: 'Bool', value: n !== 0 };
		case 'Color':
			return component ? { type: 'Color', value: { ...like.value, [component]: clamp01(n / 100) } } : null;
		default:
			return null;
	}
}

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

/** Full, for a key flashing a parameter: the top of a level, white for a color, on. */
export function full(like: ParameterValue): ParameterValue {
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: 1 };
		case 'Int':
			return { type: 'Int', value: 255 };
		case 'Bool':
			return { type: 'Bool', value: true };
		case 'Color':
			return { type: 'Color', value: { r: 1, g: 1, b: 1, overrides: {} } };
		default:
			return like;
	}
}

/** A color's hue: its channels with the brightest at 1. `null` for black, which has none. */
export function hueOf(value: ParameterValue): { r: number; g: number; b: number } | null {
	if (value.type !== 'Color') return null;
	const { r, g, b } = value.value;
	const top = Math.max(r, g, b);
	return top > 0 ? { r: r / top, g: g / top, b: b / top } : null;
}

/**
 * A fader position as a value of the kind it moves.
 *
 * A fader on a color is its brightness, and the hue it scales is `hue` — what the color
 * was last time it was anything but black. Taken from the color itself it would be
 * lost the moment the fader reached the bottom: black has no hue, so every position
 * after that would scale black to black and the fader would be dead.
 */
export function fromFader(
	like: ParameterValue,
	level: number,
	hue: { r: number; g: number; b: number } | null = null
): ParameterValue {
	switch (like.type) {
		case 'Float':
			return { type: 'Float', value: level };
		case 'Int':
			return { type: 'Int', value: Math.round(level * 255) };
		case 'Bool':
			return { type: 'Bool', value: level >= 0.5 };
		case 'Color': {
			const { r, g, b } = hue ?? hueOf(like) ?? { r: 1, g: 1, b: 1 };
			return { type: 'Color', value: { r: r * level, g: g * level, b: b * level, overrides: like.value.overrides } };
		}
		default:
			return like;
	}
}

/** Where a motor should put a fader for a value, or `null` for one it cannot show. */
export function toFader(value: ParameterValue | null): number | null {
	if (!value) return null;
	switch (value.type) {
		case 'Float':
			return value.value;
		case 'Int':
			return value.value / 255;
		case 'Bool':
			return value.value ? 1 : 0;
		case 'Color':
			return Math.max(value.value.r, value.value.g, value.value.b);
		default:
			return null;
	}
}

export const keyOf = (kind: ParameterKind): string => parameterKey(kind);

// ── Lights ────────────────────────────────────────────────────────────────────

/** The LED channels of a key, by code or by executor, as MA's table places them. */
export function ledChannels(led: WingLed): number[] {
	return [led.R, led.G, led.B].filter((c): c is number => typeof c === 'number');
}

/** How lit a key is: off, dim for "you can press this", bright for "this is live". */
export const DIM = 24;
export const LIT = 255;

/**
 * Which hardkey an LED lights, or `undefined` for one that lights no key.
 *
 * A key's LED carries the key's own code — `STORE`, `X3` — and an executor button's
 * carries no code and the executor's number. An LED with three channels and no code
 * is an encoder's ring, and the one with neither is the desk lamp; neither is a key.
 */
export function ledKeys(map: WingMap | null): Map<WingLed, number> {
	const byCode = new Map<string, number>();
	const byExecutor = new Map<string, number>();
	for (const [index, control] of Object.entries(map?.hardkeys ?? {})) {
		if (control.Code === 'EXEC' && control.ExecutorIndex) byExecutor.set(control.ExecutorIndex, Number(index));
		else if (control.Code && control.Code !== 'FADER') byCode.set(control.Code, Number(index));
	}
	const out = new Map<WingLed, number>();
	for (const led of map?.leds ?? []) {
		const key =
			led.Code && led.Code !== 'EXEC'
				? byCode.get(led.Code)
				: !led.Code && led.ExecutorIndex && led.G === undefined
					? byExecutor.get(led.ExecutorIndex)
					: undefined;
		if (key !== undefined) out.set(led, key);
	}
	return out;
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

