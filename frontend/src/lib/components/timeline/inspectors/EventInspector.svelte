<script lang="ts">
	import FolderedOptions from '$lib/components/objects/FolderedOptions.svelte';
	/**
	 * An event: when, and what it does. A Go, a jump or an Off into a sequence — the
	 * sequence stays the stack — or one of the acts a flow has: play or stop a timeline,
	 * change a variable. The last two kinds are what let a timeline hand over to the next
	 * one: a scare whose last event plays the ambience loop again.
	 */

	import type { IfRunning, TimelineAction, TimelineEvent } from '$lib/generated/index.js';
	import type { EventRow } from '$lib/kinds/timeline.kind.js';
	import { variableOpFrom, variableOpParts } from '$lib/flow.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { collection } from '$lib/stores/show.js';
	import { current, locate, writeColumns } from '$lib/stores/timelines.js';
	import { cuesOf, sequenceOf, timelineOf } from '$lib/timeline/describe.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { rows }: { rows: EventRow[] } = $props();

	const sequences = collection('sequences');
	const cues = collection('cues');
	const timelines = collection('timelines');
	const variables = collection('variables');

	async function change(row: EventRow, patch: Partial<TimelineEvent>) {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const events = timeline.events.map((e) => (e.id === row.id ? { ...e, ...patch } : e));
		await writeColumns(row.timelineId, { events: events.sort((a, b) => a.at_ms - b.at_ms) });
	}

	type Kind = 'sequence' | 'play' | 'stop' | 'variable';
	const kindOf = (action: TimelineAction): Kind =>
		'PlayTimeline' in action ? 'play' : 'StopTimeline' in action ? 'stop' : 'SetVariable' in action ? 'variable' : 'sequence';

	/** A first action of each kind, filled in from what the show has. */
	function fresh(kind: Kind, row: EventRow): TimelineAction | null {
		const otherTimeline = $timelines.find((t) => t.id !== row.timelineId)?.id ?? row.timelineId;
		switch (kind) {
			case 'sequence':
				return $sequences[0] ? { GoNext: { sequence_id: $sequences[0].id } } : null;
			case 'play':
				return { PlayTimeline: { timeline_id: otherTimeline, if_running: 'Ignore' } };
			case 'stop':
				return { StopTimeline: { timeline_id: otherTimeline } };
			case 'variable':
				return $variables[0] ? { SetVariable: { variable_id: $variables[0].id, op: { Add: 1 } } } : null;
		}
	}

	const choice = (action: TimelineAction) =>
		'GoToCue' in action ? action.GoToCue.cue_id : 'GoNext' in action ? '' : 'off';
</script>

{#each rows as row (row.id)}
	{@const event = row.event}
	{@const kind = kindOf(event.action)}
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
		<span class="label">Kind</span>
		<select
			class="select"
			disabled={!$canEdit}
			value={kind}
			onchange={(e) => {
				const action = fresh(e.currentTarget.value as Kind, row);
				if (action) void change(row, { action });
			}}
		>
			<option value="sequence" disabled={$sequences.length === 0}>A sequence</option>
			<option value="play">Play a timeline</option>
			<option value="stop">Stop a timeline</option>
			<option value="variable" disabled={$variables.length === 0}>Change a variable</option>
		</select>

		{#if kind === 'sequence'}
			{@const sequence_id = sequenceOf(event.action) ?? ''}
			<span class="label">Sequence</span>
			<select
				class="select"
				disabled={!$canEdit}
				value={sequence_id}
				onchange={(e) => void change(row, { action: { GoNext: { sequence_id: e.currentTarget.value } } })}
			>
				<FolderedOptions rows={$sequences} />
			</select>
			<span class="label">Does</span>
			<select
				class="select"
				disabled={!$canEdit}
				value={choice(event.action)}
				onchange={(e) => {
					const picked = e.currentTarget.value;
					const action: TimelineAction =
						picked === '' ? { GoNext: { sequence_id } } : picked === 'off' ? { Off: { sequence_id } } : { GoToCue: { sequence_id, cue_id: picked } };
					void change(row, { action });
				}}
			>
				<option value="">Go next</option>
				<option value="off">Off</option>
				{#each cuesOf(sequence_id, $sequences, $cues) as cue (cue.id)}
					<option value={cue.id}>Go to {cue.name}</option>
				{/each}
			</select>
		{:else if kind === 'play' || kind === 'stop'}
			{@const timeline_id = timelineOf(event.action) ?? ''}
			<span class="label">Timeline</span>
			<select
				class="select"
				disabled={!$canEdit}
				value={timeline_id}
				onchange={(e) => {
					const id = e.currentTarget.value;
					const action: TimelineAction =
						'PlayTimeline' in event.action
							? { PlayTimeline: { ...event.action.PlayTimeline, timeline_id: id } }
							: { StopTimeline: { timeline_id: id } };
					void change(row, { action });
				}}
			>
				{#each $timelines as timeline (timeline.id)}
					<option value={timeline.id}>{timeline.name}{timeline.id === row.timelineId ? ' (this one)' : ''}</option>
				{/each}
			</select>
			{#if 'PlayTimeline' in event.action}
				{@const play = event.action.PlayTimeline}
				<span class="label">If running</span>
				<select
					class="select"
					disabled={!$canEdit}
					value={play.if_running}
					onchange={(e) =>
						void change(row, {
							action: { PlayTimeline: { ...play, if_running: e.currentTarget.value as IfRunning } }
						})}
				>
					<option value="Ignore">leave it running</option>
					<option value="Restart">start it again</option>
				</select>
			{/if}
		{:else if 'SetVariable' in event.action}
			{@const target = event.action.SetVariable}
			{@const parts = variableOpParts(target.op)}
			<span class="label">Variable</span>
			<select
				class="select"
				disabled={!$canEdit}
				value={target.variable_id}
				onchange={(e) => void change(row, { action: { SetVariable: { ...target, variable_id: e.currentTarget.value } } })}
			>
				{#each $variables as v (v.id)}
					<option value={v.id}>{v.name}</option>
				{/each}
			</select>
			<span class="label">Change</span>
			<div class="inline">
				<select
					class="select"
					disabled={!$canEdit}
					value={parts.tag}
					onchange={(e) =>
						void change(row, {
							action: { SetVariable: { ...target, op: variableOpFrom(e.currentTarget.value, parts.amount) } }
						})}
				>
					<option value="Add">add</option>
					<option value="Subtract">subtract</option>
					<option value="Set">set to</option>
					<option value="Toggle">toggle</option>
					<option value="Reset">reset</option>
				</select>
				{#if parts.tag === 'Add' || parts.tag === 'Subtract' || parts.tag === 'Set'}
					<input
						class="input narrow"
						type="number"
						step="1"
						disabled={!$canEdit}
						value={parts.amount}
						onchange={(e) =>
							void change(row, {
								action: {
									SetVariable: { ...target, op: variableOpFrom(parts.tag, e.currentTarget.valueAsNumber || 0) }
								}
							})}
					/>
				{/if}
			</div>
		{/if}
	</div>
{/each}
