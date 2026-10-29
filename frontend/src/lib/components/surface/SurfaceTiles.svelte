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

	import type { AudioClip, Surface, SurfaceTile } from '$lib/generated/index.js';
	import { clock } from '$lib/waveform.js';
	import {
		inkOn,
		isAction,
		parameterReading,
		pressTarget,
		targetExists,
		tileLabel,
		timelineLength,
		timelinePosition,
		timelineProgress,
		variableReading,
		type SurfaceShow
	} from '$lib/surface.js';
	import { parameterKey } from '$lib/patch.js';
	import { output, watching } from '$lib/stores/output.js';
	import { peaksFor } from '$lib/stores/peaks.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';

	let {
		surface,
		show,
		live = true,
		selected = null,
		onselect
	}: {
		surface: Surface;
		show: SurfaceShow;
		/** False while the console is not reachable: tiles show, and do nothing. */
		live?: boolean;
		/** The tile being edited, outlined. */
		selected?: string | null;
		/**
		 * Given, a click picks the tile to edit instead of pressing it: the editor's
		 * preview is where a surface is built, the way it will look.
		 */
		onselect?: (tileId: string) => void;
	} = $props();

	const data = getDataContext();
	const client = getClientContext();

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

	// How long a clip's file is comes from its peaks, fetched once; this says "work the
	// countdown out again" when one arrives.
	let peaksArrived = $state(0);
	function fileMs(clip: AudioClip): number | null {
		void peaksArrived;
		const peaks = clip.peaks ? peaksFor(clip.peaks, () => peaksArrived++) : null;
		return peaks?.durationMs ?? null;
	}

	async function act(tile: SurfaceTile) {
		if (onselect) return onselect(tile.id);
		if (!live || !isAction(tile.target) || !targetExists(tile.target, show)) return;
		pressed = tile.id;
		setTimeout(() => {
			if (pressed === tile.id) pressed = null;
		}, 180);
		try {
			await pressTarget(tile.target, show, data, client, consoleNow());
		} catch (e) {
			addToast(e instanceof Error ? e.message : String(e));
		}
	}

	/**
	 * What a tile shows as its state, beside its label — and for a timeline that will
	 * finish, how far through it is, which is the bar along the tile's foot.
	 */
	function stateOf(tile: SurfaceTile): { text: string; color: string | null; fraction?: number } | null {
		const target = tile.target;
		switch (target.kind) {
			case 'VariableReadout':
				return variableReading(target, show, tile.color);
			case 'Parameter':
				return parameterReading(target, show, (f, k) => $output.value(f, k), tile.color);
			case 'Timeline': {
				const timeline = show.timelines.find((t) => t.id === target.timeline_id);
				if (!timeline) return null;
				const position = timelinePosition(timeline, now);
				const where = position === null ? '' : ` · ${clock(position).slice(0, -3)}`;
				// A loop is never done, so it has no countdown and no bar.
				const progress = timelineProgress(position, timelineLength(timeline, fileMs));
				// Rounded up, the way a countdown is read: it says 0:00 only once it is over.
				const left = progress ? ` · −${clock(Math.ceil(progress.remainingMs / 1000) * 1000).slice(0, -3)}` : '';
				return {
					text: (timeline.running ? 'Playing' : 'Stopped') + where + left,
					color: timeline.running ? '#22c55e' : null,
					fraction: progress?.fraction
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
			{#if onselect}
				<button class="heading pick" class:selected={selected === tile.id} onclick={() => onselect(tile.id)}>{tile.label}</button>
			{:else}
				<h3 class="heading">{tile.label}</h3>
			{/if}
		{:else}
			{@const state = exists ? stateOf(tile) : null}
			{@const fill = target.kind === 'VariableReadout' || target.kind === 'Parameter' ? (state?.color ?? null) : tile.color}
			{@const ink = fill ? inkOn(fill) : null}
			{#if isAction(target)}
				<button
					class="tile action"
					class:full={tile.size === 'Full'}
					class:pressed={pressed === tile.id}
					class:missing={!exists}
					class:selected={selected === tile.id}
					style:background={fill}
					style:color={ink}
					disabled={!onselect && (!live || !exists)}
					onclick={() => act(tile)}
				>
					<span class="label">{tileLabel(tile, show)}</span>
					{#if verbOf(tile)}<span class="verb">{verbOf(tile)}</span>{/if}
					{#if target.kind === 'VariableReadout'}
						<!-- A readout that is its own switch: the figure, and pressing changes it. -->
						<span class="figure">{state?.text ?? '–'}</span>
					{:else if state}
						<span class="state">
							{#if state.color}<span class="dot" style:background={state.color}></span>{/if}
							{state.text}
						</span>
						{#if state.fraction !== undefined}
							<span class="progress" style:width="{state.fraction * 100}%"></span>
						{/if}
					{/if}
				</button>
			{:else}
				{#if onselect}
					<button
						class="tile readout pick"
						class:full={tile.size === 'Full'}
						class:missing={!exists}
						class:selected={selected === tile.id}
						style:background={fill}
						style:color={ink}
						onclick={() => onselect(tile.id)}
					>
						<span class="label">{tileLabel(tile, show)}</span>
						<span class="figure">{state?.text ?? '–'}</span>
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
		{/if}
	{/each}
	{#if surface.tiles.length === 0}
		<p class="empty">This surface has no tiles yet.</p>
	{/if}
</div>

<style>
	/* Being edited: outlined, the way a selected object is everywhere else. */
	.selected {
		outline: 2px solid var(--accent, #4a9eff);
		outline-offset: 2px;
	}
	.pick {
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.heading.pick {
		background: none;
		border: none;
		border-bottom: 1px solid #333;
		width: 100%;
	}
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
		position: relative;
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
	/* How far through a timeline is, along the tile's foot. */
	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 4px;
		background: currentColor;
		opacity: 0.7;
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
</style>
