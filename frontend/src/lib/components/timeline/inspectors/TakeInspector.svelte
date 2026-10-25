<script lang="ts">
	/**
	 * A take: whether it plays, how far it is moved against the timeline, and where it
	 * came from. What it recorded is drawn on its lane, a row per key, from the same
	 * parse of the same bytes the station plays.
	 */

	import type { TimelineTrack } from '$lib/generated/index.js';
	import type { TakeRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { collection } from '$lib/stores/show.js';
	import { current, writeColumns } from '$lib/stores/timelines.js';
	import { trackSource } from '$lib/takes.js';
	import Help from '../../Help.svelte';

	let { rows }: { rows: TakeRow[] } = $props();

	const inputs = collection('inputs');

	async function change(row: TakeRow, patch: Partial<TimelineTrack>) {
		const timeline = current(row.timelineId);
		if (timeline) await writeColumns(row.timelineId, { tracks: timeline.tracks.map((t) => (t.id === row.id ? { ...t, ...patch } : t)) });
	}
</script>

{#each rows as row (row.id)}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{row.name}</span>{/if}
		<span class="label"
			>Plays <Help label="What a take does"
				>A take holds every key it recorded, over playback and under the programmer. Stopping the
				timeline lets those keys go, over the show's home fade, into whatever playback holds beneath
				them.</Help
			></span
		>
		<label class="inline note">
			<input
				type="checkbox"
				disabled={!$canEdit}
				checked={row.track.enabled}
				onchange={(e) => void change(row, { enabled: e.currentTarget.checked })}
			/>
			with the timeline
		</label>
		<span class="label">Moved</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				step="10"
				disabled={!$canEdit}
				title="Positive plays it later, negative earlier — a take that arrived 40 ms late wants −40"
				value={row.track.offset_ms}
				onchange={(e) => {
					const ms = Number(e.currentTarget.value);
					if (Number.isFinite(ms)) void change(row, { offset_ms: Math.round(ms) });
				}}
			/>
			<span class="note">ms against the timeline</span>
		</div>
		<span class="label">From</span>
		<span class="note">{trackSource(row.track, $inputs)} · {row.track.asset.slice(0, 8)}…</span>
	</div>
{/each}
