/**
 * Layouts: a drawing of somewhere, with what is happening there painted on it.
 *
 * The schema says what an element points at (`types::layout`); this is everything a
 * page works out from that — what color an element is at a moment, what it says, what
 * pressing it does, where the canvas sits on a screen and where a dragged element lands.
 * Pure, so the desk's panel and the page a phone is handed agree by calling one
 * function, and so it can be tested without a station.
 *
 * Three ways an element gets its color, and they are not interchangeable:
 *
 * - **An on/off reading decays.** A contact, a PIR, a flag variable: lit in the active
 *   color while on, then fading to the idle color over `decay_ms` from the moment it
 *   went off. For a sensed parameter that moment is the station's own stamp
 *   (`Fixture::sensed_at`), so two phones agree; for a variable there is no stamp, and
 *   the page notes when *it* saw the flag go off — the weaker case, written down at
 *   {@link noteFlag}.
 * - **A number is colored by its stops** and printed: a temperature, a count.
 * - **A driven parameter is what the rig is putting out**, evaluated in the page the way
 *   the surface's `Parameter` tile does — `valueColor` over the output store.
 *
 * The fade is a straight line in sRGB, deliberately: sRGB is close enough to even in
 * lightness that the glow dims at a steady rate to the eye, where a lerp in linear light
 * holds bright for most of the fade and drops at the end — which reads as somebody
 * still standing there.
 */

import type {
	AudioClip,
	Decay,
	ElementShape,
	ElementTarget,
	Fixture,
	FixtureType,
	Layout,
	LayoutElement,
	ParameterDefinition,
	ParameterKind,
	ParameterValue,
	TileTarget,
	Variable
} from './generated/index.js';
import { currentValue, variableLabel } from './flow.js';
import { displayLabel, formatValue, inCell, kindLabel, parameterForKey, parameterKey } from './patch.js';
import {
	flowButtons,
	inkOn,
	stopColor,
	timeLeft,
	timelineLength,
	timelinePosition,
	timelineProgress,
	valueColor,
	type SurfaceShow
} from './surface.js';
import { uuid } from './uuid.js';

/**
 * What a layout's elements can point at: the show, as far as a layout reads it. The
 * flows are optional because only a flow button needs them, and a page or a test that
 * has none simply has no buttons to name.
 */
export type LayoutShow = Pick<SurfaceShow, 'variables' | 'timelines' | 'fixtures' | 'types'> &
	Partial<Pick<SurfaceShow, 'flows' | 'flowNodes'>>;

/** A layout's flow buttons, by name, as the surface lists them. */
export const layoutButtons = (show: LayoutShow): { id: string; name: string }[] =>
	flowButtons({ flows: show.flows ?? [], flowNodes: show.flowNodes ?? [] });

// ── Decay ─────────────────────────────────────────────────────────────────────

/** `DEFAULT_ACTIVE_COLOR` in `types/layout.rs`: a contact somebody is on. */
export const DEFAULT_ACTIVE_COLOR = '#ff9f1c';
/** `DEFAULT_IDLE_COLOR`: one nobody has been near. */
export const DEFAULT_IDLE_COLOR = '#2b313a';
/** `DEFAULT_DECAY_MS`: how long a contact stays warm after it lets go. */
export const DEFAULT_DECAY_MS = 30_000;

/** A decay with nothing left to fall back to. */
export type ResolvedDecay = { active: string; idle: string; ms: number };

/**
 * The element's answer, with anything it leaves out taken from the layout's and the
 * defaults under both. `Decay::over`.
 */
export function resolveDecay(element: Decay | null | undefined, layout: Decay | null | undefined): ResolvedDecay {
	return {
		active: element?.active_color || layout?.active_color || DEFAULT_ACTIVE_COLOR,
		idle: element?.idle_color || layout?.idle_color || DEFAULT_IDLE_COLOR,
		ms: element?.decay_ms ?? layout?.decay_ms ?? DEFAULT_DECAY_MS
	};
}

function rgbOf(color: string): [number, number, number] | null {
	const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color.trim());
	return m ? [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)] : null;
}

/**
 * `from` at 0, `to` at 1, a straight line in sRGB between them — see the module docs
 * for why sRGB. A color this cannot read is answered as `to`, the resting color.
 */
export function mixColor(from: string, to: string, t: number): string {
	const a = rgbOf(from);
	const b = rgbOf(to);
	if (!a || !b) return to;
	const k = Math.min(1, Math.max(0, t));
	const hex = (i: number) =>
		Math.round(a[i] + (b[i] - a[i]) * k)
			.toString(16)
			.padStart(2, '0');
	return `#${hex(0)}${hex(1)}${hex(2)}`;
}

/** A color, and whether it will be a different one a frame from now. */
export type Lamp = { color: string; fading: boolean };

/**
 * What an on/off reading looks like at `now`.
 *
 * On is the active color, and needs no clock. Off is the fade from active to idle over
 * `decay.ms` from `changedAt`, the moment it went off — idle once that is over, idle
 * for a reading that has never been on (no stamp), and **idle when the page does not
 * know the station's clock yet**: a fade measured against an unadjusted clock is a
 * plausible wrong answer, and a phone that says "nobody was here" for a second until
 * it has an offset is the honest one.
 */
export function decayLamp(on: boolean, changedAt: number | null, now: number | null, decay: ResolvedDecay): Lamp {
	if (on) return { color: decay.active, fading: false };
	if (changedAt === null || now === null || decay.ms <= 0) return { color: decay.idle, fading: false };
	// A stamp a little ahead of this page's estimate of the station's clock is the
	// estimate's error, not the future: it reads as having just gone off.
	const elapsed = Math.max(0, now - changedAt);
	if (elapsed >= decay.ms) return { color: decay.idle, fading: false };
	return { color: mixColor(decay.active, decay.idle, elapsed / decay.ms), fading: true };
}

/** What a page remembers about a flag it has watched: whether it was on, and when it went off. */
export type FlagMemory = Map<string, { on: boolean; offAt: number | null }>;

/**
 * Note what a flag is now, and answer when it went off — or `null` for "not that this
 * page saw".
 *
 * **The weaker case, and on purpose.** A variable carries no stamp: it is written by a
 * flow, a phone and an operator alike, and stamping it would be a write per change on
 * the link carrying the show for one drawing's benefit. So a page notes the moment it
 * *saw* a flag go off, and a page opened after that moment shows it idle rather than
 * fading — and two phones opened at different times can disagree about a flag, where
 * they never disagree about a contact. A flag first seen off is idle; a flag seen off
 * with no clock has nothing to measure the fade from, and is idle too.
 */
export function noteFlag(memory: FlagMemory, id: string, on: boolean, now: number | null): number | null {
	const before = memory.get(id);
	if (!before) {
		memory.set(id, { on, offAt: null });
		return null;
	}
	if (on) {
		if (!before.on || before.offAt !== null) memory.set(id, { on: true, offAt: null });
		return null;
	}
	if (before.on) {
		memory.set(id, { on: false, offAt: now });
		return now;
	}
	return before.offAt;
}

// ── What an element shows ─────────────────────────────────────────────────────

/** A shape's fill when nobody chose one: a room on a dark page. */
export const SHAPE_COLOR = '#3a4250';
/** A line's stroke when nobody chose one. */
export const LINE_COLOR = '#9aa4b2';
/** Words when nobody chose a color for them. */
export const TEXT_COLOR = '#e5e7eb';
/** A flow button when nobody chose a color for it: the surface's button blue. */
export const BUTTON_COLOR = '#2d3a4f';

/** Everything an element needs drawn, worked out once per frame. */
export type Look = {
	/** The fill, or `null` for an element that has none (a line, words). */
	fill: string | null;
	/** A line's stroke. */
	stroke: string | null;
	/** What the words are written in. */
	ink: string;
	/** The label, or `''` where none is drawn. */
	label: string;
	/** A reading in words beside the label: a temperature, a count, the time left. */
	figure: string | null;
	/** How far through a timeline is, for the bar along its foot. */
	fraction: number | null;
	/** Whether the next frame will look different: a fade in flight, a timeline running. */
	animating: boolean;
	/** The thing it points at is gone, which the element then says. */
	missing: boolean;
};

/** What a look is worked out against, besides the show. */
export type LookContext = {
	/** The console millisecond, or `null` when this page cannot place itself on the clock. */
	now: number | null;
	/** What the rig is putting out on a parameter, from the output store. */
	driven: (fixtureId: string, key: string) => ParameterValue | null;
	/** When a flag went off, as this page saw it: {@link noteFlag}. */
	flagOffAt: (variableId: string, on: boolean) => number | null;
	/** How long a clip's file is, where the page knows. */
	fileMs?: (clip: AudioClip) => number | null;
};

/** A fixture's type and the definition one of its parameters reads back to. */
function definitionOf(
	show: Pick<LayoutShow, 'fixtures' | 'types'>,
	fixture: Fixture,
	kind: ParameterKind
): ParameterDefinition | null {
	const type = show.types.find((t) => t.id === fixture.fixture_type_id);
	return type ? parameterForKey(type, parameterKey(kind)) : null;
}

/** Whether a parameter is one the device reports rather than one the console drives. */
export function isSensed(show: Pick<LayoutShow, 'fixtures' | 'types'>, fixtureId: string, kind: ParameterKind): boolean {
	const fixture = show.fixtures.find((f) => f.id === fixtureId);
	return !!fixture && definitionOf(show, fixture, kind)?.direction === 'Input';
}

/**
 * What a target is called, for an element nobody gave a label — or `null` when what it
 * points at is gone. A drawing piece is called nothing.
 */
export function targetName(target: ElementTarget, show: LayoutShow): string | null {
	switch (target.kind) {
		case 'Parameter': {
			const fixture = show.fixtures.find((f) => f.id === target.fixture_id);
			return fixture ? `${fixture.name} ${displayLabel(target.parameter)}` : null;
		}
		case 'Variable':
			return show.variables.find((v) => v.id === target.variable_id)?.name ?? null;
		case 'Timeline':
			return show.timelines.find((t) => t.id === target.timeline_id)?.name ?? null;
		case 'FlowButton':
			return layoutButtons(show).find((b) => b.id === target.node_id)?.name ?? null;
		default:
			return '';
	}
}

/** What an element says: its own label, the name of its target, or that the target is gone. */
export function elementLabel(element: LayoutElement, show: LayoutShow): string {
	if (element.label.trim()) return element.label;
	return targetName(element.target, show) ?? 'missing';
}

/** What an element is called in a list, where it has to be called something. */
export function elementName(element: LayoutElement, show: LayoutShow): string {
	return elementLabel(element, show) || TARGET_KINDS.find((k) => k.kind === element.target.kind)?.label || 'Element';
}

/**
 * A number a device reported, as somebody reads it. A sensor reports in its own units
 * — 19.5 is degrees, not a fraction of anything — so it is printed as the number, with
 * the unit the kind implies.
 */
export function sensedFigure(value: ParameterValue, kind: ParameterKind): string {
	if (value.type !== 'Float' && value.type !== 'Int') return formatValue(value);
	const n = value.value;
	const shown = Number.isInteger(n) ? String(n) : n.toFixed(1);
	switch (kindLabel(kind)) {
		case 'Temperature':
			return `${shown}°`;
		case 'Humidity':
			return `${shown}%`;
		default:
			return shown;
	}
}

const plain = (look: Partial<Look> & Pick<Look, 'ink' | 'label'>): Look => ({
	fill: null,
	stroke: null,
	figure: null,
	fraction: null,
	animating: false,
	missing: false,
	...look
});

/** A number on a stop list, or on the idle color when there is no list. */
const numberColor = (element: LayoutElement, value: number, decay: ResolvedDecay): string =>
	element.stops.length ? stopColor(element.stops, value) : decay.idle;

/** What one element looks like at `ctx.now`. */
export function elementLook(element: LayoutElement, layout: Pick<Layout, 'decay'>, show: LayoutShow, ctx: LookContext): Look {
	const decay = resolveDecay(element.decay, layout.decay);
	const named = elementLabel(element, show);
	const label = element.show_label ? named : '';
	const filled = (fill: string, rest: Partial<Look> = {}): Look => plain({ fill, ink: inkOn(fill), label, ...rest });
	const missing = (): Look =>
		plain({ fill: decay.idle, ink: inkOn(decay.idle), label: element.show_label ? named : '', missing: true });
	const target = element.target;

	switch (target.kind) {
		case 'Shape':
			return filled(element.color || SHAPE_COLOR);
		case 'Line': {
			const stroke = element.color || LINE_COLOR;
			return plain({ stroke, ink: stroke, label });
		}
		case 'Text':
			return plain({ ink: element.color || TEXT_COLOR, label });

		case 'Parameter': {
			const fixture = show.fixtures.find((f) => f.id === target.fixture_id);
			if (!fixture) return missing();
			const key = parameterKey(target.parameter);
			const definition = definitionOf(show, fixture, target.parameter);
			if (definition?.direction === 'Input') {
				const value = fixture.sensed_values[key] ?? null;
				if (!value) return filled(decay.idle, { figure: '–' });
				if (value.type === 'Bool') {
					const lamp = decayLamp(value.value, fixture.sensed_at?.[key] ?? null, ctx.now, decay);
					return filled(lamp.color, { animating: lamp.fading });
				}
				if (value.type === 'Float' || value.type === 'Int') {
					return filled(numberColor(element, value.value, decay), { figure: sensedFigure(value, target.parameter) });
				}
				return filled(valueColor(value) ?? decay.idle, { figure: formatValue(value) });
			}
			// Driven: what the rig is putting out, which is `null` until the page has the
			// station's clock — drawn idle rather than guessed.
			const value = ctx.driven(fixture.id, key);
			if (!value) return filled(decay.idle, { figure: ctx.now === null ? null : '–' });
			if (value.type === 'Bool') return filled(value.value ? decay.active : decay.idle);
			const color = valueColor(value, decay.active) ?? decay.idle;
			// A color says itself; a level and a step are worth a figure.
			const figure = value.type === 'Float' || value.type === 'Int' || value.type === 'Text' ? formatValue(value) : null;
			return filled(color, { figure });
		}

		case 'Variable': {
			const variable = show.variables.find((v) => v.id === target.variable_id);
			if (!variable) return missing();
			if (variable.kind === 'Flag') {
				const on = currentValue(variable) !== 0;
				const lamp = decayLamp(on, ctx.flagOffAt(variable.id, on), ctx.now, decay);
				return filled(lamp.color, { animating: lamp.fading });
			}
			return filled(numberColor(element, currentValue(variable), decay), { figure: variableLabel(variable) });
		}

		case 'FlowButton': {
			if (!layoutButtons(show).some((b) => b.id === target.node_id)) return missing();
			return filled(element.color || BUTTON_COLOR);
		}

		case 'Timeline': {
			const timeline = show.timelines.find((t) => t.id === target.timeline_id);
			if (!timeline) return missing();
			const position = timelinePosition(timeline, ctx.now);
			const progress = timelineProgress(position, timelineLength(timeline, ctx.fileMs ?? (() => null)));
			const figure = timeline.running ? (progress ? timeLeft(progress.remainingMs) : 'Playing') : 'Stopped';
			return filled(timeline.running ? decay.active : decay.idle, {
				figure,
				fraction: progress?.fraction ?? null,
				animating: timeline.running
			});
		}
	}
}

/** Which parameters a look reads from the evaluated rig: `"<fixture>/<key>"`, for `watching`. */
export function drivenKeys(layout: Pick<Layout, 'elements'>, show: Pick<LayoutShow, 'fixtures' | 'types'>): string[] {
	return layout.elements.flatMap((e) =>
		e.target.kind === 'Parameter' && !isSensed(show, e.target.fixture_id, e.target.parameter)
			? [`${e.target.fixture_id}/${parameterKey(e.target.parameter)}`]
			: []
	);
}

// ── Pressing one ──────────────────────────────────────────────────────────────

/** What pressing an element does, in the terms of the code that already does it. */
export type LayoutPress = { kind: 'tile'; target: TileTarget } | { kind: 'select'; fixtureId: string } | null;

/**
 * An element's press, as the surface tile that does the same thing — so a layout's
 * variable and timeline presses go through `pressTarget` and nowhere else — or the
 * fixture to select. `null` for a press its target cannot take (a timeline press on a
 * variable), which a hand-edited show could hold.
 */
export function pressOf(element: LayoutElement): LayoutPress {
	const press = element.press;
	const target = element.target;
	// A button is pressed by being one.
	if (target.kind === 'FlowButton') return { kind: 'tile', target: { kind: 'FlowButton', node_id: target.node_id } };
	if (!press) return null;
	if (press.kind === 'Variable' && target.kind === 'Variable') {
		return { kind: 'tile', target: { kind: 'VariableSetter', variable_id: target.variable_id, op: press.op } };
	}
	if (press.kind === 'Timeline' && target.kind === 'Timeline') {
		return { kind: 'tile', target: { kind: 'Timeline', timeline_id: target.timeline_id, from_start: press.from_start } };
	}
	if (press.kind === 'Select' && target.kind === 'Parameter') return { kind: 'select', fixtureId: target.fixture_id };
	return null;
}

/**
 * Whether a shared page opened with `key` presses: the layout has an operating key and
 * this is it. Two links to one layout, so a read-only one and an operating one can be
 * handed out at once — and a key rather than a flag, so the read-only link cannot be
 * made to press by editing the address.
 */
export const operatesWith = (layout: Pick<Layout, 'operate_key'>, key: string | null): boolean =>
	!!layout.operate_key && key === layout.operate_key;

/** A fresh operating key: allowing operating again revokes the links handed out before. */
export const newOperateKey = (): string => uuid().replace(/-/g, '');

/**
 * What pressing an element does on the shared page, when it was opened with the
 * layout's operating key: the desk's presses, less selecting a fixture — a phone has no
 * selection to make, and a press that silently did nothing would read as a fault.
 */
export function sharedPressOf(element: LayoutElement, layout: Pick<Layout, 'operate_key'>, key: string | null): LayoutPress {
	if (!operatesWith(layout, key)) return null;
	const press = pressOf(element);
	return press?.kind === 'tile' ? press : null;
}

/**
 * The frontmost element under a point in canvas units that `can` accepts, or `null`.
 * `slop` widens every box by that much on each side, which is how a page makes a
 * small dot a finger wide.
 */
export function elementAt(
	elements: LayoutElement[],
	point: { x: number; y: number },
	can: (element: LayoutElement) => boolean,
	slop = 0
): LayoutElement | null {
	for (let i = elements.length - 1; i >= 0; i--) {
		const element = elements[i];
		if (!can(element)) continue;
		const b = boxOf(element);
		if (point.x >= b.x - slop && point.x <= b.x + b.w + slop && point.y >= b.y - slop && point.y <= b.y + b.h + slop) {
			return element;
		}
	}
	return null;
}

// ── The canvas on a screen ────────────────────────────────────────────────────

export type Size = { width: number; height: number };
export type Rect = { x: number; y: number; w: number; h: number };

/** Where the canvas is on the screen: CSS pixels per canvas unit, and where its origin lands. */
export type View = { scale: number; x: number; y: number };

/** The whole canvas, as large as it goes and centered, keeping its aspect. */
export function fitView(canvas: Size, screen: Size, margin = 0): View {
	const width = Math.max(1, screen.width - 2 * margin);
	const height = Math.max(1, screen.height - 2 * margin);
	const scale = Math.max(1e-6, Math.min(width / Math.max(canvas.width, 1e-6), height / Math.max(canvas.height, 1e-6)));
	return {
		scale,
		x: (screen.width - canvas.width * scale) / 2,
		y: (screen.height - canvas.height * scale) / 2
	};
}

/**
 * Zoomed by `factor` about a point on the screen, which stays where it is under the
 * fingers. Never smaller than fitting and never more than `most` times that: smaller
 * than fitting is a drawing lost in a black screen, and a phone needs no closer look at
 * a contact than eight times.
 */
export function zoomAt(view: View, factor: number, at: { x: number; y: number }, canvas: Size, screen: Size, most = 8): View {
	const fit = fitView(canvas, screen).scale;
	const scale = Math.min(fit * most, Math.max(fit, view.scale * factor));
	const k = scale / view.scale;
	return clampView({ scale, x: at.x - (at.x - view.x) * k, y: at.y - (at.y - view.y) * k }, canvas, screen);
}

/**
 * Kept on the screen: an axis the canvas is narrower than the screen on is centered,
 * and one it is wider on cannot be dragged past its own edge.
 */
export function clampView(view: View, canvas: Size, screen: Size): View {
	const axis = (offset: number, span: number, room: number) =>
		span <= room ? (room - span) / 2 : Math.min(0, Math.max(room - span, offset));
	return {
		scale: view.scale,
		x: axis(view.x, canvas.width * view.scale, screen.width),
		y: axis(view.y, canvas.height * view.scale, screen.height)
	};
}

/** A point on the screen, in canvas units. */
export const toCanvas = (view: View, x: number, y: number) => ({ x: (x - view.x) / view.scale, y: (y - view.y) / view.scale });

/** Whether a view is (still) the fitted one, so a rotation knows whether to refit. */
export const isFitted = (view: View, canvas: Size, screen: Size): boolean =>
	Math.abs(view.scale - fitView(canvas, screen).scale) < 1e-6 * Math.max(1, view.scale);

// ── Dragging one ──────────────────────────────────────────────────────────────

/** Which part of an element a drag has hold of: all of it, an edge or a corner. */
export type Grip = 'move' | 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

/** The nearest multiple of `grid`, or the value itself with no grid. */
export const snap = (value: number, grid: number): number => (grid > 0 ? Math.round(value / grid) * grid : value);

/** The smallest an element can be dragged to: a grid cell, or one unit with no grid. */
export const smallest = (grid: number): number => (grid > 0 ? grid : 1);

const clamp = (value: number, low: number, high: number) => Math.min(Math.max(value, low), Math.max(low, high));

/**
 * Where an element dragged by `grip` through `dx`, `dy` canvas units from `from` lands.
 *
 * The rule `resizedRect` in `stores/paperwork.ts` follows for a viewport, on a canvas
 * with a grid: what moves snaps, an edge dragged through the opposite one stops a cell
 * short rather than turning the element inside out, and a moved element stays wholly on
 * the canvas — the canvas *is* the drawing, and an element half off it is half missing
 * on every phone.
 */
export function draggedRect(from: Rect, grip: Grip, dx: number, dy: number, canvas: Size, grid: number): Rect {
	if (grip === 'move') {
		return {
			...from,
			x: clamp(snap(from.x + dx, grid), 0, canvas.width - from.w),
			y: clamp(snap(from.y + dy, grid), 0, canvas.height - from.h)
		};
	}
	const min = smallest(grid);
	let left = from.x;
	let right = from.x + from.w;
	let top = from.y;
	let bottom = from.y + from.h;
	if (grip.includes('w')) left = clamp(snap(left + dx, grid), 0, right - min);
	if (grip.includes('e')) right = clamp(snap(right + dx, grid), left + min, canvas.width);
	if (grip.includes('n')) top = clamp(snap(top + dy, grid), 0, bottom - min);
	if (grip.includes('s')) bottom = clamp(snap(bottom + dy, grid), top + min, canvas.height);
	return { x: left, y: top, w: right - left, h: bottom - top };
}

/** The box an element occupies: its own, or for a line the bounds of its points. */
export function boxOf(element: LayoutElement): Rect {
	if (element.target.kind !== 'Line' || element.target.points.length === 0) {
		return { x: element.x, y: element.y, w: element.w, h: element.h };
	}
	const xs = element.target.points.map((p) => p[0]);
	const ys = element.target.points.map((p) => p[1]);
	const x = Math.min(...xs);
	const y = Math.min(...ys);
	return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}

/**
 * The element put in a box. A line's points go with it — moved with a move, stretched
 * with a resize — because a line's box is its points' bounds and not a place of its own.
 */
export function placed(element: LayoutElement, rect: Rect): LayoutElement {
	if (element.target.kind !== 'Line') return { ...element, ...rect };
	const box = boxOf(element);
	const along = (p: number, from: number, span: number, to: number, toSpan: number) =>
		span > 0 ? to + ((p - from) / span) * toSpan : p + (to - from);
	const points = element.target.points.map(
		([px, py]) => [along(px, box.x, box.w, rect.x, rect.w), along(py, box.y, box.h, rect.y, rect.h)] as [number, number]
	);
	return withPoints({ ...element, target: { kind: 'Line', points } }, points);
}

/** A line with its box made the bounds of these points again. */
export function withPoints(element: LayoutElement, points: [number, number][]): LayoutElement {
	const line = { ...element, target: { kind: 'Line' as const, points } };
	return { ...line, ...boxOf(line) };
}

// ── Several at once ───────────────────────────────────────────────────────────

/** The rectangle two corners span, whichever way the pointer went. */
export function rectBetween(a: { x: number; y: number }, b: { x: number; y: number }): Rect {
	return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

/** Whether two boxes share any of the canvas; touching edges count, so a wall is caught. */
export const boxesMeet = (a: Rect, b: Rect): boolean =>
	a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h;

/**
 * What a lasso catches: every element whose box it touches, in drawing order. Touching
 * rather than enclosing, which is the flow editor's lasso too — a room drawn as a big
 * rectangle would otherwise be uncatchable without dragging round the whole canvas.
 */
export function inLasso(elements: LayoutElement[], lasso: Rect): string[] {
	return elements.filter((e) => boxesMeet(boxOf(e), lasso)).map((e) => e.id);
}

/** The bounds of several boxes. */
export function boundsOf(rects: Rect[]): Rect | null {
	if (rects.length === 0) return null;
	const x = Math.min(...rects.map((r) => r.x));
	const y = Math.min(...rects.map((r) => r.y));
	return {
		x,
		y,
		w: Math.max(...rects.map((r) => r.x + r.w)) - x,
		h: Math.max(...rects.map((r) => r.y + r.h)) - y
	};
}

/**
 * Several elements dragged by `dx`, `dy` from where they were (`from`, by id), as one:
 * their bounds snap and stay on the canvas, and every element moves by what the bounds
 * moved by. Each snapped on its own, a group a half cell off the grid came apart, and
 * one pushed against an edge stopped while the rest went on.
 */
export function movedGroup(
	elements: LayoutElement[],
	from: Map<string, Rect>,
	dx: number,
	dy: number,
	canvas: Size,
	grid: number
): LayoutElement[] {
	const bounds = boundsOf([...from.values()]);
	if (!bounds) return elements;
	const to = draggedRect(bounds, 'move', dx, dy, canvas, grid);
	const mx = to.x - bounds.x;
	const my = to.y - bounds.y;
	return elements.map((e) => {
		const was = from.get(e.id);
		return was ? placed(e, { ...was, x: was.x + mx, y: was.y + my }) : e;
	});
}

/** What a copy holds: the elements as they were, back to front. */
export type LayoutClip = { elements: LayoutElement[] };

/** The clipboard's key in `localStorage`, beside the flow editor's. */
export const LAYOUT_CLIP_KEY = 'pult.layoutClipboard';

/** The chosen elements, in drawing order, as a clip. */
export function copyElements(elements: LayoutElement[], ids: Set<string>): LayoutClip {
	return { elements: elements.filter((e) => ids.has(e.id)) };
}

/**
 * A clip pasted: new ids, kept in formation, centered on `at` — a grid cell down and
 * right for as long as it would land exactly on elements already there, so pasting
 * twice gives two copies to see — and wholly on the canvas.
 */
export function pastedElements(
	clip: LayoutClip,
	at: { x: number; y: number },
	existing: LayoutElement[],
	canvas: Size,
	grid: number,
	makeId: () => string = uuid
): LayoutElement[] {
	const from = new Map(clip.elements.map((e) => [e.id, boxOf(e)]));
	const bounds = boundsOf([...from.values()]);
	if (!bounds) return [];
	const step = smallest(grid) * (grid > 0 ? 1 : 5);
	const taken = new Set(existing.map((e) => JSON.stringify(boxOf(e))));
	let dx = at.x - bounds.w / 2 - bounds.x;
	let dy = at.y - bounds.h / 2 - bounds.y;
	let moved = movedGroup(clip.elements, from, dx, dy, canvas, grid);
	for (let i = 0; i < 50 && moved.some((e) => taken.has(JSON.stringify(boxOf(e)))); i++) {
		dx += step;
		dy += step;
		moved = movedGroup(clip.elements, from, dx, dy, canvas, grid);
	}
	return moved.map((e) => ({ ...e, id: makeId() }));
}

// ── Editing an element list ───────────────────────────────────────────────────

export const patchElement = (elements: LayoutElement[], id: string, change: Partial<LayoutElement>): LayoutElement[] =>
	elements.map((e) => (e.id === id ? { ...e, ...change } : e));

export const removeElements = (elements: LayoutElement[], ids: Set<string>): LayoutElement[] =>
	elements.filter((e) => !ids.has(e.id));

/** Drawn last, which is on top: the list is back to front. */
export function toFront(elements: LayoutElement[], id: string): LayoutElement[] {
	const element = elements.find((e) => e.id === id);
	return element ? [...elements.filter((e) => e.id !== id), element] : elements;
}

export function toBack(elements: LayoutElement[], id: string): LayoutElement[] {
	const element = elements.find((e) => e.id === id);
	return element ? [element, ...elements.filter((e) => e.id !== id)] : elements;
}

/** A copy of an element a grid cell down and right, with an id of its own. */
export function duplicated(element: LayoutElement, canvas: Size, grid: number, id: string = uuid()): LayoutElement {
	const step = smallest(grid) * (grid > 0 ? 1 : 5);
	const box = boxOf(element);
	const rect = draggedRect(box, 'move', step, step, canvas, grid);
	return { ...placed(element, rect), id };
}

export type TargetKind = ElementTarget['kind'];

/** Every kind of element, in the order the Add menu offers them. */
export const TARGET_KINDS: { kind: TargetKind; label: string }[] = [
	{ kind: 'Parameter', label: 'Fixture parameter' },
	{ kind: 'Variable', label: 'Variable' },
	{ kind: 'Timeline', label: 'Timeline' },
	{ kind: 'FlowButton', label: 'Flow button' },
	{ kind: 'Shape', label: 'Shape' },
	{ kind: 'Line', label: 'Line' },
	{ kind: 'Text', label: 'Text' }
];

/** How big a new element is, against the canvas: an eighth of its short side, on the grid. */
export function elementSize(canvas: Size, grid: number): number {
	const raw = Math.min(canvas.width, canvas.height) / 8;
	return Math.max(smallest(grid), grid > 0 ? Math.round(raw / grid) * grid : Math.round(raw));
}

/**
 * Every parameter a type *reports*, as the kind it is held under — `controls()`'s other
 * half: a whole parameter once, a per-cell one once per cell. What dropping a fixture on
 * a layout places an element for.
 */
export function inputsOf(type: FixtureType): { kind: ParameterKind; parameter: ParameterDefinition }[] {
	const inputs = type.parameters.filter((p) => p.direction === 'Input');
	const out = inputs.filter((p) => !(p.per_cell && type.cells > 0)).map((p) => ({ kind: p.kind, parameter: p }));
	if (type.cells > 0) {
		for (let cell = 1; cell <= type.cells; cell++) {
			for (const p of inputs.filter((p) => p.per_cell)) out.push({ kind: inCell(p.kind, cell), parameter: p });
		}
	}
	return out;
}

/** The fixtures that report something, which are the ones worth dragging onto a layout. */
export function sensingFixtures(show: Pick<LayoutShow, 'fixtures' | 'types'>): Fixture[] {
	return show.fixtures.filter((f) => {
		const type = show.types.find((t) => t.id === f.fixture_type_id);
		return !!type && inputsOf(type).length > 0;
	});
}

const blank = (id: string, rect: Rect, target: ElementTarget, shape: ElementShape = 'Rect'): LayoutElement => ({
	id,
	...rect,
	shape,
	label: '',
	show_label: true,
	color: null,
	target,
	decay: {},
	stops: [],
	press: null
});

/**
 * One element per reported parameter of each fixture, laid out in rows from `origin`
 * for somebody to arrange: an on/off reading as a dot, a number as a box wide enough for
 * its figure. Rows wrap at the canvas's right edge, and whatever does not fit below is
 * stacked at the last row rather than placed off the canvas.
 */
export function sensorElements(
	fixtures: Fixture[],
	types: FixtureType[],
	origin: { x: number; y: number },
	canvas: Size,
	grid: number,
	id: () => string = uuid,
	/** Which reported parameters to place, by fixture and key; every one when absent. */
	pick?: (fixtureId: string, key: string) => boolean
): LayoutElement[] {
	const size = elementSize(canvas, grid);
	const gap = smallest(grid) * (grid > 0 ? 1 : 2);
	const out: LayoutElement[] = [];
	const left = clamp(snap(origin.x, grid), 0, canvas.width - size);
	let x = left;
	let y = clamp(snap(origin.y, grid), 0, canvas.height - size);
	for (const fixture of fixtures) {
		const type = types.find((t) => t.id === fixture.fixture_type_id);
		if (!type) continue;
		for (const { kind, parameter } of inputsOf(type)) {
			if (pick && !pick(fixture.id, parameterKey(kind))) continue;
			const flag = parameter.default_value.type === 'Bool';
			const w = flag ? size : size * 2;
			if (x + w > canvas.width && x > left) {
				x = left;
				y = clamp(y + size + gap, 0, canvas.height - size);
			}
			const rect = { x: clamp(x, 0, canvas.width - w), y, w: Math.min(w, canvas.width), h: size };
			out.push(blank(id(), rect, { kind: 'Parameter', fixture_id: fixture.id, parameter: kind }, flag ? 'Ellipse' : 'Rect'));
			x += w + gap;
		}
	}
	return out;
}

/**
 * A new element of a kind, at a place, pointing at the first thing of its kind the show
 * has — or `null` when it has none. A parameter prefers `fixtureId`'s, and a reported
 * one over a driven one, since a layout is mostly a map of what the rig is telling us.
 */
export function newElement(
	kind: TargetKind,
	show: LayoutShow,
	center: { x: number; y: number },
	canvas: Size,
	grid: number,
	fixtureId: string | null = null,
	id: string = uuid()
): LayoutElement | null {
	const size = elementSize(canvas, grid);
	const at = (w: number, h: number): Rect => {
		const ww = Math.min(w, canvas.width);
		const hh = Math.min(h, canvas.height);
		return {
			x: clamp(snap(center.x - ww / 2, grid), 0, canvas.width - ww),
			y: clamp(snap(center.y - hh / 2, grid), 0, canvas.height - hh),
			w: ww,
			h: hh
		};
	};
	switch (kind) {
		case 'Parameter': {
			const candidates = [
				...show.fixtures.filter((f) => f.id === fixtureId),
				...show.fixtures.filter((f) => f.id !== fixtureId)
			];
			for (const fixture of candidates) {
				const type = show.types.find((t) => t.id === fixture.fixture_type_id);
				if (!type) continue;
				const sensed = inputsOf(type)[0];
				const parameter = sensed?.kind ?? type.parameters[0]?.kind;
				if (parameter === undefined) continue;
				return blank(id, at(size, size), { kind: 'Parameter', fixture_id: fixture.id, parameter }, 'Ellipse');
			}
			return null;
		}
		case 'Variable': {
			const variable: Variable | undefined = show.variables[0];
			return variable ? blank(id, at(size * 2, size), { kind: 'Variable', variable_id: variable.id }) : null;
		}
		case 'Timeline': {
			const timeline = show.timelines[0];
			return timeline ? blank(id, at(size * 4, size), { kind: 'Timeline', timeline_id: timeline.id }) : null;
		}
		case 'FlowButton': {
			const button = layoutButtons(show)[0];
			return button ? blank(id, at(size * 2, size), { kind: 'FlowButton', node_id: button.id }) : null;
		}
		case 'Shape':
			return { ...blank(id, at(size * 3, size * 2), { kind: 'Shape' }), color: SHAPE_COLOR, show_label: false };
		case 'Line': {
			const rect = at(size * 3, 0);
			return withPoints(
				{ ...blank(id, rect, { kind: 'Line', points: [] }), color: LINE_COLOR, show_label: false },
				[
					[rect.x, rect.y],
					[rect.x + rect.w, rect.y]
				]
			);
		}
		case 'Text':
			return { ...blank(id, at(size * 3, size / 2), { kind: 'Text' }), label: 'Text' };
	}
}

/** A fresh layout's canvas: a landscape screen's shape, on a grid of five. */
export const NEW_LAYOUT = { width: 200, height: 120, grid: 5 } as const;

/**
 * How large an element's words are, in canvas units: as large as the box allows, by
 * height and by how many characters have to fit across it. `shared` is a line that
 * shares the box with another — a label above a figure.
 */
export function labelSize(box: { w: number; h: number }, text: string, shared = false): number {
	const byHeight = box.h * (shared ? 0.32 : 0.5);
	const byWidth = (box.w * 0.9) / Math.max(1, text.length * 0.58);
	return Math.max(0.5, Math.min(byHeight, byWidth));
}

// ── The link ──────────────────────────────────────────────────────────────────

/**
 * Where a phone opens a layout — `surfaceUrl`'s rule, at `/layout/<id>` — and with
 * `key`, the operating link: the same page, pressing.
 */
export function layoutUrl(location: { origin: string; search: string }, id: string, key: string | null = null): string {
	const port = new URLSearchParams(location.search).get('port');
	const query = new URLSearchParams();
	if (port && /^\d+$/.test(port)) query.set('port', port);
	if (key) query.set('key', key);
	const text = query.toString();
	return `${location.origin}/layout/${encodeURIComponent(id)}${text ? `?${text}` : ''}`;
}

/** Whether a path is the shared page of a layout rather than the workspace. */
export const isLayoutPath = (pathname: string): boolean => /^\/layout(\/|$)/.test(pathname);

/** The data transfer type a dragged fixture carries: the Plan's tray already uses it. */
export const FIXTURE_MIME = 'application/x-pult-fixture';
