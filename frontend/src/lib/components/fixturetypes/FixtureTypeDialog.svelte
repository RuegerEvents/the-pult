<script lang="ts">
	/**
	 * One fixture type, edited on top of the list rather than unfolded inside it.
	 *
	 * Unfolded, a type with forty parameters pushed every other type off the panel and
	 * the list stopped being a list. A dialog keeps the list still behind it and gives
	 * the editing the room it needs, in three tabs for the three questions a type
	 * answers: what can be set (Parameters), where the bytes go (Modes), and what a
	 * color is made of (Color).
	 *
	 * **The emitter list is the layout.** On a type with no modes a color takes a byte
	 * per emitter (`implicitWidth`), so picking RGBW here is what gives a hand-made head
	 * its fourth channel, and `channel_count` is rewritten in the same breath. A type
	 * read from a GDTF file or described by a node is not this panel's to rewrite, so
	 * its emitters are shown and not offered.
	 */

	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import Palette from '@lucide/svelte/icons/palette';
	import Plus from '@lucide/svelte/icons/plus';
	import Trash from '@lucide/svelte/icons/trash-2';

	import type {
		Emitter,
		EmitterPreset,
		FixtureType,
		ParameterBinding,
		ParameterDefinition,
		ParameterDirection,
		ParameterKind
	} from '$lib/generated/index.js';
	import { anEmitter, copied, hexOf, namesOf, presetOf, vecOf } from '$lib/emitters.js';
	import {
		defaultDirectionFor,
		defaultValueFor,
		displayLabel,
		formatValue,
		implicitChannels,
		implicitWidth,
		kindFromLabel,
		kindLabel,
		kindOption,
		PARAMETER_KINDS
	} from '$lib/patch.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { showClient } from '$lib/stores/show.js';
	import { getDataContext } from '$lib/ws/context.js';
	import Dialog from '../Dialog.svelte';
	import Help from '../Help.svelte';
	import ValueControl from '../programmer/controls/ValueControl.svelte';

	let { type, onclose }: { type: FixtureType; onclose: () => void } = $props();

	const data = getDataContext();
	let tab = $state<'parameters' | 'modes' | 'color'>('parameters');

	const portBinding = (index: number): ParameterBinding => ({ Port: { index } });
	const isColor = (p: ParameterDefinition) => p.default_value.type === 'Color';

	/// A binding is a *port*, or nothing: where a DMX channel sits belongs to a mode.
	const portSlot = (binding: ParameterBinding | null) => (binding ? binding.Port.index : 0);

	const source = $derived(
		type.source === 'Manual' ? 'manual' : type.source === 'Node' ? 'node' : ('gdtf' as const)
	);
	const colors = $derived(type.parameters.map((p, i) => ({ p, i })).filter(({ p }) => isColor(p)));

	/// The presets, asked for once per dialog: a read, and a short one.
	let presets = $state<EmitterPreset[]>([]);
	$effect(() => {
		showClient()
			.call('fixtureTypes.library', {})
			.then((answer) => (presets = (answer as { emitterPresets: EmitterPreset[] }).emitterPresets ?? []))
			.catch(() => (presets = []));
	});
	/// Which color parameters are showing their emitter list for editing.
	let custom = $state<Set<number>>(new Set());

	/**
	 * Rewrite the whole parameter list: it is one PERSISTED field, not a collection.
	 * `channel_count` follows from it — the highest byte the implicit mode places, a
	 * color counting one per emitter — so the operator never types it.
	 */
	async function setParameters(parameters: ParameterDefinition[]) {
		const channels = implicitChannels(parameters);
		const highest = parameters.reduce((n, p, i) => {
			const channel = channels[i];
			return channel === null ? n : Math.max(n, channel + implicitWidth(p) - 1);
		}, 0);
		// One gesture, so RGB to RGBW is one Ctrl-Z rather than a list and a count.
		beginGesture();
		try {
			await data.fixture_types.byId(type.id).parameters.set(parameters);
			if (highest !== type.channel_count) await data.fixture_types.byId(type.id).channel_count.set(highest);
		} finally {
			endGesture();
		}
	}

	const aParameter = (kind: ParameterKind): ParameterDefinition => ({
		kind,
		direction: 'Output',
		binding: null,
		default_value: defaultValueFor(kind),
		physical: null,
		slots: [],
		feature_group: null,
		emitters: [],
		per_cell: false
	});

	async function updateParameter(index: number, patch: Partial<ParameterDefinition>) {
		const parameters = type.parameters.map((p, i) => (i === index ? { ...p, ...patch } : p));
		// A kind change makes the old default the wrong shape, and usually means the
		// parameter flows the other way — a contact is read, a relay is driven.
		if (patch.kind !== undefined) {
			parameters[index].default_value = defaultValueFor(patch.kind);
			parameters[index].direction = defaultDirectionFor(patch.kind);
			if (!isColor(parameters[index])) parameters[index].emitters = [];
		}
		await setParameters(parameters);
	}

	/// The number shown against a parameter: its port, or the channel the implicit mode
	/// lands it on.
	function slotOf(parameters: ParameterDefinition[], index: number): number {
		const parameter = parameters[index];
		if (parameter.binding) return parameter.binding.Port.index;
		return implicitChannels(parameters)[index] ?? 0;
	}

	/// Choosing a kind by name. `Switch`, `Contact` and `Raw` are numbered after the
	/// channel or port they sit on, so the number is never typed twice.
	async function setKind(index: number, label: string) {
		const parameter = type.parameters[index];
		await updateParameter(index, { kind: kindFromLabel(label, slotOf(type.parameters, index), kindLabel(parameter.kind)) });
	}

	/// The name is the whole identity of a named parameter, so an empty one is refused.
	async function renameParameter(index: number, name: string) {
		if (!name.trim()) return;
		await updateParameter(index, { kind: { Named: name.trim() } });
	}

	/// Moving a parameter between a DMX channel and a module port. The numbered kinds
	/// follow the port, so re-binding one renames it too.
	async function setBinding(index: number, binding: ParameterBinding | null) {
		const parameter = type.parameters[index];
		const slot = binding ? binding.Port.index : slotOf(type.parameters, index);
		await updateParameter(index, { binding, kind: kindFromLabel(kindLabel(parameter.kind), slot) });
	}

	const setEmitters = (index: number, emitters: Emitter[]) => updateParameter(index, { emitters: copied(emitters) });

	function editEmitter(index: number, at: number, patch: Partial<Emitter>) {
		const current = copied(type.parameters[index].emitters.length ? type.parameters[index].emitters : rgbFor());
		current[at] = { ...current[at], ...patch };
		void setEmitters(index, current);
	}

	/// The plain three, spelled out, for a color that named none and is now being edited.
	const rgbFor = (): Emitter[] => presets.find((p) => p.name === 'RGB')?.emitters ?? [];

	const modeFootprint = (breaks: number[], multi: boolean) =>
		breaks.map((b, i) => `${b} ch${multi ? ` (break ${i + 1})` : ''}`).join(' + ');
</script>

<Dialog title={type.name} size="large" {onclose}>
	{#snippet actions()}
		<span class="badge {source}">{source === 'manual' ? 'Made here' : source === 'node' ? 'From its node' : 'GDTF'}</span>
	{/snippet}
	<div class="editor">
		<div class="fields head">
			<span class="label">Name</span>
			{#if $canEdit}
				<input class="input" value={type.name} onchange={(e) => data.fixture_types.byId(type.id).name.set(e.currentTarget.value)} />
			{:else}
				<span>{type.name}</span>
			{/if}
			<span class="label">Manufacturer</span>
			{#if $canEdit}
				<input
					class="input"
					value={type.manufacturer}
					onchange={(e) => data.fixture_types.byId(type.id).manufacturer.set(e.currentTarget.value)}
				/>
			{:else}
				<span>{type.manufacturer}</span>
			{/if}
			{#if type.physical.weight_kg !== null || type.physical.power_w !== null || type.physical.beam_angle_deg !== null}
				<span class="label">Physical</span>
				<span class="dim">
					{[
						type.physical.weight_kg !== null ? `${type.physical.weight_kg} kg` : null,
						type.physical.power_w !== null ? `${type.physical.power_w} W` : null,
						type.physical.beam_angle_deg !== null ? `${type.physical.beam_angle_deg}° beam` : null,
						type.physical.operating_temperature
							? `${type.physical.operating_temperature[0]} to ${type.physical.operating_temperature[1]} °C`
							: null,
						type.physical.connectors.length > 0 ? type.physical.connectors.map((c) => c.kind).join(', ') : null
					]
						.filter(Boolean)
						.join(' · ')}
				</span>
			{/if}
		</div>

		<div class="segmented tabs" role="tablist">
			<button role="tab" aria-selected={tab === 'parameters'} class:on={tab === 'parameters'} onclick={() => (tab = 'parameters')}
				>Parameters · {type.parameters.length}</button
			>
			<button role="tab" aria-selected={tab === 'modes'} class:on={tab === 'modes'} onclick={() => (tab = 'modes')}
				>Modes · {Math.max(1, type.dmx_modes.length)}</button
			>
			<button role="tab" aria-selected={tab === 'color'} class:on={tab === 'color'} onclick={() => (tab = 'color')}
				>Color · {colors.length}</button
			>
		</div>

		{#if tab === 'parameters'}
			<table class="tbl">
				<thead>
					<tr>
						<th>Parameter</th><th>Flow</th><th>On</th><th
							>Slot <Help label="Where a parameter sits"
								>A DMX parameter's channel is read, not typed: a type made here lays its parameters out in
								the order they are listed, one byte each and one per emitter for a color. A module port is
								yours to choose.</Help
							></th
						><th
							>Default <Help label="What the default is"
								>Where the parameter rests before anything drives it — read by the outputs for a fixture
								nobody has touched, so a moving head can rest centered rather than hard left.</Help
							></th
						><th></th>
					</tr>
				</thead>
				<tbody>
					{#each type.parameters as param, i (i)}
						{@const onDmx = param.binding === null}
						{@const slot = slotOf(type.parameters, i)}
						<tr>
							<td class="kind">
								{#if !$canEdit}
									<span>{kindLabel(param.kind)}</span>
								{:else}
									<select class="select" value={kindOption(param.kind)} onchange={(e) => setKind(i, e.currentTarget.value)}>
										{#each PARAMETER_KINDS as kind (kind)}
											<option value={kind}>{kind}</option>
										{/each}
									</select>
									{#if typeof param.kind === 'object' && 'Named' in param.kind}
										<input
											class="input"
											value={param.kind.Named}
											placeholder="what the device calls it"
											onchange={(e) => renameParameter(i, e.currentTarget.value)}
										/>
									{/if}
								{/if}
							</td>
							<td>
								{#if !$canEdit}
									<span>{param.direction === 'Input' ? 'Read' : 'Driven'}</span>
								{:else}
									<select
										class="select"
										value={param.direction}
										onchange={(e) => updateParameter(i, { direction: e.currentTarget.value as ParameterDirection })}
									>
										<option value="Output">Driven</option>
										<option value="Input">Read</option>
									</select>
								{/if}
							</td>
							<td>
								{#if !$canEdit}
									<span>{onDmx ? 'DMX' : 'Port'}</span>
								{:else}
									<select
										class="select"
										value={onDmx ? 'Dmx' : 'Port'}
										onchange={(e) => setBinding(i, e.currentTarget.value === 'Dmx' ? null : portBinding(portSlot(param.binding)))}
									>
										<option value="Dmx">DMX channel</option>
										<option value="Port">Module port</option>
									</select>
								{/if}
							</td>
							<td>
								{#if !$canEdit || onDmx}
									<span class="mono">{slot}</span>
								{:else}
									<input
										class="input narrow"
										type="number"
										min="0"
										max="255"
										value={slot}
										onchange={(e) => setBinding(i, portBinding(Number(e.currentTarget.value)))}
									/>
								{/if}
								{#if onDmx && isColor(param) && implicitWidth(param) > 1}<span class="dim">+{implicitWidth(param) - 1}</span>{/if}
							</td>
							<td class="default-cell">
								{#if $canEdit}
									<ValueControl value={param.default_value} label="Default" oninput={(v) => updateParameter(i, { default_value: v })} />
								{:else}
									<span>{formatValue(param.default_value)}</span>
								{/if}
							</td>
							<td class="end">
								{#if $canEdit}
									<button
										class="btn btn-ghost btn-icon"
										title="Remove {kindLabel(param.kind)}"
										aria-label="Remove {kindLabel(param.kind)}"
										onclick={() => setParameters(type.parameters.filter((_, j) => j !== i))}><Trash size={14} /></button
									>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
			{#if $canEdit}
				<button class="btn btn-ghost add" onclick={() => setParameters([...type.parameters, aParameter('Intensity')])}
					><Plus size={14} />Parameter</button
				>
			{/if}
		{:else if tab === 'modes'}
			{#if type.dmx_modes.length > 0}
				{@const multi = type.dmx_modes.some((m) => m.breaks.length > 1)}
				<p class="lead">
					From the file <Help label="Why modes are read-only"
						>A mode is what the manufacturer's file says, and the console does not rewrite it. Which mode a
						given unit is in is on the fixture, in the Patch.</Help
					>
				</p>
				<table class="tbl">
					<thead><tr><th>Name</th><th>Footprint</th><th>Parameters</th></tr></thead>
					<tbody>
						{#each type.dmx_modes as mode (mode.name)}
							<tr>
								<td>{mode.name}</td>
								<td class="mono">{modeFootprint(mode.breaks, multi)}</td>
								<td>{new Set(mode.channels.map((c) => c.parameter_key)).size}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{:else}
				{@const channels = implicitChannels(type.parameters)}
				<p class="lead">
					Default · {type.channel_count} ch <Help label="Where the Default mode comes from"
						>A type made here names no mode, so it has one worked out from its parameters: a byte each in
						the order they are listed, one per emitter for a color, and none for a parameter on a module
						port. Reorder or change the parameters and this follows.</Help
					>
				</p>
				<table class="tbl">
					<thead><tr><th>Channel</th><th>Parameter</th></tr></thead>
					<tbody>
						{#each type.parameters as param, i (i)}
							{#if channels[i] !== null}
								{@const width = implicitWidth(param)}
								<tr>
									<td class="mono">{width > 1 ? `${channels[i]}–${(channels[i] ?? 0) + width - 1}` : channels[i]}</td>
									<td>
										{displayLabel(param.kind)}
										{#if isColor(param)}<span class="dim">· {namesOf(param).join(', ')}</span>{/if}
									</td>
								</tr>
							{/if}
						{/each}
					</tbody>
				</table>
			{/if}
		{:else}
			{#if colors.length === 0}
				<p class="empty">This type has no color parameter. Add one under Parameters to give it emitters.</p>
			{/if}
			{#each colors as { p, i } (i)}
				{@const chosen = presetOf(p, presets)}
				{@const editable = $canEdit && source === 'manual'}
				{@const shown = p.emitters.length ? p.emitters : rgbFor()}
				<section class="color">
					<h3>
						<Palette size={14} />{displayLabel(p.kind)}
						<span class="dim">· {namesOf(p).length} emitter{namesOf(p).length === 1 ? '' : 's'}</span>
						{#if source === 'gdtf'}
							<Help label="Why these cannot be changed"
								>These come from the GDTF file, measured by whoever wrote it. The file is kept whole, so
								the console reads them rather than rewriting them; a per-emitter override on a color is how
								a show corrects one.</Help
							>
						{:else if source === 'node'}
							<Help label="Why these cannot be changed"
								>This type is built from what its OpenHaunt node says about itself, and rebuilt whenever
								the node describes itself again — an edit here would be put back.</Help
							>
						{:else}
							<Help label="What emitters are"
								>The light sources a color mixes across. One byte each on the wire, in this order: RGBW is
								four channels, and the console works out the white from the color you ask for.</Help
							>
						{/if}
					</h3>
					{#if editable}
						<div class="chips">
							{#each presets as preset (preset.name)}
								<button
									class="chip"
									class:on={chosen?.name === preset.name && !custom.has(i)}
									onclick={() => {
										custom.delete(i);
										custom = new Set(custom);
										void setEmitters(i, preset.emitters);
									}}>{preset.name}</button
								>
							{/each}
							<button
								class="chip"
								class:on={custom.has(i) || (!chosen && presets.length > 0)}
								onclick={() => (custom = new Set(custom).add(i))}>Custom…</button
							>
						</div>
					{/if}
					{#if editable && (custom.has(i) || (!chosen && presets.length > 0))}
						<table class="tbl emitters">
							<thead><tr><th>Byte</th><th>Name</th><th>Color</th><th>Subtractive</th><th></th></tr></thead>
							<tbody>
								{#each shown as emitter, at (at)}
									<tr>
										<td class="mono">{at + 1}</td>
										<td>
											<input
												class="input"
												value={emitter.name}
												onchange={(e) => e.currentTarget.value.trim() && editEmitter(i, at, { name: e.currentTarget.value.trim() })}
											/>
										</td>
										<td>
											<input
												type="color"
												class="swatch-input"
												value={hexOf(emitter.rgb)}
												onchange={(e) => editEmitter(i, at, { rgb: vecOf(e.currentTarget.value) })}
											/>
										</td>
										<td>
											<input
												type="checkbox"
												checked={emitter.subtractive}
												title="A flag that takes light out (CMY) rather than a die that makes it"
												onchange={(e) => editEmitter(i, at, { subtractive: e.currentTarget.checked })}
											/>
										</td>
										<td class="end">
											<button
												class="btn btn-ghost btn-icon"
												title="Remove {emitter.name}"
												aria-label="Remove {emitter.name}"
												disabled={shown.length <= 1}
												onclick={() => setEmitters(i, shown.filter((_, j) => j !== at))}><Trash size={14} /></button
											>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
						<button class="btn btn-ghost add" onclick={() => setEmitters(i, [...shown, anEmitter(shown.map((e) => e.name))])}
							><Plus size={14} />Emitter</button
						>
					{:else}
						<div class="dies">
							{#each shown.length ? shown : namesOf(p).map((name) => ({ name, rgb: null, subtractive: false }) as Emitter) as emitter, at (at)}
								<span class="die" title={emitter.subtractive ? 'Subtractive' : ''}>
									<i class="dot" class:sub={emitter.subtractive} style:background={hexOf(emitter.rgb)}></i>{emitter.name}
								</span>
							{/each}
						</div>
					{/if}
				</section>
			{/each}
		{/if}
	</div>
	{#snippet footer()}
		<button class="btn btn-primary" onclick={onclose}>Done</button>
	{/snippet}
</Dialog>

<style>
	.editor {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 14px 16px 16px;
		min-height: 0;
	}
	.head .input {
		max-width: 360px;
	}
	.dim {
		color: var(--text-faint);
	}
	.mono {
		font-variant-numeric: tabular-nums;
		font-family: var(--font-mono, monospace);
	}
	.badge {
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 2px 7px;
		border-radius: 9px;
		background: var(--bg-raised);
		color: var(--text-dim);
	}
	.badge.gdtf {
		color: var(--accent);
	}
	.tabs {
		align-self: flex-start;
	}
	.tbl {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	.tbl th {
		text-align: left;
		font-weight: 500;
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint);
		padding: 0 10px 6px 0;
		white-space: nowrap;
	}
	.tbl td {
		padding: 6px 10px 6px 0;
		border-top: 1px solid var(--line);
		vertical-align: middle;
	}
	.tbl .end {
		text-align: right;
		padding-right: 0;
	}
	.kind {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.default-cell {
		min-width: 9rem;
	}
	.input.narrow {
		width: 5rem;
	}
	.add {
		align-self: flex-start;
		display: inline-flex;
		gap: 4px;
		align-items: center;
	}
	.lead {
		margin: 0;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		margin: 0;
	}
	.color {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 12px 0;
		border-top: 1px solid var(--line);
	}
	.color:first-of-type {
		border-top: none;
		padding-top: 0;
	}
	h3 {
		display: flex;
		align-items: center;
		gap: 6px;
		margin: 0;
		font-size: var(--font-sm);
		font-weight: 600;
	}
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.chip {
		padding: 4px 10px;
		border: 1px solid var(--line-strong);
		border-radius: 14px;
		background: var(--bg-raised);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.chip:hover {
		color: var(--text);
	}
	.chip.on {
		color: var(--accent);
		border-color: var(--accent-solid, var(--accent));
	}
	.dies {
		display: flex;
		flex-wrap: wrap;
		gap: 12px;
		font-size: var(--font-sm);
	}
	.die {
		display: inline-flex;
		align-items: center;
		gap: 5px;
	}
	.dot {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		border: 1px solid rgb(255 255 255 / 0.2);
	}
	.dot.sub {
		border-radius: 2px;
	}
	.swatch-input {
		width: 34px;
		height: 24px;
		padding: 0;
		border: 1px solid var(--line-strong);
		border-radius: 4px;
		background: none;
		cursor: pointer;
	}
	.emitters .input {
		max-width: 180px;
	}
</style>
