import { describe, it, expect } from 'vitest';
import type { FlowEdge, FlowNode, FlowNodeKind } from './generated/index.js';
import {
	actionOf,
	canConnect,
	conditionOf,
	delayMsOf,
	sourceOf,
	conditionFrom,
	conditionLabel,
	graphOf,
	inputPorts,
	nextNodePosition,
	nodeCategory,
	nodeTag,
	outputPorts,
	thresholdOf
} from './flow.js';

const node = (id: string, kind: FlowNodeKind, flow = 'f1'): FlowNode => ({
	id,
	flow_id: flow,
	kind,
	x: 0,
	y: 0,
	active: false,
	last_fired_at: null
});

const SOURCE: FlowNodeKind = {
	Source: { Parameter: { fixture_id: 'fixture', parameter: { Contact: 0 } } }
};
const ACTION: FlowNodeKind = { Action: { GoNext: { sequence_id: 'seq' } } };
const CONDITION: FlowNodeKind = { Condition: 'RisingEdge' };

describe('reading a kind', () => {
	it('pulls the payload out of the variant that has one', () => {
		expect(sourceOf(SOURCE)).toEqual({ Parameter: { fixture_id: 'fixture', parameter: { Contact: 0 } } });
		expect(conditionOf(CONDITION)).toBe('RisingEdge');
		expect(delayMsOf({ Delay: { ms: 750 } })).toBe(750);
		expect(actionOf(ACTION)).toEqual({ GoNext: { sequence_id: 'seq' } });
	});

	it('answers null for a variant that is not the one asked about', () => {
		expect(sourceOf(ACTION)).toBeNull();
		expect(conditionOf(SOURCE)).toBeNull();
		expect(actionOf(CONDITION)).toBeNull();
	});

	it('answers null for the variants that carry nothing', () => {
		// `'Delay' in kind` throws on a bare string, which is the whole reason
		// these exist rather than an inline check at each call site.
		expect(delayMsOf('And')).toBeNull();
		expect(sourceOf('Button')).toBeNull();
	});
});

describe('ports', () => {
	it('names the tag of a kind whether or not it carries a payload', () => {
		expect(nodeTag('And')).toBe('And');
		expect(nodeTag(SOURCE)).toBe('Source');
		expect(nodeTag({ Delay: { ms: 500 } })).toBe('Delay');
	});

	it('gives a source no inputs and one level out', () => {
		expect(inputPorts(SOURCE)).toEqual([]);
		expect(outputPorts(SOURCE)).toEqual(['Level']);
	});

	it('gives an action one pulse in and nothing out', () => {
		expect(inputPorts(ACTION)).toEqual(['Pulse']);
		expect(outputPorts(ACTION)).toEqual([]);
	});

	it('turns a level into a pulse at a condition', () => {
		expect(inputPorts(CONDITION)).toEqual(['Level']);
		expect(outputPorts(CONDITION)).toEqual(['Pulse']);
	});

	it('gives and/or two levels in and one out', () => {
		expect(inputPorts('And')).toEqual(['Level', 'Level']);
		expect(outputPorts('Or')).toEqual(['Level']);
	});
});

describe('canConnect', () => {
	const nodes = [node('s', SOURCE), node('c', CONDITION), node('a', ACTION), node('and', 'And')];

	it('joins a level to a level', () => {
		expect(canConnect(nodes, [], { node: 's', port: 0 }, { node: 'c', port: 0 })).toBe(true);
	});

	it('refuses a level where a pulse belongs', () => {
		// The mistake the whole port-kind split exists to prevent: a source wired
		// straight to a cue would fire on every reading rather than on a change.
		expect(canConnect(nodes, [], { node: 's', port: 0 }, { node: 'a', port: 0 })).toBe(false);
	});

	it('refuses a node wired to itself', () => {
		expect(canConnect(nodes, [], { node: 'and', port: 0 }, { node: 'and', port: 0 })).toBe(false);
	});

	it('refuses a second edge into an input that is already taken', () => {
		const edges: FlowEdge[] = [
			{ id: 'e', flow_id: 'f1', from_node: 's', from_port: 0, to_node: 'c', to_port: 0 }
		];
		expect(canConnect(nodes, edges, { node: 'and', port: 0 }, { node: 'c', port: 0 })).toBe(false);
	});

	it('still allows the other input of an and', () => {
		const edges: FlowEdge[] = [
			{ id: 'e', flow_id: 'f1', from_node: 's', from_port: 0, to_node: 'and', to_port: 0 }
		];
		expect(canConnect(nodes, edges, { node: 's', port: 0 }, { node: 'and', port: 1 })).toBe(true);
	});

	it('refuses a handle a node does not have', () => {
		expect(canConnect(nodes, [], { node: 's', port: 0 }, { node: 'c', port: 1 })).toBe(false);
	});
});

describe('conditions', () => {
	it('reads a bare condition as prose', () => {
		expect(conditionLabel('RisingEdge')).toBe('closes');
		expect(conditionLabel('FallingEdge')).toBe('opens');
	});

	it('reads a threshold with its number', () => {
		expect(conditionLabel({ Above: 21.5 })).toBe('rises above 21.5');
		expect(conditionLabel({ Below: 4 })).toBe('falls below 4');
	});

	it('round-trips a threshold through its tag', () => {
		expect(conditionFrom('Above', 21.5)).toEqual({ Above: 21.5 });
		expect(thresholdOf(conditionFrom('Below', 4))).toBe(4);
	});

	it('drops the threshold when the condition stops needing one', () => {
		expect(conditionFrom('AnyChange', 21.5)).toBe('AnyChange');
		expect(thresholdOf('AnyChange')).toBe(0);
	});
});

describe('categories', () => {
	it('groups nodes by what they are for', () => {
		expect(nodeCategory(SOURCE)).toBe('source');
		expect(nodeCategory('Button')).toBe('source');
		expect(nodeCategory('And')).toBe('logic');
		expect(nodeCategory(CONDITION)).toBe('logic');
		expect(nodeCategory({ Delay: { ms: 1 } })).toBe('timing');
		expect(nodeCategory(ACTION)).toBe('action');
	});
});

describe('layout', () => {
	it('puts the first node somewhere visible', () => {
		expect(nextNodePosition([])).toEqual({ x: 40, y: 40 });
	});

	it('puts the next one to the right of the rightmost', () => {
		const nodes = [{ ...node('a', SOURCE), x: 40 }, { ...node('b', CONDITION), x: 260, y: 90 }];
		expect(nextNodePosition(nodes)).toEqual({ x: 500, y: 90 });
	});
});

describe('graphOf', () => {
	const nodes = [node('a', SOURCE, 'f1'), node('b', SOURCE, 'f2')];
	const edges: FlowEdge[] = [
		{ id: 'e1', flow_id: 'f1', from_node: 'a', from_port: 0, to_node: 'a', to_port: 0 },
		{ id: 'e2', flow_id: 'f2', from_node: 'b', from_port: 0, to_node: 'b', to_port: 0 }
	];

	it('keeps one flow to itself', () => {
		const graph = graphOf('f1', nodes, edges);
		expect(graph.nodes.map((n) => n.id)).toEqual(['a']);
		expect(graph.edges.map((e) => e.id)).toEqual(['e1']);
	});

	it('shows nothing when no flow is picked', () => {
		expect(graphOf(null, nodes, edges)).toEqual({ nodes: [], edges: [] });
	});
});
