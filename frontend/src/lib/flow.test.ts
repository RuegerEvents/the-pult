import { describe, it, expect } from 'vitest';
import type { FlowEdge, FlowNode, FlowNodeKind, VariableOp } from './generated/index.js';
import {
	actionOf,
	canConnect,
	copyNodes,
	namesItsInputs,
	pasteClip,
	placeNear,
	portHelp,
	NUDGE,
	conditionOf,
	delayMsOf,
	sourceOf,
	conditionFrom,
	conditionLabel,
	graphOf,
	currentValue,
	inputPorts,
	nextNodePosition,
	nodeCategory,
	nodeTag,
	outputPorts,
	variableLabel,
	variableOpFrom,
	variableOpLabel,
	variableOpParts,
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

	it('lets a gate take a pulse and the level that decides it', () => {
		// Mirrors `FlowNodeKind::Gate`: the pulse on port 0, the level on port 1.
		expect(inputPorts('Gate')).toEqual(['Pulse', 'Level']);
		expect(inputPorts({ Delay: { ms: 1 } })).toEqual(['Pulse', 'Pulse']);
		expect(outputPorts('Gate')).toEqual(['Pulse']);
	});
});

describe('a gate', () => {
	const nodes = [node('c', CONDITION), node('s', SOURCE), node('g', 'Gate'), node('a', ACTION)];

	it('takes the pulse on its first port and the level on its second', () => {
		expect(canConnect(nodes, [], { node: 'c', port: 0 }, { node: 'g', port: 0 })).toBe(true);
		expect(canConnect(nodes, [], { node: 's', port: 0 }, { node: 'g', port: 1 })).toBe(true);
	});

	it('refuses them the other way round', () => {
		expect(canConnect(nodes, [], { node: 's', port: 0 }, { node: 'g', port: 0 })).toBe(false);
		expect(canConnect(nodes, [], { node: 'c', port: 0 }, { node: 'g', port: 1 })).toBe(false);
	});

	it('passes a pulse on to an action', () => {
		expect(canConnect(nodes, [], { node: 'g', port: 0 }, { node: 'a', port: 0 })).toBe(true);
	});
});

describe('variables', () => {
	const flag = { kind: 'Flag' as const, value: null, initial: 1 };
	const count = { kind: 'Number' as const, value: 3, initial: 0 };

	it('reads an unwritten variable as its initial value', () => {
		expect(currentValue(flag)).toBe(1);
		expect(variableLabel(flag)).toBe('on');
		expect(variableLabel(count)).toBe('3');
	});

	it('round-trips every op through the editor', () => {
		const ops: VariableOp[] = [{ Set: 4 }, { Add: 1 }, { Add: -1 }, 'Toggle', 'Reset'];
		for (const op of ops) {
			const { tag, amount } = variableOpParts(op);
			expect(variableOpFrom(tag, amount)).toEqual(op);
		}
	});

	it('says what an op does', () => {
		expect(variableOpLabel({ Add: -1 })).toBe('subtract 1');
		expect(variableOpLabel({ Set: 0 })).toBe('set to 0');
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

	it('takes a second edge into an input as an or, and refuses the same edge twice', () => {
		const edges: FlowEdge[] = [
			{ id: 'e', flow_id: 'f1', from_node: 's', from_port: 0, to_node: 'c', to_port: 0 }
		];
		expect(canConnect(nodes, edges, { node: 'and', port: 0 }, { node: 'c', port: 0 })).toBe(true);
		expect(canConnect(nodes, edges, { node: 's', port: 0 }, { node: 'c', port: 0 })).toBe(false);
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

describe('placeNear', () => {
	const view = { x: 0, y: 0, w: 1000, h: 600 };

	it('takes the spot asked for when it is free', () => {
		expect(placeNear({ x: 100, y: 100 }, [], view)).toEqual({ x: 100, y: 100 });
	});

	it('steps down and right past nodes already there', () => {
		const nodes = [{ x: 100, y: 100 }, { x: 100 + NUDGE, y: 100 + NUDGE }];
		expect(placeNear({ x: 100, y: 100 }, nodes, view)).toEqual({ x: 100 + 2 * NUDGE, y: 100 + 2 * NUDGE });
	});

	it('never steps out of the view', () => {
		// Asked for the bottom-right corner, which is taken: the next step would leave the
		// view, so it starts again along the top.
		const at = { x: 1000 - 184, y: 600 - 90 };
		const p = placeNear(at, [at], view);
		expect(p.x).toBeGreaterThanOrEqual(0);
		expect(p.y).toBeGreaterThanOrEqual(0);
		expect(p.x + 184).toBeLessThanOrEqual(1000);
		expect(p.y + 90).toBeLessThanOrEqual(600);
		expect(p).not.toEqual(at);
	});
});

describe('copy and paste', () => {
	const a = { ...node('a', SOURCE), x: 100, y: 50, active: true };
	const b = { ...node('b', CONDITION), x: 300, y: 80 };
	const c = node('c', ACTION);
	const edges: FlowEdge[] = [
		{ id: 'ab', flow_id: 'f1', from_node: 'a', from_port: 0, to_node: 'b', to_port: 0 },
		{ id: 'bc', flow_id: 'f1', from_node: 'b', from_port: 0, to_node: 'c', to_port: 0 }
	];

	it('takes the wires between the copied nodes and none that leave them', () => {
		const clip = copyNodes([a, b, c], edges, new Set(['a', 'b']));
		expect(clip.nodes.map((n) => n.id)).toEqual(['a', 'b']);
		expect(clip.edges.map((e) => e.id)).toEqual(['ab']);
	});

	it('pastes new ids into another flow, wired the same, moved together and at rest', () => {
		let n = 0;
		const clip = copyNodes([a, b, c], edges, new Set(['a', 'b']));
		const pasted = pasteClip(clip, 'f2', { x: 0, y: 0 }, () => `new${++n}`);
		const [pa, pb] = pasted.nodes;
		expect(pa.id).not.toBe('a');
		expect(pa.flow_id).toBe('f2');
		expect([pa.x, pa.y, pb.x, pb.y]).toEqual([0, 0, 200, 30]);
		expect(pa.active).toBe(false);
		expect(pasted.edges).toHaveLength(1);
		expect(pasted.edges[0]).toMatchObject({ flow_id: 'f2', from_node: pa.id, to_node: pb.id });
		expect(pasted.edges[0].id).not.toBe('ab');
	});
});

describe('portHelp', () => {
	it('tells a wait\'s start from its reset', () => {
		const wait: FlowNodeKind = { Delay: { ms: 1000 } };
		expect(portHelp(wait, 'in', 0).label).toBe('start');
		expect(portHelp(wait, 'in', 1).label).toBe('reset');
		expect(portHelp(wait, 'in', 1).help).toContain('nothing comes out');
		expect(namesItsInputs(wait)).toBe(true);
		expect(namesItsInputs('And')).toBe(false);
	});

	it('says which shape the handle is', () => {
		expect(portHelp('Gate', 'in', 0).help).toContain('pulse');
		expect(portHelp('Gate', 'in', 1).help).toContain('level');
	});

	it('has a sentence for every port of every kind', () => {
		const kinds: FlowNodeKind[] = [SOURCE, CONDITION, ACTION, 'Button', 'And', 'Or', 'Not', 'Gate', { Delay: { ms: 1 } }];
		for (const kind of kinds) {
			for (const [side, ports] of [['in', inputPorts(kind)], ['out', outputPorts(kind)]] as const) {
				ports.forEach((_, i) => expect(portHelp(kind, side, i).help.length).toBeGreaterThan(30));
			}
		}
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
