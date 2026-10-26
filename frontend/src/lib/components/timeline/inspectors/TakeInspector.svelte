<script lang="ts">
	/**
	 * A take — one parameter of one recording, over a stretch of the timeline: whether
	 * it plays, the stretch, how far it is moved, and where it came from. Drawn on its
	 * parameter's row of the Recordings lane, from the same parse of the same bytes the
	 * station plays.
	 */

	import type { TimelineTrack } from '$lib/generated/index.js';
	import type { TakeRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { collection } from '$lib/stores/show.js';
	import { current, writeColumns } from '$lib/stores/timelines.js';
	import { outlines } from '$lib/stores/takeOutlines.js';
	import { rowKeys, rowSpan, slid, trackSource } from '$lib/takes.js';
	import { clock } from '$lib/waveform.js';
	import Help from '../../Help.svelte';

	let { rows }: { rows: TakeRow[] } = $props();

	const inputs = collection('inputs');

	function stretch(track: TimelineTrack): string {
		const outline = $outlines[track.asset];
		const span = rowSpan(track, Array.isArray(outline) ? rowKeys(track, outline) : null);
		return span ? `${clock(span.fromMs)} – ${clock(span.toMs)}` : '…';
	}

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
				>A take holds its parameter over its stretch of the timeline, over playback and under the
				programmer. Drag either end to crop it, press S to slice the takes under the playhead, and
				delete a slice to remove that part. With the take the only thing picked up, its change points
				are handles: drag one in time or level, double-click its row to add one, and Delete removes the
				one picked. Recording the parameter again replaces it from where it is first touched to where
				the recording stops. Stopping the timeline lets go, over the show's home fade, into whatever
				playback holds beneath.</Help
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
		<span class="label">Plays</span>
		<span class="note">{stretch(row.track)}</span>
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
					// The stretch moves with it, as it does when the take is dragged.
					if (Number.isFinite(ms)) void change(row, slid(row.track, Math.round(ms)));
				}}
			/>
			<span class="note">ms against the timeline</span>
		</div>
		<span class="label">From</span>
		<span class="note">{trackSource(row.track, $inputs)} · {row.track.asset.slice(0, 8)}…</span>
	</div>
{/each}
