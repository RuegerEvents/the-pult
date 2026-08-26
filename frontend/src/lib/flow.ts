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
	TriggerSource
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
			return ['Pulse'];
		default:
			return [];
	}
}

/**
 * Can these two handles be joined?
 *
 * A connection is valid when both ends carry the same kind of signal and the
 * target port is free — one input takes one edge, so rewiring means replacing
 * rather than silently stacking a second source behind the first.
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

	return !edges.some((e) => e.to_node === to.node && e.to_port === to.port);
}

// ── Labels ────────────────────────────────────────────────────────────────────

/** Which family a node belongs to, for colouring it. */
export function nodeCategory(kind: FlowNodeKind): 'source' | 'logic' | 'timing' | 'action' {
	switch (nodeTag(kind)) {
		case 'Source':
		case 'Button':
			return 'source';
		case 'Condition':
		case 'And':
		case 'Or':
		case 'Not':
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
