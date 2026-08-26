<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { FixtureType, ParameterDefinition, ParameterKind } from '$lib/generated/index.js';
	import { parameterKindLabel, PARAMETER_KINDS, defaultValueFor } from '$lib/patch.js';

	const data = getDataContext();

	let types = $state<FixtureType[]>([]);
	let expanded = $state<string | null>(null);
	let newName = $state('');
	let creating = $state(false);

	async function createType() {
		const name = newName.trim();
		if (!name) return;
		await data.fixture_types.create({
			id: crypto.randomUUID(),
			name,
			manufacturer: 'Generic',
			channel_count: 1,
			parameters: [{ kind: 'Intensity', dmx_channel: 1, default_value: { type: 'Float', value: 0 } }]
		});
		newName = '';
		creating = false;
	}

	/// Rewrite the whole parameter list: it is one PERSISTED field, not a collection.
	async function setParameters(type: FixtureType, parameters: ParameterDefinition[]) {
		await data.fixture_types.byId(type.id).parameters.set(parameters);
		// channel_count follows from the parameters, so the operator never types it.
		const highest = parameters.reduce(
			(n, p) => Math.max(n, p.dmx_channel + (isColour(p.kind) ? 2 : 0)),
			0
		);
		if (highest !== type.channel_count) {
			await data.fixture_types.byId(type.id).channel_count.set(highest);
		}
	}

	const isColour = (kind: ParameterKind) => kind === 'ColorRgb';

	async function addParameter(type: FixtureType) {
		const next = type.parameters.length
			? Math.max(...type.parameters.map((p) => p.dmx_channel + (isColour(p.kind) ? 3 : 1)))
			: 1;
		await setParameters(type, [
			...type.parameters,
			{ kind: 'Intensity', dmx_channel: next, default_value: { type: 'Float', value: 0 } }
		]);
	}

	async function updateParameter(type: FixtureType, index: number, patch: Partial<ParameterDefinition>) {
		const parameters = type.parameters.map((p, i) => (i === index ? { ...p, ...patch } : p));
		// A kind change makes the old default the wrong shape.
		if (patch.kind !== undefined) {
			parameters[index].default_value = defaultValueFor(patch.kind);
		}
		await setParameters(type, parameters);
	}

	async function removeParameter(type: FixtureType, index: number) {
		await setParameters(type, type.parameters.filter((_, i) => i !== index));
	}

	onMount(() => data.fixture_types.subscribeDeep((v) => { types = v; }));
</script>

<section class="block">
	<header class="block-head">
		<h2>Fixture types</h2>
		<button class="ghost" onclick={() => (creating = !creating)}>
			{creating ? 'Cancel' : '+ Type'}
		</button>
	</header>

	{#if creating}
		<form class="new-row" onsubmit={(e) => { e.preventDefault(); createType(); }}>
			<input class="text-input" placeholder="Type name, e.g. Source Four" bind:value={newName} use:focusOnMount />
			<button class="primary" type="submit">Create</button>
		</form>
	{/if}

	{#if types.length === 0}
		<p class="empty">No fixture types yet. A fixture needs one to know what its channels do.</p>
	{/if}

	<ul class="list">
		{#each types as type (type.id)}
			<li class="row">
				<button
					class="disclosure"
					onclick={() => (expanded = expanded === type.id ? null : type.id)}
					aria-expanded={expanded === type.id}
				>
					<span class="caret" class:open={expanded === type.id}>▸</span>
					<span class="name">{type.name}</span>
					<span class="meta">{type.channel_count} ch · {type.parameters.length} params</span>
				</button>
				<button class="danger" title="Delete type" onclick={() => data.fixture_types.byId(type.id).delete()}>×</button>

				{#if expanded === type.id}
					<div class="detail">
						<label class="field">
							<span>Manufacturer</span>
							<input
								class="text-input"
								value={type.manufacturer}
								onchange={(e) => data.fixture_types.byId(type.id).manufacturer.set(e.currentTarget.value)}
							/>
						</label>

						<table class="params">
							<thead>
								<tr><th>Parameter</th><th>Channel</th><th></th></tr>
							</thead>
							<tbody>
								{#each type.parameters as param, i (i)}
									<tr>
										<td>
											<select
												class="text-input"
												value={parameterKindLabel(param.kind)}
												onchange={(e) => updateParameter(type, i, { kind: e.currentTarget.value as ParameterKind })}
											>
												{#each PARAMETER_KINDS as kind}
													<option value={kind}>{kind}</option>
												{/each}
											</select>
										</td>
										<td>
											<input
												class="text-input narrow"
												type="number"
												min="1"
												max="512"
												value={param.dmx_channel}
												onchange={(e) => updateParameter(type, i, { dmx_channel: Number(e.currentTarget.value) })}
											/>
											{#if isColour(param.kind)}<span class="hint">+2</span>{/if}
										</td>
										<td><button class="danger" onclick={() => removeParameter(type, i)}>×</button></td>
									</tr>
								{/each}
							</tbody>
						</table>
						<button class="ghost" onclick={() => addParameter(type)}>+ Parameter</button>
					</div>
				{/if}
			</li>
		{/each}
	</ul>
</section>

<style>
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.list { list-style: none; display: flex; flex-direction: column; gap: 2px; }
	.row { display: grid; grid-template-columns: 1fr auto; align-items: center; background: #202020; border: 1px solid #2e2e2e; border-radius: 4px; }
	.disclosure { display: flex; align-items: center; gap: 8px; background: none; border: none; color: inherit; font: inherit; text-align: left; padding: 8px 10px; cursor: pointer; width: 100%; }
	.caret { display: inline-block; transition: transform 0.12s; color: #777; }
	.caret.open { transform: rotate(90deg); }
	.name { font-weight: 500; }
	.meta { color: #777; font-size: 12px; margin-left: auto; }
	.detail { grid-column: 1 / -1; padding: 10px 12px 12px 28px; border-top: 1px solid #2e2e2e; display: flex; flex-direction: column; gap: 10px; }
	.field { display: flex; align-items: center; gap: 8px; }
	.field span { color: #999; font-size: 12px; min-width: 90px; }
	.params { width: 100%; border-collapse: collapse; font-size: 13px; }
	.params th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding-bottom: 4px; }
	.params td { padding: 2px 6px 2px 0; }
	.hint { color: #777; font-size: 11px; margin-left: 4px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.text-input.narrow { width: 70px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost:hover { border-color: #555; color: #fff; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
