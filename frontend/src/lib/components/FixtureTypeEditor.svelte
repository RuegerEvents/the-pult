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
		implicitChannels,
		defaultDirectionFor,
		defaultValueFor,
		formatValue,
		kindFromLabel,
		kindLabel,
		kindOption,
		PARAMETER_KINDS
	} from '$lib/patch.js';
	import { editing } from '$lib/stores/editing.js';
	import { backendOrigin } from '$lib/ws/endpoint.js';
	import { userId } from '$lib/stores/user.js';

	const backend = () => backendOrigin(window.location);
	import ValueControl from './programmer/controls/ValueControl.svelte';
	import FixtureTypeShare from './FixtureTypeShare.svelte';

	const data = getDataContext();
	// The same lock as the fixtures below it: this editor lives in the Patch panel
	// and changing a type reaches every fixture already patched against it.
	const unlocked = editing('patch');

	let types = $state<FixtureType[]>([]);
	/// What the last import said, shown until the next one or until it is dismissed.
	let report = $state<{ ok: boolean; text: string; warnings: string[] } | null>(null);
	let importing = $state(false);
	let fileInput = $state<HTMLInputElement | null>(null);
	/// Which half of this panel is showing: the show's own types, or the Share.
	let tab = $state<'types' | 'share'>('types');
	let expanded = $state<string | null>(null);
	let newName = $state('');
	let creating = $state(false);

	const portBinding = (index: number): ParameterBinding => ({ Port: { index } });
	const isColour = (kind: ParameterKind) => kind === 'ColorRgb';

	/// A binding is a *port*, or nothing at all.
	///
	/// Where a DMX channel sits is a fact about a mode rather than about a parameter,
	/// so nothing binds one: an imported type's modes say where each parameter goes,
	/// and a type made here is laid out in the order its parameters are listed. The
	/// number this panel shows against a DMX parameter is that order, read back from
	/// [`implicitChannels`], not something anybody typed.
	const portSlot = (binding: ParameterBinding | null) => (binding ? binding.Port.index : 0);

	/// A parameter with nothing but a kind: what this editor makes. It lands on the
	/// channel after the last one, because that is where the list puts it.
	const aParameter = (kind: ParameterKind): ParameterDefinition => ({
		kind,
		direction: 'Output',
		binding: null,
		default_value: defaultValueFor(kind),
		physical: null,
		slots: [],
		feature_group: null,
		emitters: []
	});

	/**
	 * Send a `.gdtf` to the station and say what it made of it.
	 *
	 * The file goes as raw bytes with the operator's id in a header, because an HTTP
	 * request carries no `Identify` the way the socket does — and an import an
	 * operator cannot undo would be the worst thing on this panel.
	 */
	async function importGdtf(file: File) {
		importing = true;
		report = null;
		try {
			const answer = await fetch(`${backend()}/api/import/gdtf`, {
				method: 'POST',
				headers: {
					'content-type': 'application/vnd.gdtf+zip',
					'x-pult-user': $userId
				},
				body: await file.arrayBuffer()
			});
			if (!answer.ok) {
				report = { ok: false, text: (await answer.text()) || answer.statusText, warnings: [] };
				return;
			}
			const body = await answer.json();
			const warnings: string[] = body.warnings ?? [];
			report = {
				ok: true,
				text: body.replaced
					? `Updated ${file.name} — every fixture patched to it follows.`
					: `Imported ${file.name}.`,
				warnings
			};
		} catch (error) {
			report = { ok: false, text: String(error), warnings: [] };
		} finally {
			importing = false;
			if (fileInput) fileInput.value = '';
		}
	}

	/// A type as a file to save. An imported one comes back as the archive it arrived
	/// in; one made here comes back as a generated file another console can open.
	function exportGdtf(type: FixtureType) {
		window.open(`${backend()}/api/export/gdtf/${type.id}`, '_blank');
	}

	/// Where an imported type came from, in words.
	function sourceLabel(type: FixtureType): string {
		if (type.source === 'Manual') return 'made here';
		if (type.source === 'Node') return 'described by its node';
		const revision = type.source.Gdtf.revision;
		return revision ? `GDTF · ${revision}` : 'GDTF';
	}

	const isImported = (type: FixtureType) =>
		typeof type.source === 'object' && 'Gdtf' in type.source;

	async function createType() {
		const name = newName.trim();
		if (!name) return;
		await data.fixture_types.create({
			id: crypto.randomUUID(),
			name,
			manufacturer: 'Generic',
			short_name: '',
			long_name: name,
			description: '',
			channel_count: 1,
			parameters: [aParameter('Intensity')],
			// A type made here names no mode: where its channels go follows from the
			// bindings above, and the station computes the layout from them. Modes are
			// what an imported GDTF file brings.
			dmx_modes: [],
			physical: {
				weight_kg: null,
				power_w: null,
				dimensions_m: null,
				connectors: [],
				leg_height_m: null,
				operating_temperature: null,
				beam_angle_deg: null
			},
			geometry: [],
			source: 'Manual'
		});
		newName = '';
		creating = false;
	}

	/// Rewrite the whole parameter list: it is one PERSISTED field, not a collection.
	async function setParameters(type: FixtureType, parameters: ParameterDefinition[]) {
		await data.fixture_types.byId(type.id).parameters.set(parameters);
		// channel_count follows from the parameters, so the operator never types it.
		// Only the ones the implicit mode places occupy channels; a port takes none.
		const channels = implicitChannels(parameters);
		const highest = parameters.reduce((n, p, i) => {
			const channel = channels[i];
			return channel === null ? n : Math.max(n, channel + (isColour(p.kind) ? 2 : 0));
		}, 0);
		if (highest !== type.channel_count) {
			await data.fixture_types.byId(type.id).channel_count.set(highest);
		}
	}

	async function addParameter(type: FixtureType) {
		await setParameters(type, [...type.parameters, aParameter('Intensity')]);
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
		const slot = slotOf(type.parameters, index);
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
	async function setBinding(type: FixtureType, index: number, binding: ParameterBinding | null) {
		const parameter = type.parameters[index];
		const slot = binding ? binding.Port.index : slotOf(type.parameters, index);
		await updateParameter(type, index, {
			binding,
			kind: kindFromLabel(kindLabel(parameter.kind), slot)
		});
	}

	/// The number this panel shows against a parameter: its port, or the channel the
	/// implicit mode lands it on.
	function slotOf(parameters: ParameterDefinition[], index: number): number {
		const parameter = parameters[index];
		if (parameter.binding) return parameter.binding.Port.index;
		return implicitChannels(parameters)[index] ?? 0;
	}

	async function removeParameter(type: FixtureType, index: number) {
		await setParameters(type, type.parameters.filter((_, i) => i !== index));
	}

	onMount(() => data.fixture_types.subscribeDeep((v) => { types = v; }));
</script>

<section class="block">
	<header class="block-head">
		<h2>Fixture types</h2>
		<nav class="tabs">
			<button class="tab" class:on={tab === 'types'} onclick={() => (tab = 'types')}>In this show</button>
			<button class="tab" class:on={tab === 'share'} onclick={() => (tab = 'share')}>From GDTF Share</button>
		</nav>
		{#if $unlocked && tab === 'types'}
			<button
				class="btn btn-ghost"
				disabled={importing}
				onclick={() => fileInput?.click()}
			>{importing ? 'Importing…' : 'Import GDTF'}</button>
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Type'}
			</button>
		{/if}
	</header>

	<!-- Hidden rather than styled: the button above is the control, and a file input
	     cannot be made to look like the rest of this panel. -->
	<input
		class="file"
		type="file"
		accept=".gdtf,application/vnd.gdtf+zip"
		bind:this={fileInput}
		onchange={(e) => {
			const file = e.currentTarget.files?.[0];
			if (file) importGdtf(file);
		}}
	/>

	{#if report}
		<div class="report" class:bad={!report.ok}>
			<p>{report.text}</p>
			{#if report.warnings.length > 0}
				<!-- Warnings, not errors: a Share file with a dangling reference is still
				     a fixture somebody has to patch tonight. -->
				<details>
					<summary>{report.warnings.length} thing{report.warnings.length === 1 ? '' : 's'} worth knowing</summary>
					<ul>
						{#each report.warnings as warning}<li>{warning}</li>{/each}
					</ul>
				</details>
			{/if}
			<button class="btn btn-ghost btn-icon" title="Dismiss" onclick={() => (report = null)}>×</button>
		</div>
	{/if}

	{#if tab === 'share'}
		<FixtureTypeShare />
	{:else}
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
					<span class="meta">
						{type.channel_count} ch · {type.parameters.length} params
						{#if isImported(type)}· {sourceLabel(type)}{/if}
					</span>
				</button>
				<button
					class="btn btn-ghost btn-icon"
					title="Export {type.name} as GDTF"
					onclick={() => exportGdtf(type)}
				>↓</button>
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
							{#if type.dmx_modes.length > 0}
							<!-- Read-only: a mode is what the manufacturer's file says, and
							     the console has no business rewriting it. Which mode a
							     given unit is in is on the fixture, in the Patch table. -->
							<div class="field wide">
								<span>Modes</span>
								<table class="modes">
									<thead><tr><th>Name</th><th>Footprint</th><th>Parameters</th></tr></thead>
									<tbody>
										{#each type.dmx_modes as mode (mode.name)}
											<tr>
												<td>{mode.name}</td>
												<td>{mode.breaks.map((b, i) => `${b} ch${type.dmx_modes.some((m) => m.breaks.length > 1) ? ` (break ${i + 1})` : ''}`).join(' + ')}</td>
												<td>{new Set(mode.channels.map((c) => c.parameter_key)).size}</td>
											</tr>
										{/each}
									</tbody>
								</table>
							</div>
						{/if}

						{#if type.physical.weight_kg !== null || type.physical.power_w !== null || type.physical.beam_angle_deg !== null}
							<div class="field wide">
								<span>Physical</span>
								<p class="reading">
									{[
										type.physical.weight_kg !== null ? `${type.physical.weight_kg} kg` : null,
										type.physical.power_w !== null ? `${type.physical.power_w} W` : null,
										type.physical.beam_angle_deg !== null ? `${type.physical.beam_angle_deg}° beam` : null,
										type.physical.operating_temperature
											? `${type.physical.operating_temperature[0]} to ${type.physical.operating_temperature[1]} °C`
											: null,
										type.physical.connectors.length > 0
											? type.physical.connectors.map((c) => c.kind).join(', ')
											: null
									]
										.filter(Boolean)
										.join(' · ')}
								</p>
							</div>
						{/if}

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
									{@const onDmx = param.binding === null}
									{@const slot = slotOf(type.parameters, i)}
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
														e.currentTarget.value === 'Dmx' ? null : portBinding(portSlot(param.binding))
													)}
											>
												<option value="Dmx">DMX channel</option>
												<option value="Port">Module port</option>
											</select>
											{/if}
										</td>
										<td>
											<!-- A DMX channel is read rather than typed: where a parameter
											     sits belongs to a mode, and a type made here has the implicit
											     one, which lays its parameters out in the order they are
											     listed. A port is the operator's to choose. -->
											{#if !$unlocked || onDmx}
												<span class="reading">{slot}</span>
											{:else}
											<input
												class="input narrow"
												type="number"
												min="0"
												max="255"
												value={slot}
												onchange={(e) =>
													setBinding(type, i, portBinding(Number(e.currentTarget.value)))}
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
	{/if}
</section>

<style>
	.tabs { display: flex; gap: 2px; margin-left: auto; }
	.tab {
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: #999;
		font: inherit;
		font-size: 12px;
		padding: 2px 8px 3px;
		cursor: pointer;
	}
	.tab.on { color: #ddd; border-bottom-color: var(--accent, #4a9eff); }

	/* The button in the header is the control; the input itself is never seen. */
	.file { display: none; }

	.report {
		display: grid;
		grid-template-columns: 1fr auto;
		align-items: start;
		gap: 8px;
		margin: 8px 0;
		padding: 8px 12px;
		border-radius: 4px;
		background: #1c1c1c;
		border-left: 3px solid #4caf50;
		font-size: 12px;
	}
	.report.bad { border-left-color: #e5534b; }
	.report p { margin: 0; }
	.report ul { margin: 4px 0 0; padding-left: 18px; color: #999; }
	.report summary { cursor: pointer; color: #999; }

	/* A table does not fit beside a 90px label, so a wide field stacks. */
	.field.wide { align-items: flex-start; flex-direction: column; gap: 4px; }

	.modes { width: 100%; border-collapse: collapse; font-size: 12px; }
	.modes th,
	.modes td { text-align: left; padding: 3px 8px 3px 0; border-bottom: 1px solid #2e2e2e; }
	.modes th { font-weight: 500; color: #999; }

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
