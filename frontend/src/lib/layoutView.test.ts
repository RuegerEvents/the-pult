import { describe, expect, it } from 'vitest';

import type { LayoutElement, Timeline, Variable } from './generated/index.js';
import {
	boxOf,
	copyElements,
	inLasso,
	movedGroup,
	pastedElements,
	rectBetween,
	clampView,
	decayLamp,
	DEFAULT_ACTIVE_COLOR,
	DEFAULT_DECAY_MS,
	DEFAULT_IDLE_COLOR,
	draggedRect,
	drivenKeys,
	duplicated,
	elementAt,
	elementLabel,
	elementLook,
	sharedPressOf,
	fitView,
	inputsOf,
	isLayoutPath,
	layoutUrl,
	mixColor,
	newElement,
	noteFlag,
	placed,
	pressOf,
	resolveDecay,
	sensedFigure,
	sensorElements,
	toBack,
	toCanvas,
	toFront,
	zoomAt,
	type FlagMemory,
	type LayoutShow,
	type LookContext
} from './layoutView.js';
import { aFixture, aFixtureType, aParameter } from './test-fixtures.js';

const element = (over: Partial<LayoutElement> = {}): LayoutElement => ({
	id: 'e',
	x: 10,
	y: 10,
	w: 10,
	h: 10,
	shape: 'Rect',
	label: '',
	show_label: true,
	color: null,
	target: { kind: 'Shape' },
	decay: {},
	stops: [],
	press: null,
	...over
});

const variable = (over: Partial<Variable> = {}): Variable => ({
	id: 'open',
	name: 'Maze open',
	kind: 'Flag',
	initial: 0,
	min: null,
	max: null,
	value: null,
	...over
});

const timeline = (over: Partial<Timeline> = {}): Timeline =>
	({
		id: 'scare',
		name: 'Scare',
		clips: [],
		detected: null,
		source: 'Internal',
		grid: [],
		markers: [],
		events: [],
		tracks: [],
		flashes: null,
		pending: null,
		speed_master: null,
		node_id: null,
		end: { at_ms: 60_000, then: 'Stop' },
		running: false,
		anchor_ms: 0,
		position_at_anchor_ms: 0,
		rate: 1,
		recording: null,
		armed_by: null,
		...over
	}) as Timeline;

/** A PIR node: one contact it reports, one temperature, and a lamp it is driven on. */
const pirType = aFixtureType({
	id: 'pir',
	parameters: [
		aParameter({ kind: 'Intensity', default_value: { type: 'Float', value: 0 } }),
		aParameter({ kind: { Contact: 0 }, direction: 'Input', default_value: { type: 'Bool', value: false } }),
		aParameter({ kind: 'Temperature', direction: 'Input', default_value: { type: 'Float', value: 0 } })
	]
});

const show = (over: Partial<LayoutShow> = {}): LayoutShow => ({
	variables: [],
	timelines: [],
	fixtures: [aFixture({ id: 'pir1', name: 'PIR 1', fixture_type_id: 'pir' })],
	types: [pirType],
	...over
});

const ctx = (over: Partial<LookContext> = {}): LookContext => ({
	now: 100_000,
	driven: () => null,
	flagOffAt: () => null,
	...over
});

const layout = { decay: {} };

describe('decay', () => {
	it('takes what the element says, then the layout, then the defaults', () => {
		expect(resolveDecay({ decay_ms: 1000 }, { active_color: '#ff0000', decay_ms: 5000 })).toEqual({
			active: '#ff0000',
			idle: DEFAULT_IDLE_COLOR,
			ms: 1000
		});
		expect(resolveDecay({}, {})).toEqual({ active: DEFAULT_ACTIVE_COLOR, idle: DEFAULT_IDLE_COLOR, ms: DEFAULT_DECAY_MS });
	});

	it('takes zero as an answer, a light that goes straight out', () => {
		expect(resolveDecay({ decay_ms: 0 }, { decay_ms: 5000 }).ms).toBe(0);
	});

	it('mixes in sRGB, end to end', () => {
		expect(mixColor('#000000', '#ffffff', 0)).toBe('#000000');
		expect(mixColor('#000000', '#ffffff', 1)).toBe('#ffffff');
		expect(mixColor('#000000', '#ffffff', 0.5)).toBe('#808080');
		expect(mixColor('#000000', '#ffffff', 7)).toBe('#ffffff');
		expect(mixColor('nonsense', '#123456', 0.2)).toBe('#123456');
	});

	const d = { active: '#ff0000', idle: '#000000', ms: 10_000 };

	it('is lit while on, whatever the clock knows', () => {
		expect(decayLamp(true, 5, null, d)).toEqual({ color: '#ff0000', fading: false });
	});

	it('fades from when it went off, and rests at idle afterwards', () => {
		expect(decayLamp(false, 1000, 1000, d)).toEqual({ color: '#ff0000', fading: true });
		expect(decayLamp(false, 1000, 6000, d)).toEqual({ color: '#800000', fading: true });
		expect(decayLamp(false, 1000, 11_000, d)).toEqual({ color: '#000000', fading: false });
	});

	it('is idle for a reading that has never been on', () => {
		expect(decayLamp(false, null, 5000, d).color).toBe('#000000');
	});

	it('draws no fade before the page knows the station clock', () => {
		expect(decayLamp(false, 1000, null, d)).toEqual({ color: '#000000', fading: false });
	});

	it('reads a stamp just ahead of the clock as having just gone off', () => {
		expect(decayLamp(false, 1200, 1000, d)).toEqual({ color: '#ff0000', fading: true });
	});

	it('goes straight out with no decay time', () => {
		expect(decayLamp(false, 1000, 1001, { ...d, ms: 0 }).color).toBe('#000000');
	});
});

describe('a flag the page watched', () => {
	it('is idle the first time it is seen off, and fades from when it was seen going off', () => {
		const memory: FlagMemory = new Map();
		expect(noteFlag(memory, 'v', false, 10)).toBeNull();
		expect(noteFlag(memory, 'v', true, 20)).toBeNull();
		expect(noteFlag(memory, 'v', false, 30)).toBe(30);
		// Asked again, the moment it went off stays the moment it went off.
		expect(noteFlag(memory, 'v', false, 40)).toBe(30);
		expect(noteFlag(memory, 'v', true, 50)).toBeNull();
	});

	it('has nothing to fade from when it went off before the clock was known', () => {
		const memory: FlagMemory = new Map();
		noteFlag(memory, 'v', true, null);
		expect(noteFlag(memory, 'v', false, null)).toBeNull();
	});
});

describe('what an element shows', () => {
	it('lights a contact while it is closed', () => {
		const s = show({
			fixtures: [aFixture({ id: 'pir1', name: 'PIR 1', fixture_type_id: 'pir', sensed_values: { 'Contact:0': { type: 'Bool', value: true } } })]
		});
		const look = elementLook(element({ target: { kind: 'Parameter', fixture_id: 'pir1', parameter: { Contact: 0 } } }), layout, s, ctx());
		expect(look.fill).toBe(DEFAULT_ACTIVE_COLOR);
		expect(look.animating).toBe(false);
		expect(look.label).toBe('PIR 1 Contact 0');
	});

	it('fades a contact from the station stamp of when it opened', () => {
		const s = show({
			fixtures: [
				aFixture({
					id: 'pir1',
					fixture_type_id: 'pir',
					sensed_values: { 'Contact:0': { type: 'Bool', value: false } },
					sensed_at: { 'Contact:0': 100_000 - DEFAULT_DECAY_MS / 2 }
				})
			]
		});
		const look = elementLook(element({ target: { kind: 'Parameter', fixture_id: 'pir1', parameter: { Contact: 0 } } }), layout, s, ctx());
		expect(look.fill).toBe(mixColor(DEFAULT_ACTIVE_COLOR, DEFAULT_IDLE_COLOR, 0.5));
		expect(look.animating).toBe(true);
	});

	it('colors a temperature by its stops and prints it in degrees', () => {
		const s = show({
			fixtures: [aFixture({ id: 'pir1', fixture_type_id: 'pir', sensed_values: { Temperature: { type: 'Float', value: 19.5 } } })]
		});
		const look = elementLook(
			element({
				target: { kind: 'Parameter', fixture_id: 'pir1', parameter: 'Temperature' },
				stops: [
					{ at: 0, color: '#0000ff' },
					{ at: 18, color: '#00ff00' },
					{ at: 25, color: '#ff0000' }
				]
			}),
			layout,
			s,
			ctx()
		);
		expect(look.fill).toBe('#00ff00');
		expect(look.figure).toBe('19.5°');
	});

	it('draws a number with no stops on the idle color', () => {
		const s = show({ variables: [variable({ kind: 'Number', value: 3 })] });
		const look = elementLook(element({ target: { kind: 'Variable', variable_id: 'open' } }), layout, s, ctx());
		expect(look.fill).toBe(DEFAULT_IDLE_COLOR);
		expect(look.figure).toBe('3');
	});

	it('decays a flag from when this page saw it go off', () => {
		const s = show({ variables: [variable({ value: 0 })] });
		const look = elementLook(element({ target: { kind: 'Variable', variable_id: 'open' }, decay: { decay_ms: 1000 } }), layout, s, ctx({ flagOffAt: () => 99_500 }));
		expect(look.fill).toBe(mixColor(DEFAULT_ACTIVE_COLOR, DEFAULT_IDLE_COLOR, 0.5));
	});

	it('shows a driven parameter as the rig is putting it out, and idle before the clock', () => {
		const target = { kind: 'Parameter' as const, fixture_id: 'pir1', parameter: 'Intensity' as const };
		const lit = elementLook(element({ target }), layout, show(), ctx({ driven: () => ({ type: 'Float', value: 1 }) }));
		expect(lit.fill).toBe('#ffffff');
		expect(lit.figure).toBe('100%');
		const unknown = elementLook(element({ target }), layout, show(), ctx({ now: null }));
		expect(unknown.fill).toBe(DEFAULT_IDLE_COLOR);
		expect(unknown.figure).toBeNull();
	});

	it('lights a running timeline and counts it down', () => {
		const t = timeline({ running: true, anchor_ms: 90_000, position_at_anchor_ms: 0 });
		const look = elementLook(element({ target: { kind: 'Timeline', timeline_id: 'scare' } }), layout, show({ timelines: [t] }), ctx());
		expect(look.fill).toBe(DEFAULT_ACTIVE_COLOR);
		expect(look.figure).toBe('−0:50');
		expect(look.fraction).toBeCloseTo(10 / 60);
		expect(look.animating).toBe(true);
	});

	it('says a target is gone rather than going blank', () => {
		const look = elementLook(element({ target: { kind: 'Variable', variable_id: 'gone' } }), layout, show(), ctx());
		expect(look.missing).toBe(true);
		expect(look.label).toBe('missing');
	});

	it('draws a shape, a line and words in their own color', () => {
		expect(elementLook(element({ color: '#112233' }), layout, show(), ctx()).fill).toBe('#112233');
		const line = elementLook(element({ target: { kind: 'Line', points: [[0, 0], [5, 5]] }, color: '#abcdef' }), layout, show(), ctx());
		expect(line.stroke).toBe('#abcdef');
		expect(line.fill).toBeNull();
		const text = elementLook(element({ target: { kind: 'Text' }, label: 'Exit', color: '#ff0000' }), layout, show(), ctx());
		expect(text).toMatchObject({ fill: null, ink: '#ff0000', label: 'Exit' });
	});

	it('draws no label when told not to', () => {
		expect(elementLook(element({ label: 'Room', show_label: false }), layout, show(), ctx()).label).toBe('');
	});

	it('names an element after what it points at, unless somebody named it', () => {
		const target = { kind: 'Parameter' as const, fixture_id: 'pir1', parameter: 'Temperature' as const };
		expect(elementLabel(element({ target }), show())).toBe('PIR 1 Temperature');
		expect(elementLabel(element({ target, label: 'Crypt' }), show())).toBe('Crypt');
	});

	it('asks the evaluator only for the parameters it drives', () => {
		const keys = drivenKeys(
			{
				elements: [
					element({ target: { kind: 'Parameter', fixture_id: 'pir1', parameter: 'Intensity' } }),
					element({ target: { kind: 'Parameter', fixture_id: 'pir1', parameter: { Contact: 0 } } })
				]
			},
			show()
		);
		expect(keys).toEqual(['pir1/Intensity']);
	});

	it('prints a reading in the units a sensor reports in', () => {
		expect(sensedFigure({ type: 'Float', value: 61.25 }, 'Humidity')).toBe('61.3%');
		expect(sensedFigure({ type: 'Int', value: 400 }, 'AirQuality')).toBe('400');
	});
});

describe('pressing an element', () => {
	it('goes the surface tile way for a variable and a timeline', () => {
		expect(pressOf(element({ target: { kind: 'Variable', variable_id: 'v' }, press: { kind: 'Variable', op: 'Toggle' } }))).toEqual({
			kind: 'tile',
			target: { kind: 'VariableSetter', variable_id: 'v', op: 'Toggle' }
		});
		expect(pressOf(element({ target: { kind: 'Timeline', timeline_id: 't' }, press: { kind: 'Timeline', from_start: true } }))).toEqual({
			kind: 'tile',
			target: { kind: 'Timeline', timeline_id: 't', from_start: true }
		});
	});

	it('selects a parameter element\'s fixture', () => {
		expect(pressOf(element({ target: { kind: 'Parameter', fixture_id: 'f', parameter: 'Intensity' }, press: { kind: 'Select' } }))).toEqual({
			kind: 'select',
			fixtureId: 'f'
		});
	});

	it('presses a flow button by being one', () => {
		expect(pressOf(element({ target: { kind: 'FlowButton', node_id: 'b' } }))).toEqual({
			kind: 'tile',
			target: { kind: 'FlowButton', node_id: 'b' }
		});
	});

	it('presses nothing on the shared page unless the layout lets it, and never selects there', () => {
		const button = element({ target: { kind: 'FlowButton', node_id: 'b' } });
		const select = element({ target: { kind: 'Parameter', fixture_id: 'f', parameter: 'Intensity' }, press: { kind: 'Select' } });
		expect(sharedPressOf(button, { operate_key: null }, null)).toBeNull();
		expect(sharedPressOf(button, { operate_key: null }, '')).toBeNull();
		expect(sharedPressOf(button, { operate_key: 'k1' }, null)).toBeNull();
		expect(sharedPressOf(button, { operate_key: 'k1' }, 'k0')).toBeNull();
		expect(sharedPressOf(button, { operate_key: 'k1' }, 'k1')).toEqual(pressOf(button));
		expect(sharedPressOf(select, { operate_key: 'k1' }, 'k1')).toBeNull();
	});

	it('finds the frontmost element under a finger, a dot made finger-wide by the slop', () => {
		const back = element({ id: 'back', x: 0, y: 0, w: 100, h: 100 });
		const dot = element({ id: 'dot', x: 50, y: 50, w: 2, h: 2 });
		const all = () => true;
		expect(elementAt([back, dot], { x: 51, y: 51 }, all)?.id).toBe('dot');
		expect(elementAt([back, dot], { x: 55, y: 55 }, all)?.id).toBe('back');
		expect(elementAt([back, dot], { x: 55, y: 55 }, (e) => e.id === 'dot', 4)?.id).toBe('dot');
		expect(elementAt([back, dot], { x: 200, y: 200 }, all)).toBeNull();
	});

	it('does nothing for a press its target cannot take', () => {
		expect(pressOf(element({ target: { kind: 'Shape' }, press: { kind: 'Select' } }))).toBeNull();
		expect(pressOf(element({ target: { kind: 'Variable', variable_id: 'v' } }))).toBeNull();
	});
});

describe('the canvas on a screen', () => {
	const canvas = { width: 200, height: 100 };

	it('fits, keeping the aspect, centered', () => {
		expect(fitView(canvas, { width: 400, height: 400 })).toEqual({ scale: 2, x: 0, y: 100 });
		expect(fitView(canvas, { width: 400, height: 100 })).toEqual({ scale: 1, x: 100, y: 0 });
	});

	it('zooms about the fingers, and no further out than fitting', () => {
		const screen = { width: 400, height: 200 };
		const fitted = fitView(canvas, screen);
		const zoomed = zoomAt(fitted, 2, { x: 200, y: 100 }, canvas, screen);
		expect(zoomed.scale).toBe(4);
		// The point under the fingers is still under them.
		expect(toCanvas(zoomed, 200, 100)).toEqual(toCanvas(fitted, 200, 100));
		expect(zoomAt(zoomed, 0.01, { x: 0, y: 0 }, canvas, screen)).toEqual(fitted);
		expect(zoomAt(fitted, 100, { x: 0, y: 0 }, canvas, screen).scale).toBe(16);
	});

	it('cannot be dragged off the screen', () => {
		const screen = { width: 400, height: 200 };
		expect(clampView({ scale: 4, x: 100, y: -1000 }, canvas, screen)).toEqual({ scale: 4, x: 0, y: -200 });
		// Narrower than the screen on an axis: centered on it.
		expect(clampView({ scale: 1, x: 7, y: 9 }, canvas, screen)).toEqual({ scale: 1, x: 100, y: 50 });
	});
});

describe('dragging an element', () => {
	const canvas = { width: 100, height: 50 };
	const from = { x: 10, y: 10, w: 20, h: 10 };

	it('moves on the grid and stays on the canvas', () => {
		expect(draggedRect(from, 'move', 3.2, 6.9, canvas, 5)).toEqual({ x: 15, y: 15, w: 20, h: 10 });
		expect(draggedRect(from, 'move', 500, -500, canvas, 5)).toEqual({ x: 80, y: 0, w: 20, h: 10 });
	});

	it('resizes by the edge it holds, snapped', () => {
		expect(draggedRect(from, 'se', 6, 4, canvas, 5)).toEqual({ x: 10, y: 10, w: 25, h: 15 });
		expect(draggedRect(from, 'w', -4, 0, canvas, 5)).toEqual({ x: 5, y: 10, w: 25, h: 10 });
	});

	it('stops a cell short rather than turning inside out', () => {
		expect(draggedRect(from, 'e', -100, 0, canvas, 5)).toEqual({ x: 10, y: 10, w: 5, h: 10 });
		expect(draggedRect(from, 'n', 0, 100, canvas, 5)).toEqual({ x: 10, y: 15, w: 20, h: 5 });
	});

	it('takes a line\'s points with it', () => {
		const line = element({ target: { kind: 'Line', points: [[10, 10], [30, 20]] } });
		expect(boxOf(line)).toEqual({ x: 10, y: 10, w: 20, h: 10 });
		const moved = placed(line, { x: 20, y: 0, w: 40, h: 10 });
		expect(moved.target).toEqual({ kind: 'Line', points: [[20, 0], [60, 10]] });
		expect(boxOf(moved)).toEqual({ x: 20, y: 0, w: 40, h: 10 });
	});

	it('keeps a straight line straight when its box is resized across it', () => {
		const line = element({ target: { kind: 'Line', points: [[10, 10], [30, 10]] } });
		expect(placed(line, { x: 10, y: 20, w: 20, h: 5 }).target).toEqual({ kind: 'Line', points: [[10, 20], [30, 20]] });
	});

	it('duplicates a cell down and right with an id of its own', () => {
		const copy = duplicated(element({ id: 'a' }), canvas, 5, 'b');
		expect(copy).toMatchObject({ id: 'b', x: 15, y: 15 });
	});

	it('restacks back to front', () => {
		const list = [element({ id: 'a' }), element({ id: 'b' }), element({ id: 'c' })];
		expect(toFront(list, 'a').map((e) => e.id)).toEqual(['b', 'c', 'a']);
		expect(toBack(list, 'c').map((e) => e.id)).toEqual(['c', 'a', 'b']);
	});
});

describe('adding elements', () => {
	it('places one element per reported parameter, never per driven one', () => {
		let n = 0;
		const placedOnes = sensorElements(show().fixtures, show().types, { x: 0, y: 0 }, { width: 200, height: 120 }, 5, () => `e${n++}`);
		expect(placedOnes.map((e) => e.target)).toEqual([
			{ kind: 'Parameter', fixture_id: 'pir1', parameter: { Contact: 0 } },
			{ kind: 'Parameter', fixture_id: 'pir1', parameter: 'Temperature' }
		]);
		// A contact is a dot; a number a box wide enough for its figure.
		expect(placedOnes[0].shape).toBe('Ellipse');
		expect(placedOnes[1].w).toBe(placedOnes[0].w * 2);
		expect(placedOnes[1].x).toBeGreaterThan(placedOnes[0].x);
	});

	it('places only the sensors picked', () => {
		const out = sensorElements(show().fixtures, show().types, { x: 0, y: 0 }, { width: 200, height: 120 }, 5, undefined, (_, key) => key === 'Temperature');
		expect(out.map((e) => e.target)).toEqual([{ kind: 'Parameter', fixture_id: 'pir1', parameter: 'Temperature' }]);
		expect(out[0].x).toBe(0);
	});

	it('names, draws and starts a flow button from the show\'s flows', () => {
		const withButton = show({
			flows: [{ id: 'f', name: 'Reset', enabled: true } as never],
			flowNodes: [{ id: 'b', flow_id: 'f', kind: 'Button', name: '' } as never]
		});
		const made = newElement('FlowButton', withButton, { x: 50, y: 50 }, { width: 200, height: 120 }, 5, null, 'n');
		expect(made?.target).toEqual({ kind: 'FlowButton', node_id: 'b' });
		expect(elementLabel(made!, withButton)).toBe('Reset');
		expect(elementLook(made!, layout, withButton, ctx()).missing).toBe(false);
		expect(elementLook(made!, layout, show(), ctx()).missing).toBe(true);
		expect(newElement('FlowButton', show(), { x: 0, y: 0 }, { width: 200, height: 120 }, 5)).toBeNull();
	});

	it('wraps a row at the canvas edge', () => {
		const many = Array.from({ length: 12 }, (_, i) => aFixture({ id: `p${i}`, fixture_type_id: 'pir' }));
		const out = sensorElements(many, [pirType], { x: 0, y: 0 }, { width: 200, height: 120 }, 5);
		expect(out.every((e) => e.x + e.w <= 200 && e.y + e.h <= 120)).toBe(true);
		expect(new Set(out.map((e) => e.y)).size).toBeGreaterThan(1);
	});

	it('reads every cell of a per-cell input', () => {
		const strip = aFixtureType({
			cells: 3,
			parameters: [aParameter({ kind: 'Temperature', direction: 'Input', per_cell: true, default_value: { type: 'Float', value: 0 } })]
		});
		expect(inputsOf(strip).map((i) => i.kind)).toEqual([1, 2, 3].map((cell) => ({ Cell: { cell, of: 'Temperature' } })));
	});

	it('starts a parameter element on what the fixture reports', () => {
		const made = newElement('Parameter', show(), { x: 50, y: 50 }, { width: 200, height: 120 }, 5, 'pir1', 'n');
		expect(made?.target).toEqual({ kind: 'Parameter', fixture_id: 'pir1', parameter: { Contact: 0 } });
	});

	it('refuses a kind the show has nothing of', () => {
		expect(newElement('Variable', show(), { x: 0, y: 0 }, { width: 200, height: 120 }, 5)).toBeNull();
	});

	it('makes a line whose box is its points', () => {
		const line = newElement('Line', show(), { x: 100, y: 60 }, { width: 200, height: 120 }, 5, null, 'l');
		expect(line && boxOf(line)).toEqual(line && { x: line.x, y: line.y, w: line.w, h: line.h });
	});
});

describe('the link', () => {
	it('opens /layout/<id>, carrying a second station\'s port', () => {
		expect(layoutUrl({ origin: 'http://10.0.0.2:7700', search: '?port=7701&x=1' }, 'a b')).toBe('http://10.0.0.2:7700/layout/a%20b?port=7701');
		expect(layoutUrl({ origin: 'http://h', search: '?port=7701' }, 'a', 'k1')).toBe('http://h/layout/a?port=7701&key=k1');
		expect(layoutUrl({ origin: 'http://h', search: '?key=k1' }, 'a')).toBe('http://h/layout/a');
		expect(isLayoutPath('/layout/abc')).toBe(true);
		expect(isLayoutPath('/layout')).toBe(true);
		expect(isLayoutPath('/layouts')).toBe(false);
	});
});

describe('several elements at once', () => {
	const canvas = { width: 200, height: 120 };
	const a = element({ id: 'a', x: 10, y: 10, w: 10, h: 10 });
	const b = element({ id: 'b', x: 30, y: 10, w: 10, h: 10 });
	const wall = element({ id: 'wall', target: { kind: 'Line', points: [[0, 60], [200, 60]] }, x: 0, y: 60, w: 200, h: 0 });

	it('a lasso catches what it touches, a wall included, whichever way it was drawn', () => {
		expect(inLasso([a, b, wall], rectBetween({ x: 25, y: 70 }, { x: 15, y: 15 }))).toEqual(['a', 'wall']);
	});

	it('a group moves as one: snapped together, and stopped together at an edge', () => {
		const from = new Map([
			['a', boxOf(a)],
			['b', boxOf(b)]
		]);
		const moved = movedGroup([a, b, wall], from, 12, 3, canvas, 5);
		expect(moved.map((e) => [e.x, e.y])).toEqual([[20, 15], [40, 15], [0, 60]]);
		const pushed = movedGroup([a, b], from, -50, 0, canvas, 5);
		// The spacing survives the edge.
		expect(pushed.map((e) => e.x)).toEqual([0, 20]);
	});

	it('a line moved with a group takes its points', () => {
		const moved = movedGroup([wall], new Map([['wall', boxOf(wall)]]), 0, 10, canvas, 5);
		expect(moved[0].target).toEqual({ kind: 'Line', points: [[0, 70], [200, 70]] });
	});

	it('a paste is new, in formation, centered where it is put, and steps off what is already there', () => {
		const clip = copyElements([a, b, wall], new Set(['b', 'a']));
		// Drawing order, not selection order.
		expect(clip.elements.map((e) => e.id)).toEqual(['a', 'b']);
		let n = 0;
		const ids = () => `new${n++}`;
		const first = pastedElements(clip, { x: 100, y: 60 }, [a, b], canvas, 5, ids);
		expect(first.map((e) => [e.id, e.x, e.y])).toEqual([
			['new0', 85, 55],
			['new1', 105, 55]
		]);
		const again = pastedElements(clip, { x: 100, y: 60 }, [a, b, ...first], canvas, 5, ids);
		expect(again.map((e) => [e.x, e.y])).toEqual([
			[90, 60],
			[110, 60]
		]);
	});
});
