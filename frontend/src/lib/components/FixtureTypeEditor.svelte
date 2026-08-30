<script lang="ts">
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type {
		FixtureType,
		ParameterBinding,
		ParameterDefinition,
		ParameterDirection,
		ParameterKind
	} from '$lib/generated/index.js';
	import {
		bindingChannel,
		defaultDirectionFor,
		defaultValueFor,
		formatValue,
		kindFromLabel,
		kindLabel,
		kindOption,
		PARAMETER_KINDS
	} from '$lib/patch.js';
	import { editing } from '$lib/stores/editing.js';
	import ValueControl from './programmer/controls/ValueControl.svelte';

	const data = getDataContext();
	// The same lock as the fixtures below it: this editor lives in the Patch panel
	// and changing a type reaches every fixture already patched against it.
	const unlocked = editing('patch');

	let types = $state<FixtureType[]>([]);
	let expanded = $state<string | null>(null);
	let newName = $state('');
	let creating = $state(false);

	const dmxBinding = (channel: number): ParameterBinding => ({ Dmx: { channel } });
	const portBinding = (index: number): ParameterBinding => ({ Port: { index } });
	const isColour = (kind: ParameterKind) => kind === 'ColorRgb';

	/// Whichever number a binding carries — a DMX channel or a port index.
	const bindingSlot = (binding: ParameterBinding) =>
		'Dmx' in binding ? binding.Dmx.channel : binding.Port.index;

	async function createType() {
		const name = newName.trim();
		if (!name) return;
		await data.fixture_types.create({
			id: crypto.randomUUID(),
			name,
			manufacturer: 'Generic',
			channel_count: 1,
			parameters: [
				{
					kind: 'Intensity',
					direction: 'Output',
					binding: dmxBinding(1),
					default_value: { type: 'Float', value: 0 }
				}
			]
		});
		newName = '';
		creating = false;
	}

	/// Rewrite the whole parameter list: it is one PERSISTED field, not a collection.
	async function setParameters(type: FixtureType, parameters: ParameterDefinition[]) {
		await data.fixture_types.byId(type.id).parameters.set(parameters);
		// channel_count follows from the parameters, so the operator never types it.
		// Only DMX bindings occupy channels; a port takes none.
		const highest = parameters.reduce((n, p) => {
			const channel = bindingChannel(p.binding);
			return channel === null ? n : Math.max(n, channel + (isColour(p.kind) ? 2 : 0));
		}, 0);
		if (highest !== type.channel_count) {
			await data.fixture_types.byId(type.id).channel_count.set(highest);
		}
	}

	async function addParameter(type: FixtureType) {
		// The next free DMX channel. Ports take none, so they do not move it.
		const next = type.parameters.reduce((n, p) => {
			const channel = bindingChannel(p.binding);
			return channel === null ? n : Math.max(n, channel + (isColour(p.kind) ? 3 : 1));
		}, 1);
		await setParameters(type, [
			...type.parameters,
			{
				kind: 'Intensity',
				direction: 'Output',
				binding: dmxBinding(next),
				default_value: { type: 'Float', value: 0 }
			}
		]);
	}

	async function updateParameter(
		type: FixtureType,
		index: number,
		patch: Partial<ParameterDefinition>
	) {
		const parameters = type.parameters.map((p, i) => (i === index ? { ...p, ...patch } : p));
		// A kind change makes the old default the wrong shape, and usually means the
		// parameter flows the other way — a contact is read, a relay is driven.
		if (patch.kind !== undefined) {
			parameters[index].default_value = defaultValueFor(patch.kind);
			parameters[index].direction = defaultDirectionFor(patch.kind);
		}
		await setParameters(type, parameters);
	}

	/// Choosing a kind by name. `Switch`, `Contact` and `Raw` are numbered after the
	/// channel or port they sit on, so the number is never typed twice.
	async function setKind(type: FixtureType, index: number, label: string) {
		const parameter = type.parameters[index];
		const slot = bindingSlot(parameter.binding);
		await updateParameter(type, index, {
			// A parameter changing *into* a named one keeps whatever it was called if
			// it already had a name, and otherwise gets a placeholder to type over.
			kind: kindFromLabel(label, slot, kindLabel(parameter.kind))
		});
	}

	/// Renaming a named parameter. The name is the whole identity of one — it is what
	/// the operator sees and what its `live_values` key is built from — so an empty
	/// one is refused rather than written.
	async function renameParameter(type: FixtureType, index: number, name: string) {
		if (!name.trim()) return;
		await updateParameter(type, index, { kind: { Named: name.trim() } });
	}

	/// Moving a parameter between a DMX channel and a module port. The numbered kinds
	/// follow the port, so re-binding one renames it too.
	async function setBinding(type: FixtureType, index: number, binding: ParameterBinding) {
		const parameter = type.parameters[index];
		await updateParameter(type, index, {
			binding,
			kind: kindFromLabel(kindLabel(parameter.kind), bindingSlot(binding))
		});
	}

	async function removeParameter(type: FixtureType, index: number) {
		await setParameters(type, type.parameters.filter((_, i) => i !== index));
	}

	onMount(() => data.fixture_types.subscribeDeep((v) => { types = v; }));
</script>

<section class="block">
	<header class="block-head">
		<h2>Fixture types</h2>
		{#if $unlocked}
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Type'}
			</button>
		{/if}
	</header>

	{#if creating && $unlocked}
		<form class="new-row" onsubmit={(e) => { e.preventDefault(); createType(); }}>
			<input class="input" placeholder="Type name, e.g. Source Four" bind:value={newName} use:focusOnMount />
			<button class="btn btn-primary" type="submit">Create</button>
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
				{#if $unlocked}
					<button
						class="btn btn-danger btn-icon"
						title="Delete {type.name}"
						onclick={() => data.fixture_types.byId(type.id).delete()}
					>×</button>
				{:else}
					<span></span>
				{/if}

				{#if expanded === type.id}
					<div class="detail">
						<label class="field">
							<span>Name</span>
							{#if $unlocked}
								<input
									class="input"
									value={type.name}
									onchange={(e) => data.fixture_types.byId(type.id).name.set(e.currentTarget.value)}
								/>
							{:else}
								<span class="reading">{type.name}</span>
							{/if}
						</label>
						<label class="field">
							<span>Manufacturer</span>
							{#if $unlocked}
								<input
									class="input"
									value={type.manufacturer}
									onchange={(e) => data.fixture_types.byId(type.id).manufacturer.set(e.currentTarget.value)}
								/>
							{:else}
								<span class="reading">{type.manufacturer}</span>
							{/if}
						</label>

						<table class="params">
							<thead>
								<tr><th>Parameter</th><th>Flow</th><th>On</th><th>Slot</th><th>Default</th><th></th></tr>
							</thead>
							<tbody>
								{#each type.parameters as param, i (i)}
									{@const onDmx = bindingChannel(param.binding) !== null}
									<tr>
										<td class="kind">
											{#if !$unlocked}
												<span class="reading">{kindLabel(param.kind)}</span>
											{:else}
												<select
													class="select"
													value={kindOption(param.kind)}
													onchange={(e) => setKind(type, i, e.currentTarget.value)}
												>
													{#each PARAMETER_KINDS as kind}
														<option value={kind}>{kind}</option>
													{/each}
												</select>
												{#if typeof param.kind === 'object' && 'Named' in param.kind}
													<!-- The name is the whole identity of a named parameter, so
													     it is typed here rather than inferred. A device that
													     named its own port supplies this on adoption; a type
													     built by hand needs somewhere to say it. -->
													<input
														class="input"
														value={param.kind.Named}
														placeholder="what the device calls it"
														onchange={(e) => renameParameter(type, i, e.currentTarget.value)}
													/>
												{/if}
											{/if}
										</td>
										<td>
											{#if !$unlocked}
												<span class="reading">{param.direction === 'Input' ? 'Read' : 'Driven'}</span>
											{:else}
											<select
												class="select"
												value={param.direction}
												onchange={(e) =>
													updateParameter(type, i, {
														direction: e.currentTarget.value as ParameterDirection
													})}
											>
												<option value="Output">Driven</option>
												<option value="Input">Read</option>
											</select>
											{/if}
										</td>
										<td>
											{#if !$unlocked}
												<span class="reading">{onDmx ? 'DMX' : 'Port'}</span>
											{:else}
											<select
												class="select"
												value={onDmx ? 'Dmx' : 'Port'}
												onchange={(e) =>
													setBinding(
														type,
														i,
														e.currentTarget.value === 'Dmx'
															? dmxBinding(bindingSlot(param.binding) || 1)
															: portBinding(bindingSlot(param.binding))
													)}
											>
												<option value="Dmx">DMX channel</option>
												<option value="Port">Module port</option>
											</select>
											{/if}
										</td>
										<td>
											{#if !$unlocked}
												<span class="reading">{bindingSlot(param.binding)}</span>
											{:else}
											<input
												class="input narrow"
												type="number"
												min={onDmx ? 1 : 0}
												max={onDmx ? 512 : 255}
												value={bindingSlot(param.binding)}
												onchange={(e) =>
													setBinding(
														type,
														i,
														onDmx
															? dmxBinding(Number(e.currentTarget.value))
															: portBinding(Number(e.currentTarget.value))
													)}
											/>
											{/if}
											{#if onDmx && isColour(param.kind)}<span class="hint">+2</span>{/if}
										</td>
										<td class="default-cell">
											<!-- Where this parameter sits before anything drives it. Read by
											     the output plugins for a fixture that has never been touched,
											     so a moving head can rest centred rather than hard left. -->
											{#if $unlocked}
												<ValueControl
													value={param.default_value}
													label="Default"
													oninput={(v) => updateParameter(type, i, { default_value: v })}
												/>
											{:else}
												<span class="reading">{formatValue(param.default_value)}</span>
											{/if}
										</td>
										<td>
											{#if $unlocked}
												<button class="btn btn-danger btn-icon" onclick={() => removeParameter(type, i)}>×</button>
											{/if}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
						{#if $unlocked}
							<button class="btn btn-ghost" onclick={() => addParameter(type)}>+ Parameter</button>
						{/if}
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
	.disclosure { display: flex; align-items: center; gap: 8px; background: none; border: none; color: inherit; font: inherit; text-align: left; padding: 8px 10px; cursor: pointer; width: 100%; min-height: var(--hit); }
	.caret { display: inline-block; transition: transform 0.12s; color: #777; }
	.caret.open { transform: rotate(90deg); }
	.name { font-weight: 500; }
	.meta { color: #777; font-size: 12px; margin-left: auto; }
	.detail { grid-column: 1 / -1; padding: 10px 12px 12px 28px; border-top: 1px solid #2e2e2e; display: flex; flex-direction: column; gap: 10px; }
	.field { display: flex; align-items: center; gap: 8px; }
	.field span { color: #999; font-size: 12px; min-width: 90px; }
	.params { width: 100%; border-collapse: collapse; font-size: 13px; }
	.params th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding-bottom: 4px; }
	.params td { padding: 6px 6px 6px 0; height: var(--hit); vertical-align: middle; }
	.hint { color: #777; font-size: 11px; margin-left: 4px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	/* Buttons and inputs come from `styles/controls.css`. */
	.input.narrow { width: 5rem; }
	.kind { display: flex; align-items: center; gap: 6px; }
	.default-cell { min-width: 9rem; }
	/* What a field shows when the panel is locked: the value, plainly, in the space
	   the control would have taken, so unlocking does not reflow the table. */
	.reading { color: var(--text); padding: 0 2px; }
</style>
