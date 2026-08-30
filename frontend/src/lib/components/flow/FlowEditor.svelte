<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import {
		Background,
		Controls,
		SvelteFlow,
		type Connection,
		type Edge,
		type Node,
		type NodeTypes
	} from '@xyflow/svelte';
	import '@xyflow/svelte/dist/style.css';

	import { getDataContext } from '$lib/ws/context.js';
	import { focusOnMount } from '$lib/actions.js';
	import type {
		Cue,
		Fixture,
		FixtureType,
		Flow,
		FlowEdge,
		FlowNode,
		FlowNodeKind,
		Sequence
	} from '$lib/generated/index.js';
	import { canConnect, graphOf, nextNodePosition, nodeTag } from '$lib/flow.js';

	import { setFlowContext } from './context.js';
	import { editing } from '$lib/stores/editing.js';
	import ActionNode from './ActionNode.svelte';
	import ButtonNode from './ButtonNode.svelte';
	import ConditionNode from './ConditionNode.svelte';
	import DelayNode from './DelayNode.svelte';
	import LogicNode from './LogicNode.svelte';
	import SourceNode from './SourceNode.svelte';

	const data = getDataContext();

	let flows = $state<Flow[]>([]);
	let nodes = $state<FlowNode[]>([]);
	let edges = $state<FlowEdge[]>([]);
	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let sequences = $state<Sequence[]>([]);
	let cues = $state<Cue[]>([]);

	let selectedId = $state<string | null>(null);
	// A flow's name is how an operator finds it in a list of twenty, so renaming is
	// behind the lock along with deleting: both are edits to the show, done rarely.
	const unlocked = editing('flows');
	let renaming = $state<string | null>(null);
	let draftName = $state('');

	async function saveName(id: string) {
		const trimmed = draftName.trim();
		if (trimmed) await data.flows.byId(id).name.set(trimmed);
		renaming = null;
	}
	let creating = $state(false);
	let newName = $state('');

	setFlowContext({
		get fixtures() { return fixtures; },
		get types() { return types; },
		get sequences() { return sequences; },
		get cues() { return cues; }
	});

	const nodeTypes: NodeTypes = {
		Source: SourceNode,
		Button: ButtonNode,
		Condition: ConditionNode,
		And: LogicNode,
		Or: LogicNode,
		Not: LogicNode,
		Delay: DelayNode,
		Action: ActionNode
	};

	const selected = $derived(flows.find((f) => f.id === selectedId) ?? null);
	const graph = $derived(graphOf(selectedId, nodes, edges));

	// ── Keeping the canvas and the show in step ──────────────────────────────
	//
	// Svelte Flow owns the array it renders, so the canvas cannot simply be a
	// `$derived` of the show. Positions are held locally while a node is being
	// dragged and taken from the show otherwise, which is what stops a node lighting
	// up at 40 Hz from yanking the node somebody has hold of.

	let canvasNodes = $state.raw<Node[]>([]);
	let canvasEdges = $state.raw<Edge[]>([]);
	let dragging = $state.raw<Set<string>>(new Set());

	$effect(() => {
		const held = dragging;
		const incoming = graph.nodes;
		// Read the canvas without depending on it: Svelte Flow writes to the same
		// array as the user drags, and an effect that both reads and writes it
		// re-runs itself forever.
		const previous = untrack(() => new Map(canvasNodes.map((n) => [n.id, n])));
		canvasNodes = incoming.map((node) => {
			const before = previous.get(node.id);
			return {
				id: node.id,
				type: nodeTag(node.kind),
				position: before && held.has(node.id) ? before.position : { x: node.x, y: node.y },
				selected: before?.selected ?? false,
				data: { node }
			};
		});
	});

	$effect(() => {
		canvasEdges = graph.edges.map((edge) => ({
			id: edge.id,
			source: edge.from_node,
			sourceHandle: `out-${edge.from_port}`,
			target: edge.to_node,
			targetHandle: `in-${edge.to_port}`,
			// A pulse edge animates so a graph at rest still shows which way it runs.
			animated: true
		}));
	});

	const handleIndex = (handle: string | null | undefined) =>
		handle ? Number(handle.split('-')[1] ?? 0) : 0;

	function validConnection(connection: Connection | Edge): boolean {
		return canConnect(
			graph.nodes,
			graph.edges,
			{ node: connection.source, port: handleIndex(connection.sourceHandle) },
			{ node: connection.target, port: handleIndex(connection.targetHandle) }
		);
	}

	async function connect(connection: Connection) {
		if (!selectedId || !validConnection(connection)) return;
		await data.flow_edges.create({
			id: crypto.randomUUID(),
			flow_id: selectedId,
			from_node: connection.source,
			from_port: handleIndex(connection.sourceHandle),
			to_node: connection.target,
			to_port: handleIndex(connection.targetHandle)
		});
	}

	async function remove({ nodes: gone, edges: cut }: { nodes: Node[]; edges: Edge[] }) {
		// Deleting a node leaves edges pointing at nothing, and the evaluator ignores
		// those — but a graph full of dangling wires is unreadable, so they go too.
		const orphaned = graph.edges.filter(
			(e) => gone.some((n) => n.id === e.from_node || n.id === e.to_node)
		);
		const edgeIds = new Set([...cut.map((e) => e.id), ...orphaned.map((e) => e.id)]);
		await Promise.all([...edgeIds].map((id) => data.flow_edges.byId(id).delete()));
		await Promise.all(gone.map((n) => data.flow_nodes.byId(n.id).delete()));
	}

	async function addNode(kind: FlowNodeKind) {
		if (!selectedId) return;
		const { x, y } = nextNodePosition(graph.nodes);
		await data.flow_nodes.create({
			id: crypto.randomUUID(),
			flow_id: selectedId,
			kind,
			x,
			y,
			active: false,
			last_fired_at: null
		});
	}

	async function createFlow() {
		const name = newName.trim();
		if (!name) return;
		const id = crypto.randomUUID();
		await data.flows.create({ id, name, enabled: true });
		selectedId = id;
		newName = '';
		creating = false;
	}

	async function deleteFlow(flow: Flow) {
		const { nodes: mine, edges: wires } = graphOf(flow.id, nodes, edges);
		await Promise.all(wires.map((e) => data.flow_edges.byId(e.id).delete()));
		await Promise.all(mine.map((n) => data.flow_nodes.byId(n.id).delete()));
		await data.flows.byId(flow.id).delete();
		if (selectedId === flow.id) selectedId = null;
	}

	/// What a new node of each kind starts out as. A source with no fixture to watch
	/// would be a node that cannot be finished, so the palette waits for a rig.
	const palette = $derived.by((): { label: string; kind: FlowNodeKind; ready: boolean }[] => {
		const fixture = fixtures[0];
		const type = fixture && types.find((t) => t.id === fixture.fixture_type_id);
		const parameter = type?.parameters[0];
		return [
			{
				label: 'Watch',
				kind: { Source: { Parameter: { fixture_id: fixture?.id ?? '', parameter: parameter?.kind ?? 'Intensity' } } },
				ready: Boolean(fixture && parameter)
			},
			{ label: 'Button', kind: 'Button', ready: true },
			{ label: 'When', kind: { Condition: 'RisingEdge' }, ready: true },
			{ label: 'And', kind: 'And', ready: true },
			{ label: 'Or', kind: 'Or', ready: true },
			{ label: 'Not', kind: 'Not', ready: true },
			{ label: 'Wait', kind: { Delay: { ms: 1000 } }, ready: true },
			{
				label: 'Then',
				kind: { Action: { GoNext: { sequence_id: sequences[0]?.id ?? '' } } },
				ready: sequences.length > 0
			}
		];
	});

	onMount(() => {
		const stops = [
			data.flows.subscribeDeep((v) => {
				flows = v;
				if (!selectedId || !v.some((f) => f.id === selectedId)) selectedId = v[0]?.id ?? null;
			}),
			data.flow_nodes.subscribeDeep((v) => { nodes = v; }),
			data.flow_edges.subscribeDeep((v) => { edges = v; }),
			data.fixtures.subscribeDeep((v) => { fixtures = v; }),
			data.fixture_types.subscribeDeep((v) => { types = v; }),
			data.sequences.subscribeDeep((v) => { sequences = v; }),
			data.cues.subscribeDeep((v) => { cues = v; })
		];
		return () => stops.forEach((stop) => stop());
	});
</script>

<div class="flows">
	<aside class="rail">
		<header class="rail-head">
			<h2>Flows</h2>
			<button class="ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+'}
			</button>
		</header>

		{#if creating}
			<form onsubmit={(e) => { e.preventDefault(); createFlow(); }}>
				<input class="text-input" placeholder="What does it do?" bind:value={newName} use:focusOnMount />
			</form>
		{/if}

		{#if flows.length === 0}
			<p class="empty">Nothing wired up yet.</p>
		{:else}
			<ul>
				{#each flows as flow (flow.id)}
					<li>
						{#if renaming === flow.id && $unlocked}
							<form
								class="rename"
								onsubmit={(e) => { e.preventDefault(); saveName(flow.id); }}
							>
								<input
									class="input"
									bind:value={draftName}
									use:focusOnMount
									onblur={() => saveName(flow.id)}
									onkeydown={(e) => { if (e.key === 'Escape') renaming = null; }}
								/>
							</form>
						{:else}
							<button
								class="pick"
								class:current={flow.id === selectedId}
								class:off={!flow.enabled}
								title={$unlocked ? 'Click to open, double-click to rename' : undefined}
								onclick={() => (selectedId = flow.id)}
								ondblclick={() => {
									if (!$unlocked) return;
									renaming = flow.id;
									draftName = flow.name;
								}}
							>
								{flow.name}
							</button>
						{/if}
						<input
							type="checkbox"
							title={flow.enabled ? 'Switch off' : 'Switch on'}
							checked={flow.enabled}
							onchange={(e) => data.flows.byId(flow.id).enabled.set(e.currentTarget.checked)}
						/>
						{#if $unlocked}
							<button class="danger" title="Delete flow" onclick={() => deleteFlow(flow)}>×</button>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		<p class="note">
			Only the console leading the session fires a flow. A flow that sets a parameter a cue is
			also fading will lose to it on the next tick.
		</p>
	</aside>

	<section class="canvas">
		{#if !selected}
			<p class="empty centred">Add a flow to start drawing one.</p>
		{:else}
			<nav class="palette">
				<span class="label">Add</span>
				{#each palette as item (item.label)}
					<button
						class="ghost"
						disabled={!item.ready}
						title={item.ready ? '' : 'Needs a patched fixture and a sequence first'}
						onclick={() => addNode(item.kind)}
					>
						{item.label}
					</button>
				{/each}
			</nav>
			<div class="board">
				<SvelteFlow
					bind:nodes={canvasNodes}
					bind:edges={canvasEdges}
					{nodeTypes}
					colorMode="dark"
					fitView
					isValidConnection={validConnection}
					onconnect={connect}
					ondelete={remove}
					onnodedragstart={({ targetNode }) => {
						if (targetNode) dragging = new Set([...dragging, targetNode.id]);
					}}
					onnodedragstop={({ targetNode }) => {
						if (!targetNode) return;
						data.flow_nodes.byId(targetNode.id).x.set(targetNode.position.x);
						data.flow_nodes.byId(targetNode.id).y.set(targetNode.position.y);
						const next = new Set(dragging);
						next.delete(targetNode.id);
						dragging = next;
					}}
				>
					<Background bgColor="#1a1a1a" patternColor="#2e2e2e" />
					<Controls />
				</SvelteFlow>
			</div>
		{/if}
	</section>
</div>

<style>
	.flows { display: flex; height: 100%; min-height: 0; }
	.rail { width: 200px; flex: none; border-right: 1px solid #2a2a2a; padding: 16px 12px; display: flex; flex-direction: column; gap: 8px; overflow-y: auto; }
	.rail-head { display: flex; align-items: center; justify-content: space-between; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	ul { list-style: none; display: flex; flex-direction: column; gap: 2px; }
	li { display: flex; align-items: center; gap: 4px; }
	.pick { flex: 1; text-align: left; background: none; border: none; color: #bbb; padding: 5px 7px; border-radius: 3px; font: inherit; font-size: 13px; cursor: pointer; }
	.pick:hover { background: #252525; color: #fff; }
	.pick.current { background: #1e3a5f44; color: #4a9eff; }
	.pick.off { opacity: 0.5; }

	.canvas { flex: 1; min-width: 0; display: flex; flex-direction: column; }
	.palette { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 10px 16px; border-bottom: 1px solid #2a2a2a; }
	.label { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; margin-right: 2px; }
	.board { flex: 1; min-height: 0; }

	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.centred { margin: auto; }
	.note { color: #666; font-size: 11px; margin-top: auto; padding-top: 12px; font-style: italic; line-height: 1.4; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; width: 100%; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; font-size: 12px; cursor: pointer; }
	.ghost:hover:not(:disabled) { border-color: #555; color: #fff; }
	.ghost:disabled { opacity: 0.4; cursor: not-allowed; }
	.danger { background: none; border: none; color: #777; font-size: 15px; line-height: 1; padding: 2px 5px; cursor: pointer; }
	.danger:hover { color: #e05555; }

	/* Svelte Flow ships its own light-ish chrome; this is the rest of the console. */
	.board :global(.svelte-flow) { background: #1a1a1a; }
	.board :global(.svelte-flow__node) { font-family: inherit; }
	.board :global(.svelte-flow__edge-path) { stroke: #4a5568; stroke-width: 1.5; }
	.board :global(.svelte-flow__edge.selected .svelte-flow__edge-path) { stroke: #4a9eff; }
	.board :global(.svelte-flow__controls-button) { background: #252525; border-bottom: 1px solid #2e2e2e; fill: #bbb; }
	.board :global(.svelte-flow__controls-button:hover) { background: #2e2e2e; }
	.rename {
		flex: 1;
		min-width: 0;
	}
	.rename .input {
		width: 100%;
	}
</style>
