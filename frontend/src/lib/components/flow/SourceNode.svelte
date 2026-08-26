<script lang="ts">
	import type { NodeProps } from '@xyflow/svelte';
	import type { FlowNode, ParameterDefinition } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { parameterKindLabel } from '$lib/patch.js';
	import { sourceOf } from '$lib/flow.js';
	import { getFlowContext } from './context.js';
	import NodeShell from './NodeShell.svelte';

	let { data: nodeData }: NodeProps = $props();
	const node = $derived((nodeData as { node: FlowNode }).node);

	const data = getDataContext();
	const show = getFlowContext();

	const watched = $derived(sourceOf(node.kind)?.Parameter ?? { fixture_id: '', parameter: null });

	/// Inputs first: a flow almost always watches something the rig reports rather
	/// than something the console drives.
	function parametersOf(fixtureId: string): ParameterDefinition[] {
		const fixture = show.fixtures.find((f) => f.id === fixtureId);
		const type = fixture && show.types.find((t) => t.id === fixture.fixture_type_id);
		return [...(type?.parameters ?? [])].sort((a, b) =>
			a.direction === b.direction ? 0 : a.direction === 'Input' ? -1 : 1
		);
	}

	async function watch(fixtureId: string, parameterLabel?: string) {
		const parameters = parametersOf(fixtureId);
		const chosen =
			parameters.find((p) => parameterKindLabel(p.kind) === parameterLabel) ?? parameters[0];
		if (!chosen) return;
		await data.flow_nodes
			.byId(node.id)
			.kind.set({ Source: { Parameter: { fixture_id: fixtureId, parameter: chosen.kind } } });
	}
</script>

<NodeShell {node} title="Watch">
	<select
		class="text-input nodrag"
		value={watched.fixture_id}
		onchange={(e) => watch(e.currentTarget.value)}
	>
		{#each show.fixtures as fixture (fixture.id)}
			<option value={fixture.id}>{fixture.name}</option>
		{/each}
	</select>
	<select
		class="text-input nodrag"
		value={watched.parameter ? parameterKindLabel(watched.parameter) : ''}
		onchange={(e) => watch(watched.fixture_id, e.currentTarget.value)}
	>
		{#each parametersOf(watched.fixture_id) as param (parameterKindLabel(param.kind))}
			<option value={parameterKindLabel(param.kind)}>
				{parameterKindLabel(param.kind)}{param.direction === 'Input' ? '' : ' (driven)'}
			</option>
		{/each}
	</select>
</NodeShell>

<style>
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 3px 5px; font: inherit; font-size: 12px; width: 100%; }
</style>
