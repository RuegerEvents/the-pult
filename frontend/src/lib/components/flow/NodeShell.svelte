<script lang="ts">
	import { Handle, Position } from '@xyflow/svelte';
	import type { Snippet } from 'svelte';
	import type { FlowNode } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { inputPorts, namesItsInputs, nodeCategory, outputPorts, portHelp } from '$lib/flow.js';
	import { uuid } from '$lib/uuid.js';
	import { canEdit } from '$lib/stores/showLock.js';

	let {
		node,
		title,
		live = null,
		children
	}: {
		node: FlowNode;
		title: string;
		/** What it is reading or holding right now, shown in the header: a value, on or off. */
		live?: { text: string; on?: boolean } | null;
		children?: Snippet;
	} = $props();

	/**
	 * A pulse going through flashes the node once. `pulsed_at` changes on every pulse,
	 * so the flash is restarted by taking the class off for a frame and putting it back;
	 * the first value seen is history and does not flash.
	 */
	let flashing = $state(false);
	let seen: number | null | undefined = undefined;
	$effect(() => {
		const at = node.pulsed_at ?? null;
		if (seen === undefined) {
			seen = at;
			return;
		}
		if (at === seen) return;
		seen = at;
		flashing = false;
		requestAnimationFrame(() => (flashing = true));
	});

	const data = getDataContext();
	const ins = $derived(inputPorts(node.kind));
	const outs = $derived(outputPorts(node.kind));
	const category = $derived(nodeCategory(node.kind));
	const labeled = $derived(namesItsInputs(node.kind));
	/** Only an action can be switched off: it is the one place a pulse does something. */
	const off = $derived(category === 'action' && node.disabled === true);
	const toggleOff = () => void data.flow_nodes.byId(node.id).disabled!.set(off ? null : true);

	/**
	 * Its own name in place of what its kind is called, given by double-clicking the
	 * title. An empty name goes back to the kind's.
	 */
	let renaming = $state(false);
	function rename(value: string) {
		renaming = false;
		const name = value.trim() || null;
		if (name !== (node.name ?? null)) void data.flow_nodes.byId(node.id).name!.set(name);
	}

	/** A copy of this node with its settings and none of its wires, just below it. */
	function duplicate() {
		void data.flow_nodes.create({
			...node,
			id: uuid(),
			x: node.x + 24,
			y: node.y + 90,
			active: false,
			last_fired_at: null,
			pulsed_at: null,
			due_at: null
		});
	}
</script>

<div class="node {category}" class:lit={node.active && !off} class:off class:flash={flashing && !off} onanimationend={() => (flashing = false)}>
	{#each ins as port, i (i)}
		{@const help = portHelp(node.kind, 'in', i)}
		<Handle
			type="target"
			position={Position.Left}
			id={`in-${i}`}
			class={port.toLowerCase()}
			style={`top: ${28 + i * 18}px`}
			title={help.help}
		/>
		{#if labeled}
			<span class="port-label in" style={`top: ${28 + i * 18}px`} title={help.help}>{help.label}</span>
		{/if}
	{/each}

	<header>
		{#if renaming}
			<!-- svelte-ignore a11y_autofocus -->
			<input
				class="rename nodrag"
				value={node.name ?? ''}
				placeholder={title}
				autofocus
				onblur={(e) => rename(e.currentTarget.value)}
				onkeydown={(e) => {
					if (e.key === 'Enter') e.currentTarget.blur();
					if (e.key === 'Escape') renaming = false;
				}}
			/>
		{:else}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<span
				class="title"
				class:named={!!node.name}
				title={$canEdit ? `${node.name ? title + ' · ' : ''}double-click to name it` : node.name ? title : undefined}
				ondblclick={() => $canEdit && (renaming = true)}>{node.name || title}</span
			>
		{/if}
		{#if off}
			<span class="live">off</span>
		{:else if live}<span class="live" class:on={live.on}>{live.text}</span>{/if}
		{#if category === 'action' && $canEdit}
			<button
				class="remove power nodrag"
				class:is-off={off}
				title={off ? 'Switched off: a pulse stops here and nothing fires. Click to switch it on.' : 'Switch off: a pulse stops here and nothing fires, and the wires stay'}
				aria-pressed={off}
				onclick={toggleOff}>⏻</button
			>
		{/if}
		<button class="remove" title="Duplicate node: the same settings, unconnected, just below" onclick={duplicate}
			>⧉</button
		>
		<button
			class="remove"
			title="Delete node"
			onclick={() => data.flow_nodes.byId(node.id).delete()}>×</button
		>
	</header>

	{#if children}
		<div class="body">{@render children()}</div>
	{/if}

	{#each outs as port, i (i)}
		<Handle
			type="source"
			position={Position.Right}
			id={`out-${i}`}
			class={port.toLowerCase()}
			style={`top: ${28 + i * 18}px`}
			title={portHelp(node.kind, 'out', i).help}
		/>
	{/each}
</div>

<style>
	.node {
		position: relative;
		/* Fixed rather than shrink-to-fit: a graph whose boxes are all one width
		   reads as a diagram, and the layout can then space them without guessing. */
		width: 184px;
		background: #252525;
		border: 1px solid #3a3a3a;
		border-left-width: 3px;
		border-radius: 4px;
		color: #e0e0e0;
		font-size: 12px;
	}
	/* One color per family, so the shape of a graph reads before the words do. */
	.node.source { border-left-color: #4a9eff; }
	.node.logic { border-left-color: #a78bfa; }
	.node.timing { border-left-color: #fbbf24; }
	.node.action { border-left-color: #22c55e; }

	/* Selected: what a copy, a delete or a drag will take. Outside the box, so it
	   never fights the lit glow or a pulse's flash for the border. */
	:global(.svelte-flow__node.selected) > .node { outline: 2px solid #4a9eff; outline-offset: 2px; }

	/* What makes the graph an instrument rather than a diagram: a signal passing
	   through lights the node it passed through. */
	.node.lit { border-color: #4a9eff; box-shadow: 0 0 0 1px #4a9eff55, 0 0 12px #4a9eff33; }
	/* A pulse going through: one bright flash that fades, whatever the node's state. */
	.node.flash { animation: flash 0.7s ease-out; }
	@keyframes flash {
		0% { box-shadow: 0 0 0 2px #fbbf24, 0 0 18px #fbbf24aa; border-color: #fbbf24; }
		100% { box-shadow: 0 0 0 0 #fbbf2400, 0 0 0 #fbbf2400; }
	}
	/* Switched off: still where it was and still wired, plainly not taking part. */
	.node.off { opacity: 0.55; border-style: dashed; border-left-style: solid; border-left-color: #555; }
	.power.is-off { color: #e0a055; }
	.power:hover { color: #22c55e; }
	.power.is-off:hover { color: #22c55e; }

	/* The word for an input that means something its neighbor does not, outside the
	   box beside its handle, so it never sits over the node's own fields. */
	.port-label {
		position: absolute;
		right: calc(100% + 9px);
		transform: translateY(-50%);
		font-size: 9px;
		line-height: 1;
		color: #888;
		white-space: nowrap;
		pointer-events: auto;
		cursor: help;
	}

	.live {
		margin-left: auto;
		font-size: 10px;
		font-variant-numeric: tabular-nums;
		padding: 0 5px;
		border-radius: 8px;
		background: #1a1a1a;
		color: #888;
		white-space: nowrap;
		max-width: 90px;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.live.on { background: #4a9eff33; color: #cfe3ff; }

	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 6px;
		padding: 5px 4px 5px 9px;
		border-bottom: 1px solid #2e2e2e;
	}
	.title { font-weight: 600; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: #999; }
	.node.lit .title { color: #cfe3ff; }
	/* A name an operator gave it reads as theirs: as written, not shouted. */
	.title.named { text-transform: none; letter-spacing: 0; font-size: 12px; color: #e0e0e0; }
	.rename { flex: 1; min-width: 0; background: #171717; border: 1px solid #4a9eff; border-radius: 3px; color: #e0e0e0; padding: 1px 4px; font: inherit; font-size: 12px; }
	.remove { background: none; border: none; color: #666; font-size: 15px; line-height: 1; padding: 0 4px; cursor: pointer; }
	.remove:hover { color: #e05555; }
	.body { padding: 7px 9px 8px; display: flex; flex-direction: column; gap: 5px; }

	/* A level handle is round, a pulse handle is square: the two never join, and
	   the shape says so before the drag is refused. */
	.node :global(.svelte-flow__handle) { width: 9px; height: 9px; border: 1px solid #1a1a1a; }
	.node :global(.svelte-flow__handle.level) { background: #a78bfa; border-radius: 50%; }
	.node :global(.svelte-flow__handle.pulse) { background: #fbbf24; border-radius: 1px; }
</style>
