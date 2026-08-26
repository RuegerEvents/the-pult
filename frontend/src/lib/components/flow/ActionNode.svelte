<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode, TriggerAction } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { defaultValueFor, parameterKindLabel } from '$lib/patch.js';
	import { actionOf } from '$lib/flow.js';
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

	/// Switching what an action does needs a whole new value, not a field edit —
	/// the three shapes share no keys.
	function retarget(nextTag: string) {
		if (nextTag === 'GoNext') return set({ GoNext: { sequence_id: sequenceId } });
		if (nextTag === 'GoToCue') {
			const cue = cuesOf(sequenceId)[0];
			if (!cue) return;
			return set({ GoToCue: { sequence_id: sequenceId, cue_id: cue.id } });
		}
		const fixture = show.fixtures[0];
		const type = fixture && show.types.find((t) => t.id === fixture.fixture_type_id);
		const parameter = type?.parameters.find((p) => p.direction === 'Output') ?? type?.parameters[0];
		if (!fixture || !parameter) return;
		return set({
			SetParameter: {
				fixture_id: fixture.id,
				parameter: parameter.kind,
				value: defaultValueFor(parameter.kind)
			}
		});
	}
</script>

<NodeShell {node} title="Then">
	<select class="text-input nodrag" value={tag} onchange={(e) => retarget(e.currentTarget.value)}>
		<option value="GoNext">go to the next cue</option>
		<option value="GoToCue">go to a cue</option>
		<option value="SetParameter">set a parameter</option>
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
			onchange={(e) => set({ SetParameter: { ...target, fixture_id: e.currentTarget.value } })}
		>
			{#each show.fixtures as fixture (fixture.id)}
				<option value={fixture.id}>{fixture.name}</option>
			{/each}
		</select>
		<span class="hint">{parameterKindLabel(target.parameter)} · a running fade wins over this.</span>
	{/if}
</NodeShell>

<style>
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
	.hint { color: #666; font-size: 10px; line-height: 1.3; }
</style>
