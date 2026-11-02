<script lang="ts">
	/**
	 * A tempo segment: a tempo from a position, and how many beats make a bar. A segment
	 * per tempo change, because a song has a rallentando in it and a show has an interval.
	 */

	import type { GridSegment } from '$lib/generated/index.js';
	import type { SegmentRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { current, writeColumns } from '$lib/stores/timelines.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { rows }: { rows: SegmentRow[] } = $props();

	/** `grid` is kept sorted, which is what keeps a segment's index — its name — still. */
	async function change(row: SegmentRow, patch: Partial<GridSegment>) {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const grid = timeline.grid.map((s, i) => (i === row.index ? { ...s, ...patch } : s));
		await writeColumns(row.timelineId, { grid: grid.sort((a, b) => a.at_ms - b.at_ms) });
	}
</script>

{#each rows as row (row.id)}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{row.name}</span>{/if}
		<span class="label">From</span>
		<input
			class="input narrow"
			disabled={!$canEdit}
			value={clock(row.segment.at_ms)}
			onchange={(e) => {
				const ms = parsePosition(e.currentTarget.value);
				if (ms !== null) void change(row, { at_ms: ms });
			}}
		/>
		<span class="label">Tempo</span>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				value={Math.round(row.segment.bpm * 100) / 100}
				onchange={(e) => {
					const bpm = Number(e.currentTarget.value);
					if (Number.isFinite(bpm) && bpm > 0) void change(row, { bpm });
				}}
			/>
			<span class="note">bpm</span>
		</div>
		<span class="label">Bar</span>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				value={row.segment.beats_per_bar}
				onchange={(e) => {
					const beats = Number(e.currentTarget.value);
					if (Number.isFinite(beats) && beats >= 1) void change(row, { beats_per_bar: Math.round(beats) });
				}}
			/>
			<span class="note">beats</span>
		</div>
	</div>
{/each}
