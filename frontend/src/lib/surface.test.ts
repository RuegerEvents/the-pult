import { describe, it, expect } from 'vitest';
import type { AudioClip, FlowNode, SurfaceTile, Timeline, Variable } from './generated/index.js';
import {
	flowButtons,
	inkOn,
	isAction,
	isSurfacePath,
	moveTile,
	newTile,
	OFF_COLOR,
	opWords,
	parameterReading,
	phoneHosts,
	phoneLocation,
	patchTile,
	removeTile,
	stopColor,
	surfaceIdOf,
	surfaceUrl,
	tileLabel,
	timelineLength,
	timelinePosition,
	timelineProgress,
	variableReading,
	type SurfaceShow
} from './surface.js';
import { aFixture, aFixtureType, aParameter } from './test-fixtures.js';

const variable = (over: Partial<Variable> = {}): Variable => ({
	id: 'area2',
	name: 'Area 2 inside',
	kind: 'Number',
	initial: 0,
	min: 0,
	max: null,
	value: null,
	...over
});

const button = (id: string, flow: string): FlowNode => ({
	id,
	flow_id: flow,
	kind: 'Button',
	x: 0,
	y: 0,
	active: false,
	last_fired_at: null
});

const timeline = (over: Partial<Timeline> = {}): Timeline =>
	({
		id: 'ambient',
		name: 'Ambience',
		clips: [],
		detected: null,
		source: 'Internal',
		grid: [],
		markers: [],
		events: [],
		tracks: [],
		speed_master: null,
		node_id: null,
		end: null,
		running: false,
		anchor_ms: 0,
		position_at_anchor_ms: 0,
		rate: 1,
		recording: null,
		...over
	}) as Timeline;

const empty: SurfaceShow = {
	variables: [],
	timelines: [],
	sequences: [],
	flows: [],
	flowNodes: [],
	fixtures: [],
	types: []
};

const tile = (over: Partial<SurfaceTile>): SurfaceTile => ({
	id: 't',
	label: '',
	color: null,
	size: 'Half',
	target: { kind: 'Heading' },
	...over
});

describe('a readout color', () => {
	const occupied = [
		{ at: 0, color: '#22c55e' },
		{ at: 1, color: '#ef4444' }
	];

	it('takes the highest stop at or below the value', () => {
		expect(stopColor(occupied, 0)).toBe('#22c55e');
		expect(stopColor(occupied, 1)).toBe('#ef4444');
		expect(stopColor(occupied, 3)).toBe('#ef4444');
	});

	it('does not care what order the stops were written in', () => {
		expect(stopColor([...occupied].reverse(), 2)).toBe('#ef4444');
	});

	it('reads below every stop as the lowest one, not as nothing', () => {
		expect(stopColor(occupied, -1)).toBe('#22c55e');
	});

	it('is on or off when nobody gave it stops', () => {
		expect(stopColor([], 1, '#123456')).toBe('#123456');
		expect(stopColor([], 0, '#123456')).toBe(OFF_COLOR);
	});

	it('picks ink that reads on it', () => {
		expect(inkOn('#ffffff')).toBe('#111');
		expect(inkOn('#000000')).toBe('#fff');
		expect(inkOn('not a color')).toBe('#fff');
	});
});

describe('what a tile says', () => {
	const show: SurfaceShow = {
		...empty,
		variables: [variable()],
		flows: [
			{ id: 'f1', name: 'Reset maze', enabled: true },
			{ id: 'f2', name: 'Scares', enabled: true }
		],
		flowNodes: [button('b1', 'f1'), button('b2', 'f2'), button('b3', 'f2')]
	};

	it('names a flow button after its flow, numbered when a flow has two', () => {
		expect(flowButtons(show)).toEqual([
			{ id: 'b1', name: 'Reset maze' },
			{ id: 'b2', name: 'Scares · 1' },
			{ id: 'b3', name: 'Scares · 2' }
		]);
	});

	it('uses its own label first and its target’s name otherwise', () => {
		const t = tile({ target: { kind: 'VariableSetter', variable_id: 'area2', op: 'Reset' } });
		expect(tileLabel(t, show)).toBe('Area 2 inside: reset');
		expect(tileLabel({ ...t, label: 'Clear area 2' }, show)).toBe('Clear area 2');
	});

	it('says a target is missing rather than going blank', () => {
		const t = tile({ target: { kind: 'VariableReadout', variable_id: 'gone', stops: [] } });
		expect(tileLabel(t, show)).toBe('missing');
	});

	it('presses a readout only when it says what pressing does', () => {
		const readout = { kind: 'VariableReadout' as const, variable_id: 'area2', stops: [] };
		expect(isAction(readout)).toBe(false);
		expect(isAction({ ...readout, press: null })).toBe(false);
		expect(isAction({ ...readout, press: 'Toggle' })).toBe(true);
	});

	it('writes an add as a sign', () => {
		expect(opWords({ Add: 1 })).toBe('+1');
		expect(opWords({ Add: -1 })).toBe('−1');
	});

	it('reads a variable with its color', () => {
		const target = {
			kind: 'VariableReadout' as const,
			variable_id: 'area2',
			stops: [
				{ at: 0, color: '#0f0' },
				{ at: 1, color: '#f00' }
			]
		};
		expect(variableReading(target, { variables: [variable({ value: 2 })] }, null)).toEqual({
			text: '2',
			color: '#f00'
		});
		// No value yet is the initial value, not nothing.
		expect(variableReading(target, { variables: [variable()] }, null)?.text).toBe('0');
	});
});

describe('a parameter readout', () => {
	const types = [
		aFixtureType({
			parameters: [
				aParameter({ kind: 'ColorRgb', default_value: { type: 'Color', value: { r: 0, g: 0, b: 0, overrides: {} } } }),
				aParameter({ kind: { Contact: 0 }, direction: 'Input', default_value: { type: 'Bool', value: false } })
			]
		})
	];

	it('reads an output from the evaluated rig', () => {
		const show = { fixtures: [aFixture({ name: 'Light' })], types };
		const red = { type: 'Color' as const, value: { r: 1, g: 0, b: 0, overrides: {} } };
		const reading = parameterReading(
			{ kind: 'Parameter', fixture_id: 'fixture', parameter: 'ColorRgb' },
			show,
			() => red
		);
		expect(reading?.color).toBe('#ff0000');
	});

	it('reads an input from what the device reported, which nothing evaluates', () => {
		const show = {
			fixtures: [aFixture({ sensed_values: { 'Contact:0': { type: 'Bool', value: true } } })],
			types
		};
		const reading = parameterReading(
			{ kind: 'Parameter', fixture_id: 'fixture', parameter: { Contact: 0 } },
			show,
			() => null
		);
		expect(reading?.text).toBe('on');
		expect(reading?.color).toBe('#22c55e');
	});

	it('draws a relay that is on in the tile’s own color, so the red lamp reads red', () => {
		const show = {
			fixtures: [aFixture({ sensed_values: { 'Contact:0': { type: 'Bool', value: true } } })],
			types
		};
		const target = { kind: 'Parameter' as const, fixture_id: 'fixture', parameter: { Contact: 0 } };
		expect(parameterReading(target, show, () => null, '#dc2626')?.color).toBe('#dc2626');
	});

	it('shows a gap before the page knows the clock', () => {
		const show = { fixtures: [aFixture()], types };
		const reading = parameterReading(
			{ kind: 'Parameter', fixture_id: 'fixture', parameter: 'ColorRgb' },
			show,
			() => null
		);
		expect(reading).toEqual({ text: '–', color: null });
	});
});

describe('where a timeline is', () => {
	it('stands still while stopped, clock or no clock', () => {
		expect(timelinePosition(timeline({ position_at_anchor_ms: 4000 }), null)).toBe(4000);
	});

	it('says nothing about a running one before the page knows the clock', () => {
		expect(timelinePosition(timeline({ running: true }), null)).toBeNull();
	});

	it('runs from its anchor, and wraps at a looping end', () => {
		const running = timeline({ running: true, anchor_ms: 1000, position_at_anchor_ms: 0 });
		expect(timelinePosition(running, 3500)).toBe(2500);
		const looping = { ...running, end: { at_ms: 2000, then: 'Loop' as const } };
		expect(timelinePosition(looping, 3500)).toBe(500);
		const stopping = { ...running, end: { at_ms: 2000, then: 'Stop' as const } };
		expect(timelinePosition(stopping, 3500)).toBe(2000);
	});
});

describe('how much of a timeline is left', () => {
	const clip = (over: Partial<AudioClip> = {}): AudioClip =>
		({ id: 'c', name: '', asset: 'a', peaks: 'p', start_ms: 1000, offset_ms: 0, length_ms: null, repeats: 1, ...over }) as AudioClip;
	const unknown = () => null;

	it('runs to its end where it stops at one', () => {
		expect(timelineLength(timeline({ end: { at_ms: 90_000, then: 'Stop' } }), unknown)).toBe(90_000);
	});

	it('is never done when it loops', () => {
		expect(timelineLength(timeline({ end: { at_ms: 90_000, then: 'Loop' } }), unknown)).toBeNull();
	});

	it('runs to its last event, marker or sound where it has no end', () => {
		const events = [{ at_ms: 5000 }] as Timeline['events'];
		expect(timelineLength(timeline({ events }), unknown)).toBe(5000);
		expect(timelineLength(timeline({ events, clips: [clip()] }), () => 30_000)).toBe(31_000);
		expect(timelineLength(timeline({ clips: [clip({ length_ms: 2000, repeats: 3 })] }), unknown)).toBe(7000);
	});

	it('says nothing until it knows how long a file playing to its end is', () => {
		expect(timelineLength(timeline({ clips: [clip()] }), unknown)).toBeNull();
		expect(timelineLength(timeline(), unknown)).toBeNull();
	});

	it('counts down to the end and stops at nothing left', () => {
		expect(timelineProgress(15_000, 60_000)).toEqual({ remainingMs: 45_000, fraction: 0.25 });
		expect(timelineProgress(70_000, 60_000)).toEqual({ remainingMs: 0, fraction: 1 });
		expect(timelineProgress(null, 60_000)).toBeNull();
		expect(timelineProgress(1000, null)).toBeNull();
	});
});

describe('editing the list', () => {
	const tiles = ['a', 'b', 'c'].map((id) => tile({ id }));

	it('moves a tile and leaves the rest in order', () => {
		expect(moveTile(tiles, 0, 2).map((t) => t.id)).toEqual(['b', 'c', 'a']);
		expect(moveTile(tiles, 2, 0).map((t) => t.id)).toEqual(['c', 'a', 'b']);
	});

	it('moves nothing past either end', () => {
		expect(moveTile(tiles, 0, -1)).toBe(tiles);
		expect(moveTile(tiles, 2, 3)).toBe(tiles);
	});

	it('patches and removes by id', () => {
		expect(patchTile(tiles, 'b', { label: 'B' })[1].label).toBe('B');
		expect(removeTile(tiles, 'b').map((t) => t.id)).toEqual(['a', 'c']);
	});

	it('makes a tile only when the show has something for it to point at', () => {
		expect(newTile('VariableReadout', empty)).toBeNull();
		expect(newTile('Heading', empty, 'h')).toEqual({
			id: 'h',
			label: 'Heading',
			color: null,
			size: 'Full',
			target: { kind: 'Heading' }
		});
		const flag = newTile('VariableSetter', { ...empty, variables: [variable({ kind: 'Flag' })] });
		expect(flag?.target).toEqual({ kind: 'VariableSetter', variable_id: 'area2', op: 'Toggle' });
	});
});

describe('the link', () => {
	it('is this station’s address, carrying ?port= over', () => {
		expect(surfaceUrl({ origin: 'http://10.0.0.5:7700', search: '' }, 'abc')).toBe(
			'http://10.0.0.5:7700/surface/abc'
		);
		expect(surfaceUrl({ origin: 'http://host:5173', search: '?port=7701&x=1' }, 'abc')).toBe(
			'http://host:5173/surface/abc?port=7701'
		);
	});

	it('reads the id back off the path', () => {
		expect(surfaceIdOf('/surface/abc')).toBe('abc');
		expect(surfaceIdOf('/surface/abc/')).toBe('abc');
		expect(surfaceIdOf('/surface')).toBeNull();
		expect(isSurfacePath('/surface')).toBe(true);
		expect(isSurfacePath('/surface/x')).toBe(true);
		expect(isSurfacePath('/surfaces')).toBe(false);
		expect(isSurfacePath('/')).toBe(false);
	});
});

describe('the link a phone is given', () => {
	it('swaps a loopback origin for one of the station’s own addresses', () => {
		const here = { origin: 'http://localhost:7700', hostname: 'localhost', search: '?port=7701' };
		expect(phoneLocation(here, '192.168.1.20', 7701)).toEqual({ origin: 'http://192.168.1.20:7701', search: '' });
	});

	it('keeps an origin the network can already reach', () => {
		const here = { origin: 'http://10.0.0.5:7700', hostname: '10.0.0.5', search: '' };
		expect(phoneLocation(here, '192.168.1.20', 7700)).toBe(here);
	});

	it('keeps loopback when the station knows no better', () => {
		const here = { origin: 'http://localhost:7700', hostname: 'localhost', search: '' };
		expect(phoneLocation(here, null, 7700)).toBe(here);
	});

	it('offers addresses a phone can dial, a tunnel last', () => {
		const nic = (name: string, addresses: string[], up = true, loopback = false) => ({ name, addresses, up, loopback });
		expect(
			phoneHosts([
				nic('lo0', ['127.0.0.1'], true, true),
				nic('utun3', ['10.8.0.2']),
				nic('en0', ['fe80::1', '169.254.3.3', '192.168.1.20']),
				nic('en5', ['2.0.0.10'], false)
			])
		).toEqual([
			{ name: 'en0', address: '192.168.1.20' },
			{ name: 'utun3', address: '10.8.0.2' }
		]);
	});
});
