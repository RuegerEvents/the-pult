import { getContext, setContext } from 'svelte';
import type { Cue, Fixture, FixtureType, Sequence } from '$lib/generated/index.js';

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
};

const KEY = 'pult:flow';

export const setFlowContext = (ctx: FlowContext) => setContext(KEY, ctx);
export const getFlowContext = () => getContext<FlowContext>(KEY);
