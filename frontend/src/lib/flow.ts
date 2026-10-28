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
