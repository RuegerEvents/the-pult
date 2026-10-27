import { getContext, setContext } from 'svelte';
import type {
	Cue,
	Fixture,
	FixtureType,
	FlowEdge,
	FlowNode,
	Sequence,
	Timeline,
	Variable
} from '$lib/generated/index.js';

/**
 * What a node needs to know about the rest of the show.
 *
 * Passed as context rather than in each node's `data`, because a fixture changing
 * would otherwise rewrite every node object in the graph and Svelte Flow would
 * redraw the lot — including the one somebody is dragging.
 */
export type FlowContext = {
	readonly fixtures: Fixture[];
	readonly types: FixtureType[];
	readonly sequences: Sequence[];
	readonly cues: Cue[];
	readonly variables: Variable[];
	readonly timelines: Timeline[];
	/** The flow on the canvas, so a node can read what feeds it. */
	readonly nodes: FlowNode[];
	readonly edges: FlowEdge[];
};

/**
 * Whether the level arriving at one input is true: any node feeding it is lit — the
 * station's own answer (`FlowNode::active`), read rather than worked out again here.
 */
export function inputOn(ctx: FlowContext, nodeId: string, port: number): boolean | null {
	const feeding = ctx.edges.filter((e) => e.to_node === nodeId && e.to_port === port);
	if (feeding.length === 0) return null;
	return feeding.some((e) => ctx.nodes.find((n) => n.id === e.from_node)?.active === true);
}

const KEY = 'pult:flow';

export const setFlowContext = (ctx: FlowContext) => setContext(KEY, ctx);
export const getFlowContext = () => getContext<FlowContext>(KEY);
