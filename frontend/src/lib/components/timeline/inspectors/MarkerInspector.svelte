<script lang="ts">
	/** A marker: a name at a position, to go to and to snap to. */

	import type { MarkerRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { current, locate, writeColumns } from '$lib/stores/timelines.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { rows }: { rows: MarkerRow[] } = $props();

	async function move(row: MarkerRow, at_ms: number) {
		const timeline = current(row.timelineId);
		if (timeline) await writeColumns(row.timelineId, { markers: timeline.markers.map((m) => (m.id === row.id ? { ...m, at_ms } : m)) });
	}
</script>

{#each rows as row (row.id)}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{row.name}</span>{/if}
		<span class="label">At</span>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				value={clock(row.marker.at_ms)}
				onchange={(e) => {
					const ms = parsePosition(e.currentTarget.value);
					if (ms !== null) void move(row, ms);
				}}
			/>
			<button class="btn btn-ghost" onclick={() => locate(row.timelineId, row.marker.at_ms)}>Go to</button>
		</div>
	</div>
{/each}
