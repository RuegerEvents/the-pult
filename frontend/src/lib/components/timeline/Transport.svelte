<script lang="ts">
	/**
	 * The transport, on one row above the lanes: play or stop, back to the top, where
	 * the playhead is, how it ends, what it records and whether drags snap.
	 *
	 * The readout is arithmetic over the SYNCED anchor once an animation frame, the way
	 * the station works it out, and **shows a gap until this browser knows the station's
	 * clock** — a playhead out by however wrong this machine's clock is would be the
	 * plausible-wrong-number `ws/clock.ts` exists to remove.
	 *
	 * The end and the recording are chips that say what is set; pressing one opens the
	 * timeline's own settings in the Inspector, which is where they are changed.
	 */

	import Circle from '@lucide/svelte/icons/circle';
	import Magnet from '@lucide/svelte/icons/magnet';
	import Play from '@lucide/svelte/icons/play';
	import Repeat from '@lucide/svelte/icons/repeat';
	import SkipBack from '@lucide/svelte/icons/skip-back';
	import Square from '@lucide/svelte/icons/square';

	import type { Timeline } from '$lib/generated/index.js';
	import { selectSubject } from '$lib/objects.js';
	import { lapOf, locate, now, play, positionOf, stop, takes } from '$lib/stores/timelines.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { timeline, snapping = $bindable() }: { timeline: Timeline; snapping: boolean } = $props();

	const position = $derived(positionOf(timeline, $now));
	const shown = $derived(position === null ? null : lapOf(timeline, position));
	let typing = $state(false);
	const take = $derived($takes?.[timeline.id] ?? null);
</script>

<div class="transport">
	{#if timeline.running}
		<button class="btn btn-primary" onclick={() => stop(timeline.id)} title="Stop"><Square size={14} />Stop</button>
	{:else}
		<button class="btn btn-primary" onclick={() => play(timeline.id)} title="Play"><Play size={14} />Play</button>
	{/if}
	<button class="btn btn-icon" title="Back to the top" aria-label="Back to the top" onclick={() => locate(timeline.id, 0)}><SkipBack size={14} /></button>

	{#if typing}
		<!-- svelte-ignore a11y_autofocus -->
		<input
			class="input readout-input"
			placeholder="1:04.20"
			autofocus
			onkeydown={(e) => {
				if (e.key === 'Escape') typing = false;
				if (e.key === 'Enter') {
					const ms = parsePosition(e.currentTarget.value);
					if (ms !== null) void locate(timeline.id, ms);
					typing = false;
				}
			}}
			onblur={() => (typing = false)}
		/>
	{:else}
		<button class="readout" class:rolling={timeline.running} title="Type a position to go there" onclick={() => (typing = true)}>
			{#if shown === null}
				<span class="gap" title="This browser has not worked out the station's clock yet">—:—.—</span>
			{:else}
				{clock(shown.position)}{#if shown.lap > 0}<small> lap {shown.lap + 1}</small>{/if}
			{/if}
		</button>
	{/if}

	<button class="chip" title="How it ends — in the Inspector" onclick={() => selectSubject({ kind: 'timeline', id: timeline.id })}>
		{#if timeline.end?.then === 'Loop'}
			<Repeat size={12} />goes round at {clock(timeline.end.at_ms)}
		{:else if timeline.end}
			stops at {clock(timeline.end.at_ms)}
		{:else}
			runs until stopped
		{/if}
	</button>
	<button
		class="chip"
		class:armed={!!timeline.recording}
		title="What it records — in the Inspector"
		onclick={() => selectSubject({ kind: 'timeline', id: timeline.id })}
	>
		<Circle size={10} />{#if take}recording{:else if timeline.recording}armed{:else}not recording{/if}
	</button>
	<span class="spacer"></span>
	<button class="chip" class:on={snapping} aria-pressed={snapping} title="Drags snap to the grid and the markers" onclick={() => (snapping = !snapping)}>
		<Magnet size={12} />snap
	</button>
</div>

<style>
	.transport {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}
	.spacer {
		flex: 1;
	}
	.readout {
		min-width: 104px;
		background: none;
		border: none;
		color: var(--text);
		font: inherit;
		font-size: 20px;
		font-variant-numeric: tabular-nums;
		text-align: left;
		cursor: text;
		padding: 0 6px;
	}
	.readout.rolling {
		color: var(--good);
	}
	.readout small {
		font-size: var(--font-xs);
		color: var(--text-dim);
		margin-left: 4px;
	}
	.readout-input {
		width: 104px;
	}
	.gap {
		color: var(--text-faint);
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		min-height: 28px;
		padding: 0 9px;
		border: 1px solid var(--line-strong);
		border-radius: 14px;
		background: var(--bg-raised);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
		white-space: nowrap;
	}
	.chip:hover {
		color: var(--text);
		border-color: var(--line-input);
	}
	.chip.on {
		color: var(--accent);
		border-color: var(--accent-solid);
	}
	.chip.armed {
		color: var(--bad);
		border-color: var(--bad);
	}
</style>
