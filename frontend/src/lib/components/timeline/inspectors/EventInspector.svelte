<script lang="ts">
	/**
	 * An event: when, and which Go into which sequence. A timeline says *when*; the
	 * sequence stays the stack, so all an event can be is a position and a Go.
	 */

	import type { TimelineAction, TimelineEvent } from '$lib/generated/index.js';
	import type { EventRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { collection } from '$lib/stores/show.js';
	import { current, locate, writeColumns } from '$lib/stores/timelines.js';
	import { cuesOf, sequenceOf } from '$lib/timeline/describe.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { rows }: { rows: EventRow[] } = $props();

	const sequences = collection('sequences');
	const cues = collection('cues');

	async function change(row: EventRow, patch: Partial<TimelineEvent>) {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const events = timeline.events.map((e) => (e.id === row.id ? { ...e, ...patch } : e));
		await writeColumns(row.timelineId, { events: events.sort((a, b) => a.at_ms - b.at_ms) });
	}

	const choice = (action: TimelineAction) =>
		'GoToCue' in action ? action.GoToCue.cue_id : 'GoNext' in action ? '' : 'off';
</script>

{#each rows as row (row.id)}
	{@const event = row.event}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{row.name}</span>{/if}
		<span class="label">At</span>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				value={clock(event.at_ms)}
				onchange={(e) => {
					const ms = parsePosition(e.currentTarget.value);
					if (ms !== null) void change(row, { at_ms: ms });
				}}
			/>
			<button class="btn btn-ghost" onclick={() => locate(row.timelineId, event.at_ms)}>Go to</button>
		</div>
		<span class="label">Sequence</span>
		<select
			class="select"
			disabled={!$canEdit}
			value={sequenceOf(event.action)}
			onchange={(e) => void change(row, { action: { GoNext: { sequence_id: e.currentTarget.value } } })}
		>
			{#each $sequences as sequence (sequence.id)}
				<option value={sequence.id}>{sequence.name}</option>
			{/each}
		</select>
		<span class="label">Does</span>
		<select
			class="select"
			disabled={!$canEdit}
			value={choice(event.action)}
			onchange={(e) => {
				const sequence_id = sequenceOf(event.action);
				const picked = e.currentTarget.value;
				const action: TimelineAction =
					picked === '' ? { GoNext: { sequence_id } } : picked === 'off' ? { Off: { sequence_id } } : { GoToCue: { sequence_id, cue_id: picked } };
				void change(row, { action });
			}}
		>
			<option value="">Go next</option>
			<option value="off">Off</option>
			{#each cuesOf(sequenceOf(event.action), $sequences, $cues) as cue (cue.id)}
				<option value={cue.id}>Go to {cue.name}</option>
			{/each}
		</select>
	</div>
{/each}
