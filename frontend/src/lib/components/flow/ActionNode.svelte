<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type {
		FlowNode,
		IfRunning,
		ParameterDefinition,
		TriggerAction
	} from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { defaultValueFor, parameterKindLabel } from '$lib/patch.js';
	import { actionOf, variableOpFrom, variableOpParts } from '$lib/flow.js';
	import HomeValue from '$lib/components/HomeValue.svelte';
	import { getFlowContext } from './context.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
	const show = getFlowContext();

	const action = $derived(actionOf(node.kind));
	const tag = $derived(action ? Object.keys(action)[0] : 'GoNext');

	const set = (next: TriggerAction) => data.flow_nodes.byId(node.id).kind.set({ Action: next });

	const sequenceId = $derived(
		action && 'GoNext' in action
			? action.GoNext.sequence_id
			: action && 'GoToCue' in action
				? action.GoToCue.sequence_id
				: (show.sequences[0]?.id ?? '')
	);
	const cuesOf = (id: string) => {
		const sequence = show.sequences.find((s) => s.id === id);
		return (sequence?.cue_ids ?? [])
			.map((cueId) => show.cues.find((c) => c.id === cueId))
			.filter((c) => c !== undefined);
	};

	/// What an action can set on a fixture: what the console drives, never what the
	/// fixture reports.
	function outputsOf(fixtureId: string): ParameterDefinition[] {
		const fixture = show.fixtures.find((f) => f.id === fixtureId);
		const type = fixture && show.types.find((t) => t.id === fixture.fixture_type_id);
		return (type?.parameters ?? []).filter((p) => p.direction === 'Output');
	}

	function setParameter(fixtureId: string, label?: string) {
		const parameters = outputsOf(fixtureId);
		const parameter = parameters.find((p) => parameterKindLabel(p.kind) === label) ?? parameters[0];
		if (!parameter) return;
		return set({
			SetParameter: {
				fixture_id: fixtureId,
				parameter: parameter.kind,
				value: defaultValueFor(parameter.kind)
			}
		});
	}

	/// Switching what an action does needs a whole new value, not a field edit —
	/// the shapes share no keys.
	function retarget(nextTag: string) {
		switch (nextTag) {
			case 'GoNext':
				return set({ GoNext: { sequence_id: sequenceId } });
			case 'GoToCue': {
				const cue = cuesOf(sequenceId)[0];
				if (!cue) return;
				return set({ GoToCue: { sequence_id: sequenceId, cue_id: cue.id } });
			}
			case 'SetVariable': {
				const variable = show.variables[0];
				if (!variable) return;
				return set({ SetVariable: { variable_id: variable.id, op: { Add: 1 } } });
			}
			case 'PlayTimeline': {
				const timeline = show.timelines[0];
				if (!timeline) return;
				return set({ PlayTimeline: { timeline_id: timeline.id, if_running: 'Ignore' } });
			}
			case 'StopTimeline': {
				const timeline = show.timelines[0];
				if (!timeline) return;
				return set({ StopTimeline: { timeline_id: timeline.id } });
			}
			default: {
				const fixture = show.fixtures.find((f) => outputsOf(f.id).length > 0);
				if (fixture) return setParameter(fixture.id);
			}
		}
	}
</script>

<NodeShell {node} title="Then">
	<select class="text-input nodrag" value={tag} onchange={(e) => retarget(e.currentTarget.value)}>
		<option value="GoNext" disabled={show.sequences.length === 0}>go to the next cue</option>
		<option value="GoToCue" disabled={show.sequences.length === 0}>go to a cue</option>
		<option value="SetParameter">set a parameter</option>
		<option value="SetVariable" disabled={show.variables.length === 0}>change a variable</option>
		<option value="PlayTimeline" disabled={show.timelines.length === 0}>play a timeline</option>
		<option value="StopTimeline" disabled={show.timelines.length === 0}>stop a timeline</option>
	</select>

	{#if action && ('GoNext' in action || 'GoToCue' in action)}
		<select
			class="text-input nodrag"
			value={sequenceId}
			onchange={(e) =>
				'GoToCue' in action
					? retarget('GoToCue')
					: set({ GoNext: { sequence_id: e.currentTarget.value } })}
		>
			{#each show.sequences as sequence (sequence.id)}
				<option value={sequence.id}>{sequence.name}</option>
			{/each}
		</select>
	{/if}

	{#if action && 'GoToCue' in action}
		<select
			class="text-input nodrag"
			value={action.GoToCue.cue_id}
			onchange={(e) =>
				set({ GoToCue: { sequence_id: sequenceId, cue_id: e.currentTarget.value } })}
		>
			{#each cuesOf(sequenceId) as cue (cue.id)}
				<option value={cue.id}>{cue.number} · {cue.name}</option>
			{/each}
		</select>
	{/if}

	{#if action && 'SetParameter' in action}
		{@const target = action.SetParameter}
		<select
			class="text-input nodrag"
			value={target.fixture_id}
			onchange={(e) => setParameter(e.currentTarget.value)}
		>
			{#each show.fixtures.filter((f) => outputsOf(f.id).length > 0) as fixture (fixture.id)}
				<option value={fixture.id}>{fixture.name}</option>
			{/each}
		</select>
		<select
			class="text-input nodrag"
			value={parameterKindLabel(target.parameter)}
			onchange={(e) => setParameter(target.fixture_id, e.currentTarget.value)}
		>
			{#each outputsOf(target.fixture_id) as param (parameterKindLabel(param.kind))}
				<option value={parameterKindLabel(param.kind)}>{parameterKindLabel(param.kind)}</option>
			{/each}
		</select>
		<div class="nodrag value">
			<span class="unit">to</span>
			<HomeValue
				label={parameterKindLabel(target.parameter)}
				value={target.value}
				onchange={(value) => set({ SetParameter: { ...target, value } })}
			/>
		</div>
		<span class="hint">Takes the parameter from any fade; the next cue takes it back.</span>
	{/if}

	{#if action && 'SetVariable' in action}
		{@const target = action.SetVariable}
		{@const parts = variableOpParts(target.op)}
		<select
			class="text-input nodrag"
			value={target.variable_id}
			onchange={(e) =>
				set({ SetVariable: { ...target, variable_id: e.currentTarget.value } })}
		>
			{#each show.variables as v (v.id)}
				<option value={v.id}>{v.name}</option>
			{/each}
		</select>
		<div class="row">
			<select
				class="text-input nodrag"
				value={parts.tag}
				onchange={(e) =>
					set({
						SetVariable: { ...target, op: variableOpFrom(e.currentTarget.value, parts.amount) }
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
					class="text-input nodrag amount"
					type="number"
					step="1"
					value={parts.amount}
					onchange={(e) =>
						set({
							SetVariable: {
								...target,
								op: variableOpFrom(parts.tag, e.currentTarget.valueAsNumber || 0)
							}
						})}
				/>
			{/if}
		</div>
	{/if}

	{#if action && ('PlayTimeline' in action || 'StopTimeline' in action)}
		{@const timelineId =
			'PlayTimeline' in action ? action.PlayTimeline.timeline_id : action.StopTimeline.timeline_id}
		<select
			class="text-input nodrag"
			value={timelineId}
			onchange={(e) =>
				'PlayTimeline' in action
					? set({ PlayTimeline: { ...action.PlayTimeline, timeline_id: e.currentTarget.value } })
					: set({ StopTimeline: { timeline_id: e.currentTarget.value } })}
		>
			{#each show.timelines as timeline (timeline.id)}
				<option value={timeline.id}>{timeline.name}</option>
			{/each}
		</select>
		{#if 'PlayTimeline' in action}
			{@const play = action.PlayTimeline}
			<select
				class="text-input nodrag"
				value={play.if_running}
				onchange={(e) =>
					set({ PlayTimeline: { ...play, if_running: e.currentTarget.value as IfRunning } })}
			>
				<option value="Ignore">from the start, unless it is running</option>
				<option value="Restart">from the start, even if running</option>
			</select>
		{/if}
	{/if}
</NodeShell>

<style>
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
	.row { display: flex; gap: 4px; }
	.amount { width: 56px; flex: none; }
	.value { display: flex; align-items: center; gap: 5px; }
	.unit { color: #777; font-size: 11px; }
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
