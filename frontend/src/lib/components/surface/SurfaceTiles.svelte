<script lang="ts">
	/**
	 * A surface's tiles, drawn and pressable: the phone view and the editor's preview
	 * are this one component, so what the editor shows is what the phone will.
	 *
	 * Every act goes the way the panel that owns the thing already sends it — a flow
	 * button's own `press`, a variable through the unattributed `variable.*` RPCs, a
	 * timeline or sequence with the console millisecond it was taken at. Nothing here
	 * is a second way of doing any of them.
	 *
	 * Disconnected, the tiles stay where they are with their last known state, dimmed,
	 * and refuse to be pressed: a maze runner needs to see that the page is stale, and a
	 * press that went nowhere would read as one that worked.
	 */

	import type { Surface, SurfaceTile, TileTarget } from '$lib/generated/index.js';
	import { clock } from '$lib/waveform.js';
	import {
		inkOn,
		isAction,
		parameterReading,
		targetExists,
		tileLabel,
		timelinePosition,
		variableReading,
		type SurfaceShow
	} from '$lib/surface.js';
	import { parameterKey } from '$lib/patch.js';
	import { output, watching } from '$lib/stores/output.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';

	let {
		surface,
		show,
		live = true
	}: {
		surface: Surface;
		show: SurfaceShow;
		/** False while the console is not reachable: tiles show, and do nothing. */
		live?: boolean;
	} = $props();

	const data = getDataContext();
	const client = getClientContext();

	let problem = $state<string | null>(null);
	let pressed = $state<string | null>(null);

	// The parameters this page reads, so the output loop evaluates them and only them.
	const keys = $derived(
		surface.tiles.flatMap((t) =>
			t.target.kind === 'Parameter' ? [`${t.target.fixture_id}/${parameterKey(t.target.parameter)}`] : []
		)
	);
	$effect(() => {
		const registered = watching(keys);
		return () => registered.stop();
	});

	// A clock for the timeline tiles, and only while one of them is running: a phone in
	// a pocket with nothing moving on its page should not be drawing frames.
	let now = $state<number | null>(consoleNow());
	const anyRunning = $derived(
		surface.tiles.some(
			(t) =>
				t.target.kind === 'Timeline' &&
				show.timelines.find((tl) => tl.id === (t.target as { timeline_id: string }).timeline_id)?.running
		)
	);
	$effect(() => {
		if (!anyRunning) return;
		let frame = requestAnimationFrame(function tick() {
			now = consoleNow();
			frame = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frame);
	});

	/** The console millisecond, or a refusal: an act without one would be anchored wrong. */
	function at(): number {
		const stamp = consoleNow();
		if (stamp === null) throw new Error('This page has not worked out the station clock yet.');
		return stamp;
	}

	async function act(tile: SurfaceTile) {
		if (!live || !isAction(tile.target) || !targetExists(tile.target, show)) return;
		pressed = tile.id;
		setTimeout(() => {
			if (pressed === tile.id) pressed = null;
		}, 180);
		try {
			problem = null;
			await run(tile.target);
		} catch (e) {
			problem = e instanceof Error ? e.message : String(e);
		}
	}

	async function run(target: TileTarget) {
		switch (target.kind) {
			case 'FlowButton':
				await data.flow_nodes.byId(target.node_id).press();
				return;
			case 'VariableSetter': {
				const op = target.op;
				const variableId = target.variable_id;
				if (op === 'Toggle') await client.call('variable.toggle', { variableId });
				else if (op === 'Reset') await client.call('variable.reset', { variableId });
				else if ('Set' in op) await client.call('variable.set', { variableId, value: op.Set });
				else await client.call('variable.add', { variableId, by: op.Add });
				return;
			}
			case 'Timeline': {
				const timeline = show.timelines.find((t) => t.id === target.timeline_id);
				if (!timeline) return;
				const stamp = at();
				const proxy = data.timelines.byId(timeline.id);
				if (timeline.running) await proxy.stop({ at: stamp });
				else await proxy.play(target.from_start ? { at: stamp, fromMs: 0 } : { at: stamp });
				return;
			}
			case 'SequenceGo':
				await data.sequences.byId(target.sequence_id).goNext({ at: at() });
				return;
			case 'SequenceOff':
				await data.sequences.byId(target.sequence_id).off({ at: at() });
				return;
		}
	}

	/** What a tile shows as its state, beside its label. */
	function stateOf(tile: SurfaceTile): { text: string; colour: string | null } | null {
		const target = tile.target;
		switch (target.kind) {
			case 'VariableReadout':
				return variableReading(target, show, tile.colour);
			case 'Parameter':
				return parameterReading(target, show, (f, k) => $output.value(f, k), tile.colour);
			case 'Timeline': {
				const timeline = show.timelines.find((t) => t.id === target.timeline_id);
				if (!timeline) return null;
				const position = timelinePosition(timeline, now);
				const where = position === null ? '' : ` · ${clock(position).slice(0, -3)}`;
				return {
					text: (timeline.running ? 'Playing' : 'Stopped') + where,
					colour: timeline.running ? '#22c55e' : null
				};
			}
			default:
				return null;
		}
	}

	/** What pressing a timeline tile will do, which is the other thing from what it is doing. */
	function verbOf(tile: SurfaceTile): string | null {
		if (tile.target.kind !== 'Timeline') return null;
		const id = tile.target.timeline_id;
		return show.timelines.find((t) => t.id === id)?.running ? 'Stop' : 'Play';
	}
</script>

<div class="tiles" class:stale={!live}>
	{#each surface.tiles as tile (tile.id)}
		{@const target = tile.target}
		{@const exists = targetExists(target, show)}
		{#if target.kind === 'Heading'}
			<h3 class="heading">{tile.label}</h3>
		{:else}
			{@const state = exists ? stateOf(tile) : null}
			{@const fill = target.kind === 'VariableReadout' || target.kind === 'Parameter' ? (state?.colour ?? null) : tile.colour}
			{@const ink = fill ? inkOn(fill) : null}
			{#if isAction(target)}
				<button
					class="tile action"
					class:full={tile.size === 'Full'}
					class:pressed={pressed === tile.id}
					class:missing={!exists}
					style:background={fill}
					style:color={ink}
					disabled={!live || !exists}
					onclick={() => act(tile)}
				>
					<span class="label">{tileLabel(tile, show)}</span>
					{#if verbOf(tile)}<span class="verb">{verbOf(tile)}</span>{/if}
					{#if state}
						<span class="state">
							{#if state.colour}<span class="dot" style:background={state.colour}></span>{/if}
							{state.text}
						</span>
					{/if}
				</button>
			{:else}
				<div
					class="tile readout"
					class:full={tile.size === 'Full'}
					class:missing={!exists}
					style:background={fill}
					style:color={ink}
				>
					<span class="label">{tileLabel(tile, show)}</span>
					<span class="figure">{state?.text ?? '–'}</span>
				</div>
			{/if}
		{/if}
	{/each}
	{#if surface.tiles.length === 0}
		<p class="empty">This surface has no tiles yet.</p>
	{/if}
</div>

{#if problem}<p class="problem" role="alert">{problem}</p>{/if}

<style>
	.tiles {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
		transition: opacity 0.2s;
	}
	.tiles.stale {
		opacity: 0.45;
		filter: grayscale(0.6);
	}
	.heading {
		grid-column: 1 / -1;
		font-size: 13px;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #9a9a9a;
		margin-top: 8px;
		border-bottom: 1px solid #333;
		padding-bottom: 4px;
	}
	.tile {
		display: flex;
		flex-direction: column;
		justify-content: center;
		gap: 4px;
		/* A thumb, not a pointer: well over the 48 px a touch target needs. */
		min-height: 84px;
		padding: 10px 12px;
		border-radius: 10px;
		background: #262626;
		color: #eee;
		border: 1px solid #353535;
		font: inherit;
		text-align: left;
		overflow: hidden;
	}
	.full {
		grid-column: 1 / -1;
	}
	.action {
		cursor: pointer;
		background: #2d3a4f;
		border-color: #3d4f6b;
		touch-action: manipulation;
		user-select: none;
		-webkit-user-select: none;
		-webkit-tap-highlight-color: transparent;
	}
	.action:active:not(:disabled),
	.action.pressed {
		filter: brightness(1.45);
		transform: scale(0.98);
	}
	.action:disabled {
		cursor: default;
	}
	.missing {
		border-style: dashed;
		opacity: 0.6;
	}
	.label {
		font-size: 15px;
		font-weight: 600;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}
	.verb {
		font-size: 22px;
		font-weight: 700;
	}
	.state {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 13px;
		opacity: 0.85;
		font-variant-numeric: tabular-nums;
	}
	.dot {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.figure {
		font-size: 30px;
		font-weight: 700;
		font-variant-numeric: tabular-nums;
		line-height: 1;
	}
	.empty {
		grid-column: 1 / -1;
		color: #777;
		font-size: 13px;
	}
	.problem {
		margin-top: 10px;
		color: #f87171;
		font-size: 13px;
	}
</style>
