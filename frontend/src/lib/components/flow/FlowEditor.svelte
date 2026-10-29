<script lang="ts">
	import NameDialog from '../NameDialog.svelte';
	import Foldered from '$lib/components/objects/Foldered.svelte';
	import { objectHandle } from '$lib/objectHandle.js';
	import { onMount, untrack } from 'svelte';
	import {
		Background,
		Controls,
		SvelteFlow,
		type Connection,
		type Edge,
		type Node,
		type NodeTypes,
		type useSvelteFlow
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
		Sequence,
		Timeline,
		Variable
	} from '$lib/generated/index.js';
	import {
		NODE_SIZE,
		NUDGE,
		canConnect,
		clipOrigin,
		copyNodes,
		graphOf,
		nextNodePosition,
		nodeTag,
		pasteClip,
		placeNear,
		type FlowClip
	} from '$lib/flow.js';
	import { renaming } from '$lib/objects.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { keepView, showsAny, viewOf } from '$lib/stores/flowViews.js';
	import FlowBridge from './FlowBridge.svelte';

	import { setFlowContext } from './context.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import ActionNode from './ActionNode.svelte';
	import ButtonNode from './ButtonNode.svelte';
	import ConditionNode from './ConditionNode.svelte';
	import DelayNode from './DelayNode.svelte';
	import LogicNode from './LogicNode.svelte';
	import SourceNode from './SourceNode.svelte';
	import { uuid } from '$lib/uuid.js';

	const data = getDataContext();

	let flows = $state<Flow[]>([]);
	let nodes = $state<FlowNode[]>([]);
	let edges = $state<FlowEdge[]>([]);
	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let sequences = $state<Sequence[]>([]);
	let cues = $state<Cue[]>([]);
	let variables = $state<Variable[]>([]);
	let timelines = $state<Timeline[]>([]);

	let selectedId = $state<string | null>(null);
	// A flow's name is how an operator finds it in a list of twenty, so renaming is
	// behind the lock along with deleting: both are edits to the show, done rarely.
	// Which row is being renamed is the console's one `renaming`, so the context
	// menu's Rename and a double-click open the same field.
	const renamingId = $derived($renaming?.kind === 'flow' ? $renaming.id : null);
	let draftName = $state('');
	$effect(() => {
		const id = renamingId;
		if (id) draftName = untrack(() => flows.find((f) => f.id === id)?.name ?? '');
	});

	async function saveName(id: string) {
		// Enter submits and the field then loses focus: only the first of the two saves.
		if (renamingId !== id) return;
		renaming.set(null);
		const trimmed = draftName.trim();
		if (trimmed) await data.flows.byId(id).name.set(trimmed);
	}
	let creating = $state(false);

	setFlowContext({
		get fixtures() { return fixtures; },
		get types() { return types; },
		get sequences() { return sequences; },
		get cues() { return cues; },
		get variables() { return variables; },
		get timelines() { return timelines; },
		get nodes() { return graph.nodes; },
		get edges() { return graph.edges; }
	});

	const nodeTypes: NodeTypes = {
		Source: SourceNode,
		Button: ButtonNode,
		Condition: ConditionNode,
		And: LogicNode,
		Or: LogicNode,
		Not: LogicNode,
		Gate: LogicNode,
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

	/**
	 * Where a node was let go, until the show says so too.
	 *
	 * Letting go hands the position back to the show, and the show still holds the old
	 * one until the write has been round the station: taken from it at once, the node
	 * jumped back to where the drag began and then on to where it was dropped. So the
	 * dropped position is kept until the show agrees with it, or for a few seconds if a
	 * write never lands — after which the show is the truth again.
	 */
	const settling = new Map<string, { x: number; y: number; until: number }>();
	/** Nodes to be selected when they arrive, which is what a paste leaves selected. */
	let selectOnArrival = new Set<string>();

	$effect(() => {
		const held = dragging;
		const incoming = graph.nodes;
		// Read the canvas without depending on it: Svelte Flow writes to the same
		// array as the user drags, and an effect that both reads and writes it
		// re-runs itself forever.
		const previous = untrack(() => new Map(canvasNodes.map((n) => [n.id, n])));
		canvasNodes = incoming.map((node) => {
			const before = previous.get(node.id);
			let position = { x: node.x, y: node.y };
			const settled = settling.get(node.id);
			if (before && held.has(node.id)) {
				position = before.position;
			} else if (settled) {
				const agrees = Math.abs(settled.x - node.x) < 0.5 && Math.abs(settled.y - node.y) < 0.5;
				if (!agrees && Date.now() < settled.until) position = { x: settled.x, y: settled.y };
				else settling.delete(node.id);
			}
			return {
				id: node.id,
				type: nodeTag(node.kind),
				position,
				selected: before ? (before.selected ?? false) : selectOnArrival.has(node.id),
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
			id: uuid(),
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
		// One act, however many rows: a selection of six deleted is one Ctrl-Z.
		beginGesture();
		try {
			await Promise.all([...edgeIds].map((id) => data.flow_edges.byId(id).delete()));
			await Promise.all(gone.map((n) => data.flow_nodes.byId(n.id).delete()));
		} finally {
			endGesture();
		}
	}

	// ── The board's own controls ─────────────────────────────────────────────

	let api = $state.raw<ReturnType<typeof useSvelteFlow> | null>(null);
	let board = $state<HTMLDivElement | null>(null);

	/** The part of the drawing on screen, in the drawing's own coordinates. */
	function viewRect(): { x: number; y: number; w: number; h: number } | null {
		if (!api || !board) return null;
		const r = board.getBoundingClientRect();
		if (r.width === 0 || r.height === 0) return null;
		const a = api.screenToFlowPosition({ x: r.left, y: r.top });
		const b = api.screenToFlowPosition({ x: r.right, y: r.bottom });
		return { x: a.x, y: a.y, w: b.x - a.x, h: b.y - a.y };
	}

	/**
	 * A new node: where it was dropped, or in the middle of the view — a step off any
	 * node already there, and always somewhere on screen. Arrives selected.
	 */
	async function addNode(kind: FlowNodeKind, at?: { x: number; y: number }) {
		if (!selectedId) return;
		const view = viewRect();
		const middle = view
			? { x: view.x + view.w / 2 - NODE_SIZE.w / 2, y: view.y + view.h / 2 - NODE_SIZE.h / 2 }
			: nextNodePosition(graph.nodes);
		const { x, y } = at ?? placeNear(middle, graph.nodes, view);
		const id = uuid();
		deselectAll();
		selectOnArrival = new Set([id]);
		await data.flow_nodes.create({
			id,
			flow_id: selectedId,
			kind,
			x,
			y,
			active: false,
			last_fired_at: null
		});
	}

	function deselectAll() {
		if (canvasNodes.some((n) => n.selected)) canvasNodes = canvasNodes.map((n) => ({ ...n, selected: false }));
	}

	// ── Dragging ─────────────────────────────────────────────────────────────

	function startDrag(moving: Node[]) {
		dragging = new Set([...dragging, ...moving.map((n) => n.id)]);
	}

	/** Every node the drag carried, written as one act, and held where it was let go. */
	function stopDrag(moving: Node[]) {
		const until = Date.now() + 3000;
		const moved = moving.filter((n) => {
			const was = graph.nodes.find((g) => g.id === n.id);
			return was && (was.x !== n.position.x || was.y !== n.position.y);
		});
		for (const n of moved) settling.set(n.id, { x: n.position.x, y: n.position.y, until });
		if (moved.length > 0) {
			beginGesture();
			for (const n of moved) {
				void data.flow_nodes.byId(n.id).x.set(n.position.x);
				void data.flow_nodes.byId(n.id).y.set(n.position.y);
			}
			endGesture();
		}
		const next = new Set(dragging);
		for (const n of moving) next.delete(n.id);
		dragging = next;
	}

	/**
	 * No text is selected while anything on the board is being dragged — a node, a
	 * wire, the selection box, the view. Turned off for the whole page and not only
	 * the board, because the pointer leaves the board in most drags and the rail's
	 * names and the palette were what got selected. A press in a field is somebody
	 * selecting text in it, and is left alone.
	 */
	function holdSelection(event: PointerEvent) {
		boardActive = true;
		if (event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]')) return;
		const root = document.documentElement;
		root.classList.add('flow-dragging');
		window.getSelection()?.removeAllRanges();
		const done = () => {
			root.classList.remove('flow-dragging');
			window.removeEventListener('pointerup', done);
			window.removeEventListener('pointercancel', done);
		};
		window.addEventListener('pointerup', done);
		window.addEventListener('pointercancel', done);
	}

	// ── Copy, cut and paste ──────────────────────────────────────────────────
	//
	// Kept in `localStorage` rather than the system clipboard, which a page off
	// `localhost` cannot read without asking: a copy in one flow pastes into any other,
	// in this tab or another one on the same browser.

	const CLIP_KEY = 'pult.flowClipboard';

	/** Whether the keys are the board's: the last press on the page was on it. */
	let boardActive = false;

	const selectedNodeIds = () => new Set(canvasNodes.filter((n) => n.selected).map((n) => n.id));

	function copySelected(): boolean {
		const ids = selectedNodeIds();
		if (ids.size === 0) return false;
		const clip = copyNodes(graph.nodes, graph.edges, ids);
		try {
			localStorage.setItem(CLIP_KEY, JSON.stringify(clip));
		} catch {
			return false;
		}
		return true;
	}

	async function cutSelected() {
		if (!copySelected()) return;
		await remove({ nodes: canvasNodes.filter((n) => n.selected), edges: [] });
	}

	/**
	 * What was copied, new, in the middle of the view — stepped down and right for as
	 * long as it would land on nodes already there, so pasting twice gives two visible
	 * copies. One act, and what arrives is what is selected.
	 */
	async function paste() {
		if (!selectedId) return;
		let clip: FlowClip | null = null;
		try {
			clip = JSON.parse(localStorage.getItem(CLIP_KEY) ?? 'null');
		} catch {
			clip = null;
		}
		if (!clip?.nodes?.length) return;
		const origin = clipOrigin(clip);
		const width = Math.max(...clip.nodes.map((n) => n.x)) - origin.x + NODE_SIZE.w;
		const height = Math.max(...clip.nodes.map((n) => n.y)) - origin.y + NODE_SIZE.h;
		const view = viewRect();
		let at = view
			? { x: view.x + (view.w - width) / 2, y: view.y + (view.h - height) / 2 }
			: { x: origin.x + NUDGE, y: origin.y + NUDGE };
		const lands = (p: { x: number; y: number }) =>
			clip!.nodes.some((n) =>
				graph.nodes.some(
					(g) => Math.abs(p.x + n.x - origin.x - g.x) < NUDGE && Math.abs(p.y + n.y - origin.y - g.y) < NUDGE
				)
			);
		for (let i = 0; i < 50 && lands(at); i++) at = { x: at.x + NUDGE, y: at.y + NUDGE };
		const fresh = pasteClip(clip, selectedId, at, uuid);
		deselectAll();
		selectOnArrival = new Set(fresh.nodes.map((n) => n.id));
		beginGesture();
		try {
			await Promise.all(fresh.nodes.map((n) => data.flow_nodes.create(n)));
			await Promise.all(fresh.edges.map((e) => data.flow_edges.create(e)));
		} finally {
			endGesture();
		}
	}

	function onKey(event: KeyboardEvent) {
		if (!boardActive || !selectedId || !(event.metaKey || event.ctrlKey) || event.altKey) return;
		if (event.target instanceof HTMLElement && event.target.closest('input, textarea, select, [contenteditable]')) return;
		switch (event.key.toLowerCase()) {
			case 'c':
				if (copySelected()) event.preventDefault();
				break;
			case 'x':
				if ($canEdit && selectedNodeIds().size > 0) {
					event.preventDefault();
					void cutSelected();
				}
				break;
			case 'v':
				if ($canEdit) {
					event.preventDefault();
					void paste();
				}
				break;
			case 'a':
				event.preventDefault();
				canvasNodes = canvasNodes.map((n) => ({ ...n, selected: true }));
				break;
		}
	}

	function onPagePress(event: PointerEvent) {
		boardActive = !!board && event.target instanceof Node && board.contains(event.target);
	}

	// ── Dragging a node out of the palette ───────────────────────────────────

	const PALETTE_MIME = 'application/x-pult-flow-node';

	function dropFromPalette(event: DragEvent) {
		const label = event.dataTransfer?.getData(PALETTE_MIME);
		const item = palette.find((p) => p.label === label && p.ready);
		if (!item || !api) return;
		event.preventDefault();
		const at = api.screenToFlowPosition({ x: event.clientX, y: event.clientY });
		// Held by the middle of its title bar, which is where it was picked up from.
		void addNode(item.kind, { x: at.x - NODE_SIZE.w / 2, y: at.y - 14 });
	}

	// ── The view, per flow ───────────────────────────────────────────────────
	//
	// Each flow opens where this browser last left it — unless that view shows none
	// of its nodes, which is a view nobody wants back, and then the whole flow is
	// fitted. A flow never looked at here is fitted too.

	let nodesLoaded = $state(false);
	let viewedFor: string | null = null;

	$effect(() => {
		const id = selectedId;
		const flowApi = api;
		if (!flowApi || !id || !nodesLoaded || viewedFor === id) return;
		viewedFor = id;
		// After the canvas has taken the new flow's nodes and measured them, which a
		// fit needs: a timer rather than an animation frame, which a backgrounded tab
		// is never given.
		setTimeout(() => {
			if (selectedId !== id) return;
			const here = graph.nodes.map((n) => ({ x: n.x, y: n.y }));
			const saved = viewOf(id);
			const r = board?.getBoundingClientRect();
			if (saved && r && (here.length === 0 || showsAny(saved, here, r.width, r.height))) {
				void flowApi.setViewport(saved);
			} else if (here.length > 0) {
				void flowApi.fitView({ padding: 0.2, maxZoom: 1.2 });
			} else {
				void flowApi.setViewport({ x: 0, y: 0, zoom: 1 });
			}
		}, 60);
	});

	function keepThisView(viewport: { x: number; y: number; zoom: number }) {
		if (selectedId && viewedFor === selectedId) keepView(selectedId, viewport);
	}

	async function createFlow(name: string) {
		const id = uuid();
		await data.flows.create({ id, name, enabled: true });
		selectedId = id;
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
		// An action starts as whatever the show has something to aim it at.
		const thenKind: FlowNodeKind | null = sequences[0]
			? { Action: { GoNext: { sequence_id: sequences[0].id } } }
			: timelines[0]
				? { Action: { PlayTimeline: { timeline_id: timelines[0].id, if_running: 'Ignore' } } }
				: variables[0]
					? { Action: { SetVariable: { variable_id: variables[0].id, op: { Add: 1 } } } }
					: null;
		const fixture = fixtures[0];
		const type = fixture && types.find((t) => t.id === fixture.fixture_type_id);
		const parameter = type?.parameters[0];
		return [
			{
				label: 'Watch',
				kind: { Source: { Parameter: { fixture_id: fixture?.id ?? '', parameter: parameter?.kind ?? 'Intensity' } } },
				ready: Boolean(fixture && parameter)
			},
			{
				label: 'Read',
				kind: { Source: { Variable: { variable_id: variables[0]?.id ?? '' } } },
				ready: variables.length > 0
			},
			{ label: 'Button', kind: 'Button', ready: true },
			{ label: 'When', kind: { Condition: 'RisingEdge' }, ready: true },
			{ label: 'And', kind: 'And', ready: true },
			{ label: 'Or', kind: 'Or', ready: true },
			{ label: 'Not', kind: 'Not', ready: true },
			{ label: 'Only if', kind: 'Gate', ready: true },
			{ label: 'Wait', kind: { Delay: { ms: 1000 } }, ready: true },
			{
				label: 'Then',
				kind: thenKind ?? 'Button',
				ready: thenKind !== null
			}
		];
	});

	onMount(() => {
		const stops = [
			data.flows.subscribeDeep((v) => {
				flows = v;
				if (!selectedId || !v.some((f) => f.id === selectedId)) selectedId = v[0]?.id ?? null;
			}),
			data.flow_nodes.subscribeDeep((v) => {
				nodes = v;
				nodesLoaded = true;
			}),
			data.flow_edges.subscribeDeep((v) => { edges = v; }),
			data.fixtures.subscribeDeep((v) => { fixtures = v; }),
			data.fixture_types.subscribeDeep((v) => { types = v; }),
			data.sequences.subscribeDeep((v) => { sequences = v; }),
			data.cues.subscribeDeep((v) => { cues = v; }),
			data.variables.subscribeDeep((v) => { variables = v; }),
			data.timelines.subscribeDeep((v) => { timelines = v; })
		];
		return () => stops.forEach((stop) => stop());
	});
</script>

<svelte:window onkeydown={onKey} onpointerdowncapture={onPagePress} />

<div class="holder"><div class="flows">
	<aside class="rail">
		<header class="rail-head">
			<h2>Flows</h2>
			<button class="ghost" title="New flow" onclick={() => (creating = true)}>+</button>
		</header>

		{#if creating}
			<NameDialog
				title="New flow"
				placeholder="What does it do?"
				onsubmit={createFlow}
				onclose={() => (creating = false)}
			/>
		{/if}

		{#if flows.length === 0}
			<p class="empty">Nothing wired up yet.</p>
		{:else}
			<Foldered kind="flow" rows={flows} layout="column">
				{#snippet item(flow)}
					<div class="flow-row" use:objectHandle={{ kind: 'flow', id: flow.id }}>
						{#if renamingId === flow.id && $canEdit}
							<form
								class="rename"
								onsubmit={(e) => { e.preventDefault(); saveName(flow.id); }}
							>
								<input
									class="input"
									bind:value={draftName}
									use:focusOnMount
									onblur={() => saveName(flow.id)}
									onkeydown={(e) => {
										if (e.key === 'Escape') renaming.set(null);
										e.stopPropagation();
									}}
								/>
							</form>
						{:else}
							<button
								class="pick"
								class:current={flow.id === selectedId}
								class:off={!flow.enabled}
								title={$canEdit ? 'Click to open, double-click to rename' : undefined}
								onclick={() => (selectedId = flow.id)}
								ondblclick={() => {
									if ($canEdit) renaming.set({ kind: 'flow', id: flow.id });
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
						{#if $canEdit}
							<button class="danger" title="Delete flow" onclick={() => deleteFlow(flow)}>×</button>
						{/if}
					</div>
				{/snippet}
			</Foldered>
		{/if}

		<p class="note">
			Only the console leading the session fires a flow. A flow's own variable writes are seen
			on the next pass, never inside the one that made them.
		</p>
	</aside>

	<section class="canvas">
		{#if !selected}
			<p class="empty centered">Add a flow to start drawing one.</p>
		{:else}
			<nav class="palette">
				<span class="label">Add</span>
				{#each palette as item (item.label)}
					<button
						class="ghost"
						disabled={!item.ready}
						draggable={item.ready}
						title={item.ready ? 'Click to add in the middle of the view, or drag onto the board' : 'Needs something in the show to point at first'}
						onclick={() => addNode(item.kind)}
						ondragstart={(e) => {
							if (!e.dataTransfer) return;
							e.dataTransfer.setData(PALETTE_MIME, item.label);
							e.dataTransfer.effectAllowed = 'copy';
						}}
					>
						{item.label}
					</button>
				{/each}
			</nav>
			<!-- Selecting and moving the view, one way for each hand:
			     trackpad — one finger drags a lasso, two fingers move the view, a pinch zooms;
			     mouse — the left button drags a lasso, the middle one moves the view, and
			     Option (or Space) with the left one moves it too; ⌘ or Ctrl with the wheel zooms. -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="board"
				bind:this={board}
				onpointerdowncapture={holdSelection}
				onscrollcapture={(e) => {
					// The board's container clips and never scrolls; a browser scrolling it to
					// show a focused field slid the whole drawing out from under the pointer.
					const target = e.target as HTMLElement;
					if (target.classList?.contains('svelte-flow')) target.scrollTop = target.scrollLeft = 0;
				}}
				ondragover={(e) => {
					if (e.dataTransfer?.types.includes(PALETTE_MIME)) {
						e.preventDefault();
						e.dataTransfer.dropEffect = 'copy';
					}
				}}
				ondrop={dropFromPalette}
			>
				<SvelteFlow
					bind:nodes={canvasNodes}
					bind:edges={canvasEdges}
					{nodeTypes}
					colorMode="dark"
					deleteKey={['Backspace', 'Delete']}
					selectionOnDrag
					panOnDrag={[1]}
					panActivationKey={['Alt', 'Space']}
					panOnScroll
					zoomActivationKey={['Meta', 'Control']}
					multiSelectionKey={['Meta', 'Control', 'Shift']}
					isValidConnection={validConnection}
					onconnect={connect}
					ondelete={remove}
					onnodedragstart={({ nodes: moving }) => startDrag(moving)}
					onnodedragstop={({ nodes: moving }) => stopDrag(moving)}
					onselectiondragstart={(_, moving) => startDrag(moving)}
					onselectiondragstop={(_, moving) => stopDrag(moving)}
					onmoveend={(_, viewport) => keepThisView(viewport)}
				>
					<FlowBridge onready={(flowApi) => (api = flowApi)} />
					<Background bgColor="#1a1a1a" patternColor="#2e2e2e" />
					<Controls />
				</SvelteFlow>
			</div>
		{/if}
	</section>
</div></div>

<style>
	.holder { height: 100%; container-type: inline-size; }
	.flows { display: flex; height: 100%; min-height: 0; }
	/* Narrow — a phone, or a thin tile: the list above the board rather than beside
	   it, or the board is a strip nobody can wire anything in. */
	@container (max-width: 560px) {
		.flows { flex-direction: column; }
		.rail { width: auto; max-height: 30%; border-right: none; border-bottom: 1px solid #2a2a2a; padding: 10px 12px; }
		.note { display: none; }
		.palette { padding: 8px 10px; }
	}
	/* Dragging a node or a wire must not select the labels it passes over; the
	   fields inside a node stay selectable, since they are typed into. */
	.board { user-select: none; -webkit-user-select: none; }
	/* While a drag on the board is under way, nothing on the page is selectable. */
	:global(html.flow-dragging),
	:global(html.flow-dragging *) { user-select: none !important; -webkit-user-select: none !important; }
	.board :global(input),
	.board :global(textarea) { user-select: text; -webkit-user-select: text; }
	.rail { width: 200px; flex: none; border-right: 1px solid #2a2a2a; padding: 16px 12px; display: flex; flex-direction: column; gap: 8px; overflow-y: auto; }
	.rail-head { display: flex; align-items: center; justify-content: space-between; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.flow-row { display: flex; align-items: center; gap: 4px; }
	.pick { flex: 1; text-align: left; background: none; border: none; color: #bbb; padding: 5px 7px; border-radius: 3px; font: inherit; font-size: 13px; cursor: pointer; }
	.pick:hover { background: #252525; color: #fff; }
	.pick.current { background: #1e3a5f44; color: #4a9eff; }
	.pick.off { opacity: 0.5; }

	.canvas { flex: 1; min-width: 0; display: flex; flex-direction: column; }
	.palette { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 10px 16px; border-bottom: 1px solid #2a2a2a; }
	.label { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; margin-right: 2px; }
	.board { flex: 1; min-height: 0; }

	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.centered { margin: auto; }
	.note { color: #666; font-size: 11px; margin-top: auto; padding-top: 12px; font-style: italic; line-height: 1.4; }
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
