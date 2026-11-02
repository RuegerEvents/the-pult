/**
 * Pure helpers for the flow editor.
 *
 * The port rules here are the same ones `FlowNodeKind::inputs`/`outputs` state in
 * `crates/pult-schema/src/types/flow.rs`, and they have to stay that way: the
 * backend refuses to evaluate a level arriving where a pulse belongs, and the
 * editor's job is to make sure such an edge is never drawn in the first place.
 */

import type {
	FlowEdge,
	FlowNode,
	FlowNodeKind,
	PortKind,
	TriggerAction,
	TriggerCondition,
	TriggerSource,
	VariableOp
} from './generated/index.js';
import { kindLabel } from './patch.js';

// ── Reading a kind ────────────────────────────────────────────────────────────
//
// `FlowNodeKind` is a union of bare strings and single-key objects, so `'Source' in
// kind` does not narrow it. These four are the only places that have to know that.

/** What a source watches, or `null` if this is not a source. */
export function sourceOf(kind: FlowNodeKind): TriggerSource | null {
	return typeof kind === 'object' && 'Source' in kind ? kind.Source : null;
}

export function conditionOf(kind: FlowNodeKind): TriggerCondition | null {
	return typeof kind === 'object' && 'Condition' in kind ? kind.Condition : null;
}

export function delayMsOf(kind: FlowNodeKind): number | null {
	return typeof kind === 'object' && 'Delay' in kind ? kind.Delay.ms : null;
}

export function actionOf(kind: FlowNodeKind): TriggerAction | null {
	return typeof kind === 'object' && 'Action' in kind ? kind.Action : null;
}

// ── Ports ─────────────────────────────────────────────────────────────────────

/** The tag of a `FlowNodeKind`, whether it carries a payload or not. */
export function nodeTag(kind: FlowNodeKind): string {
	return typeof kind === 'string' ? kind : Object.keys(kind)[0];
}

/** What a node takes, in handle order. Mirrors `FlowNodeKind::inputs`. */
export function inputPorts(kind: FlowNodeKind): PortKind[] {
	switch (nodeTag(kind)) {
		case 'Source':
		case 'Button':
			return [];
		case 'Condition':
		case 'Not':
			return ['Level'];
		case 'And':
		case 'Or':
			return ['Level', 'Level'];
		// The pulse to let through, and the level that decides whether it goes.
		case 'Gate':
			return ['Pulse', 'Level'];
		// Start, and reset: a pulse on the second stops it counting.
		case 'Delay':
			return ['Pulse', 'Pulse'];
		default:
			return ['Pulse'];
	}
}

/** What a node gives, in handle order. Mirrors `FlowNodeKind::outputs`. */
export function outputPorts(kind: FlowNodeKind): PortKind[] {
	switch (nodeTag(kind)) {
		case 'Source':
		case 'And':
		case 'Or':
		case 'Not':
			return ['Level'];
		case 'Button':
		case 'Condition':
		case 'Delay':
		case 'Gate':
			return ['Pulse'];
		default:
			return [];
	}
}

/**
 * Can these two handles be joined?
 *
 * A connection is valid when both ends carry the same kind of signal. Several
 * edges into one input are an *or*, as the station evaluates them (`upstream` in
 * `model/flows.rs`): a PIR and a button both firing the same scare is two wires into
 * one input. Only the very same wire twice is refused, since it would mean nothing.
 */
export function canConnect(
	nodes: FlowNode[],
	edges: FlowEdge[],
	from: { node: string; port: number },
	to: { node: string; port: number }
): boolean {
	if (from.node === to.node) return false;
	const source = nodes.find((n) => n.id === from.node);
	const target = nodes.find((n) => n.id === to.node);
	if (!source || !target) return false;

	const out = outputPorts(source.kind)[from.port];
	const into = inputPorts(target.kind)[to.port];
	if (!out || !into || out !== into) return false;

	return !edges.some(
		(e) => e.from_node === from.node && e.from_port === from.port && e.to_node === to.node && e.to_port === to.port
	);
}

// ── Variables ─────────────────────────────────────────────────────────────────

/** What a variable holds now: its value, or its initial value when it has none. */
export const currentValue = (variable: { value: number | null; initial: number }): number =>
	variable.value ?? variable.initial;

/** A variable's value as an operator reads it: on and off for a flag, the number otherwise. */
export function variableLabel(variable: {
	kind: 'Flag' | 'Number';
	value: number | null;
	initial: number;
}): string {
	const value = currentValue(variable);
	if (variable.kind === 'Flag') return value !== 0 ? 'on' : 'off';
	return Number.isInteger(value) ? String(value) : value.toFixed(2);
}

/**
 * What a flag can be told: on, off, or the other way. A flag is a number underneath,
 * but "set to 1" is the storage showing through, and add, subtract and reset say
 * nothing a flag needs.
 */
export type FlagOp = 'On' | 'Off' | 'Toggle';

/** The flag op an op is, where it is one; anything else reads as on. */
export function flagOpOf(op: VariableOp): FlagOp {
	if (op === 'Toggle') return 'Toggle';
	if (op !== 'Reset' && 'Set' in op) return op.Set === 0 ? 'Off' : 'On';
	return 'On';
}

export function flagOpFrom(tag: FlagOp): VariableOp {
	return tag === 'Toggle' ? 'Toggle' : { Set: tag === 'On' ? 1 : 0 };
}

/** What a `SetVariable` op says, in words — for a flag, on and off. */
export function variableOpLabel(op: VariableOp, kind: 'Flag' | 'Number' = 'Number'): string {
	if (kind === 'Flag') return { On: 'turn on', Off: 'turn off', Toggle: 'toggle' }[flagOpOf(op)];
	if (op === 'Toggle') return 'toggle';
	if (op === 'Reset') return 'reset';
	if ('Set' in op) return `set to ${op.Set}`;
	return op.Add >= 0 ? `add ${op.Add}` : `subtract ${-op.Add}`;
}

/** The op an editor's tag and number make. */
export function variableOpFrom(tag: string, amount: number): VariableOp {
	switch (tag) {
		case 'Set':
			return { Set: amount };
		case 'Add':
			return { Add: amount };
		case 'Subtract':
			return { Add: -Math.abs(amount) };
		case 'Toggle':
			return 'Toggle';
		default:
			return 'Reset';
	}
}

/** The editor's tag and number for an op, the inverse of `variableOpFrom`. */
export function variableOpParts(op: VariableOp): { tag: string; amount: number } {
	if (op === 'Toggle' || op === 'Reset') return { tag: op, amount: 1 };
	if ('Set' in op) return { tag: 'Set', amount: op.Set };
	return op.Add < 0 ? { tag: 'Subtract', amount: -op.Add } : { tag: 'Add', amount: op.Add };
}

// ── Labels ────────────────────────────────────────────────────────────────────

/** Which family a node belongs to, for coloring it. */
export function nodeCategory(kind: FlowNodeKind): 'source' | 'logic' | 'timing' | 'action' {
	switch (nodeTag(kind)) {
		case 'Source':
		case 'Button':
			return 'source';
		case 'Condition':
		case 'And':
		case 'Or':
		case 'Not':
		case 'Gate':
			return 'logic';
		case 'Delay':
			return 'timing';
		default:
			return 'action';
	}
}

export const conditionTag = (condition: TriggerCondition): string =>
	typeof condition === 'string' ? condition : Object.keys(condition)[0];

export function conditionLabel(condition: TriggerCondition): string {
	if (typeof condition === 'string') {
		return { RisingEdge: 'closes', FallingEdge: 'opens', AnyChange: 'changes' }[condition] ?? condition;
	}
	return 'Above' in condition ? `rises above ${condition.Above}` : `falls below ${condition.Below}`;
}

export function conditionFrom(tag: string, threshold: number): TriggerCondition {
	if (tag === 'Above') return { Above: threshold };
	if (tag === 'Below') return { Below: threshold };
	return tag as TriggerCondition;
}

export const thresholdOf = (condition: TriggerCondition): number =>
	typeof condition === 'string' ? 0 : 'Above' in condition ? condition.Above : condition.Below;

/**
 * Is this parameter a switch or a level? Decides which conditions are offered —
 * "rises above 21.5" makes sense of a temperature and nothing of a contact.
 */
export function isSwitchLike(kind: { [k: string]: unknown } | string): boolean {
	return ['Contact', 'Switch'].includes(kindLabel(kind as never));
}

// ── Layout ────────────────────────────────────────────────────────────────────

/** One node's width plus a gap, so a new node lands clear of the last one. */
export const NODE_STEP = 240;

/** How big a node is drawn, near enough to keep two from landing on each other. */
export const NODE_SIZE = { w: 184, h: 90 };

/** How far a node is moved off one already in its place. */
export const NUDGE = 32;

type Point = { x: number; y: number };
type Rect = { x: number; y: number; w: number; h: number };

const overlaps = (a: Point, b: Point) =>
	Math.abs(a.x - b.x) < NUDGE && Math.abs(a.y - b.y) < NUDGE;

/**
 * Where a new node goes: `at`, or a step down and to the right of it for as long as
 * a node already sits there — but never out of `view`, the part of the board on
 * screen. A step that would leave it goes back to the top left of the view and
 * walks again from there; a view with no free spot at all gets `at` itself.
 */
export function placeNear(at: Point, nodes: Pick<FlowNode, 'x' | 'y'>[], view: Rect | null): Point {
	const inView = (p: Point) =>
		!view || (p.x >= view.x && p.y >= view.y && p.x + NODE_SIZE.w <= view.x + view.w && p.y + NODE_SIZE.h <= view.y + view.h);
	const taken = (p: Point) => nodes.some((n) => overlaps(p, n));
	let p = { ...at };
	for (let tries = 0; tries < 200; tries++) {
		if (!taken(p)) return p;
		const next = { x: p.x + NUDGE, y: p.y + NUDGE };
		if (inView(next)) {
			p = next;
		} else if (view) {
			// Off the bottom or the right: start a new diagonal one step along the top.
			const row = Math.floor(tries / 4) + 1;
			p = { x: view.x + NUDGE * row, y: view.y + NUDGE };
			if (!inView(p)) return at;
		} else {
			p = next;
		}
	}
	return at;
}

// ── Copying ───────────────────────────────────────────────────────────────────

/** Nodes and the wires between them, lifted out of a flow. */
export type FlowClip = { nodes: FlowNode[]; edges: FlowEdge[] };

/**
 * What copying `ids` takes: those nodes, and every wire with both ends among them.
 * A wire to a node left behind would arrive in the paste pointing at nothing.
 */
export function copyNodes(nodes: FlowNode[], edges: FlowEdge[], ids: Set<string>): FlowClip {
	return {
		nodes: nodes.filter((n) => ids.has(n.id)),
		edges: edges.filter((e) => ids.has(e.from_node) && ids.has(e.to_node))
	};
}

/**
 * A clip, made new for `flowId`: every node and wire gets its own id, the wires follow
 * their nodes, the whole thing is moved so its top-left node lands on `at`, and what
 * a node was in the middle of doing — lit, counting, last fired — stays behind.
 */
export function pasteClip(clip: FlowClip, flowId: string, at: Point, newId: () => string): FlowClip {
	if (clip.nodes.length === 0) return { nodes: [], edges: [] };
	const left = Math.min(...clip.nodes.map((n) => n.x));
	const top = Math.min(...clip.nodes.map((n) => n.y));
	const ids = new Map(clip.nodes.map((n) => [n.id, newId()]));
	return {
		nodes: clip.nodes.map((n) => ({
			...n,
			id: ids.get(n.id)!,
			flow_id: flowId,
			x: at.x + (n.x - left),
			y: at.y + (n.y - top),
			active: false,
			last_fired_at: null,
			pulsed_at: null,
			due_at: null
		})),
		edges: clip.edges.map((e) => ({
			...e,
			id: newId(),
			flow_id: flowId,
			from_node: ids.get(e.from_node)!,
			to_node: ids.get(e.to_node)!
		}))
	};
}

/** The top-left corner of a clip, for working out where it is pasted. */
export function clipOrigin(clip: FlowClip): Point {
	return {
		x: Math.min(...clip.nodes.map((n) => n.x)),
		y: Math.min(...clip.nodes.map((n) => n.y))
	};
}

// ── What a port is for ────────────────────────────────────────────────────────

/**
 * What one input or output of a node is, in a word and in a sentence: the word drawn
 * beside a handle where a node's inputs mean different things (a Wait's start and
 * reset), the sentence shown when the pointer is over it. Every port has one.
 */
export function portHelp(kind: FlowNodeKind, side: 'in' | 'out', index: number): { label: string; help: string } {
	const tag = nodeTag(kind);
	const port = (side === 'in' ? inputPorts(kind) : outputPorts(kind))[index];
	const shape = port === 'Pulse' ? 'pulse, a square handle' : 'level, a round handle';
	const say = (label: string, help: string) => ({ label, help: `${label[0].toUpperCase()}${label.slice(1)} (${shape}). ${help}` });
	if (side === 'out') {
		switch (tag) {
			case 'Source':
				return say('value', 'What this watches, as it is right now. Wire it into a When to fire on a change.');
			case 'Button':
				return say('press', 'One pulse each time the button is pressed, from any console or surface.');
			case 'Condition':
				return say('fired', 'One pulse each time the change chosen here happens.');
			case 'And':
				return say('both', 'True while both inputs are true.');
			case 'Or':
				return say('either', 'True while either input is true.');
			case 'Not':
				return say('not', 'True while the input is false.');
			case 'Gate':
				return say('let through', 'The pulses that arrived while the condition was true.');
			case 'Delay':
				return say('done', 'One pulse when the wait runs out. Nothing if it was reset first.');
			default:
				return say('out', '');
		}
	}
	switch (tag) {
		case 'Condition':
			return say('watch', 'The level whose change this looks for.');
		case 'And':
		case 'Or':
			return say(index === 0 ? 'first' : 'second', tag === 'And' ? 'Both have to be true.' : 'Either one will do.');
		case 'Not':
			return say('in', 'The level to turn around.');
		case 'Gate':
			return index === 0
				? say('pulse', 'The pulse to pass on, if the condition below allows it.')
				: say('only if', 'The pulse goes through only while this is true, as it stood before the pulse changed anything.');
		case 'Delay':
			return index === 0
				? say('start', 'Starts the wait. A pulse while it is already counting starts it again from the top.')
				: say('reset', 'Stops the wait, and nothing comes out. A pulse while it is not counting does nothing.');
		case 'Action':
			return say('go', 'Each pulse here fires this action, unless the node is switched off.');
		default:
			return say('in', '');
	}
}

/**
 * Whether a node's inputs need their words drawn beside them. Only where two inputs
 * mean different things: an And's two are the same, a Wait's start and reset are not.
 */
export function namesItsInputs(kind: FlowNodeKind): boolean {
	return ['Delay', 'Gate'].includes(nodeTag(kind));
}

/** Where a new node goes: to the right of the rightmost one, or at the origin. */
export function nextNodePosition(nodes: FlowNode[]): { x: number; y: number } {
	if (nodes.length === 0) return { x: 40, y: 40 };
	const rightmost = nodes.reduce((a, b) => (b.x > a.x ? b : a));
	return { x: rightmost.x + NODE_STEP, y: rightmost.y };
}

/**
 * The nodes and edges of one flow.
 *
 * Every graph lives in the same three collections, so the editor filters rather
 * than fetching per flow — there is no path that would ask for one flow's nodes.
 */
export function graphOf(flowId: string | null, nodes: FlowNode[], edges: FlowEdge[]) {
	if (!flowId) return { nodes: [], edges: [] };
	return {
		nodes: nodes.filter((n) => n.flow_id === flowId),
		edges: edges.filter((e) => e.flow_id === flowId)
	};
}
