<script lang="ts">
	/**
	 * Building an effect for the selection: from the pool, by hand, or for a strip.
	 *
	 * This panel *is* an editor, and everything it applies goes to the programmer,
	 * which is the scratch buffer. Nothing reaches the show until it is
	 * stored into a cue — except the pool itself, which is show data and changes only
	 * when somebody presses Store or Update.
	 *
	 * The picture is the interface. The preview row is the selection, in its own order,
	 * evaluated by the same arithmetic the lamps take; the waveform takes drags for
	 * rate, width and levels; the handle over the preview is the spread. The numbers are
	 * still there, folded away, for when a value has to be exact.
	 *
	 * What is being built is a *template*, one layer per parameter, so fire is
	 * intensity and color together and Apply is one act over both.
	 */

	import type {
		EffectLayer,
		EffectTemplate,
		Fixture,
		FixtureType,
		ParameterKind,
		ParameterValue,
		Shape,
		Spread
	} from '$lib/generated/index.js';
	import { defaultSpec, shapesFor, SPREADS, spreadLabel } from '$lib/effects.js';
	import {
		gradientCurve,
		gridCurve,
		gridLevels,
		pixelSpread,
		readPixelSpread,
		splitTarget,
		targetIds,
		templateApply
	} from '$lib/effectPool.js';
	import { editableParameters, hexToRgb, rgbToHex } from '$lib/programmer.js';
	import { displayLabel, hasCells, parameterKey } from '$lib/patch.js';
	import { effectiveHz } from '$lib/speedmasters.js';
	import {
		applyLayers,
		deleteTemplate,
		storeTemplate,
		templates,
		updateTemplate
	} from '$lib/stores/effectPool.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { byKey, entries, setValue } from '$lib/stores/programmer.js';
	import { elements, selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import ValueControl from '../programmer/controls/ValueControl.svelte';
	import Card from '../objects/Card.svelte';
	import { openTemplate } from '$lib/kinds/effects.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import EffectPreview from './EffectPreview.svelte';
	import EffectWave from './EffectWave.svelte';
	import StepGrid from './StepGrid.svelte';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const masters = collection('speed_masters');

	const fixtureById = $derived(new Map<string, Fixture>($fixtures.map((f) => [f.id, f])));
	const typeById = $derived(new Map<string, FixtureType>($types.map((t) => [t.id, t])));
	/**
	 * What the selection acts on, in its own order: each fixture, and a strip's cells
	 * along it — one target each, so a chase spreads across pixels and pars alike.
	 */
	const order = $derived(targetIds($elements.filter((e) => fixtureById.has(e.fixture))));
	const picked = $derived(
		$selection.filter((id) => fixtureById.has(id)).map((id) => fixtureById.get(id)!)
	);
	const shared = $derived(editableParameters($types, picked));

	/** Every selected fixture has cells: a strip the console draws, or a bar. */
	const pixels = $derived(
		picked.length > 0 && picked.every((f) => hasCells(typeById.get(f.fixture_type_id)))
	);
	/**
	 * Several multi-cell fixtures: one spread along all their cells end to end, or
	 * restarting at each fixture so they run in step. Not stored with a template,
	 * because it is a choice about this selection rather than about the effect.
	 */
	let eachFixture = $state(false);
	const multiCell = $derived(picked.filter((f) => hasCells(typeById.get(f.fixture_type_id))).length > 1);
	/** Every selected fixture is a strip the node draws itself. */
	const nodeStrips = $derived(
		picked.length > 0 &&
			picked.every(
				(f) => 'OpenHaunt' in f.address && f.address.OpenHaunt.strip?.mode === 'effect'
			)
	);

	// ── What is being built ───────────────────────────────────────────────────

	let layers = $state<EffectLayer[]>([]);
	let active = $state(0);
	/** The pool entry this was loaded from, so Update knows where it goes back. */
	let from = $state<EffectTemplate | null>(null);
	/** Where the preview's cycle starts. Fixed, so dragging does not restart it. */
	const t0 = Date.now();

	function fresh(kind: ParameterKind): EffectLayer {
		const param = shared.find((p) => parameterKey(p.kind) === parameterKey(kind));
		const spec = defaultSpec(kind, param?.defaultValue ?? { type: 'Float', value: 0 });
		return {
			kind,
			curve: spec.curve,
			rate: spec.rate,
			low: spec.low,
			high: spec.high,
			width: spec.width,
			direction: spec.direction,
			spread: pixels ? 'Linear' : 'Linear',
			spread_cycles: 1
		};
	}

	// A selection with nothing built yet starts on its first parameter. A new selection
	// keeps what is being built: an operator who loaded "fire" and then picked the
	// lanterns wants fire on the lanterns.
	$effect(() => {
		if (layers.length === 0 && shared.length > 0) {
			const intensity = shared.find((p) => p.kind === 'Intensity');
			layers = [fresh((intensity ?? shared[0]).kind)];
			active = 0;
		}
	});

	const layer = $derived<EffectLayer | null>(layers[active] ?? layers[0] ?? null);

	function edit(patch: Partial<EffectLayer>) {
		if (!layer) return;
		const i = layers.indexOf(layer);
		layers = layers.map((l, j) => (j === i ? { ...l, ...patch } : l));
	}

	function hzOf(l: EffectLayer): number {
		if ('Hz' in l.rate) return l.rate.Hz;
		const named = l.rate.Master;
		const master = $masters.find((m) => m.id === named.id);
		return master ? effectiveHz(master) * named.multiplier : 0;
	}
	const rateHz = $derived(layer ? hzOf(layer) : 0);
	/** For the preview: a layer's rate, by its index in what is being built. */
	const rateOf = $derived((index: number) => (layers[index] ? hzOf(layers[index]) : 0));

	function setRate(hz: number) {
		if (!layer) return;
		if ('Hz' in layer.rate) edit({ rate: { Hz: hz } });
		else {
			// A master's rate is the master's: a drag changes this effect's ratio to it.
			const base = rateHz / Math.max(layer.rate.Master.multiplier, 1e-6);
			edit({
				rate: {
					Master: {
						id: layer.rate.Master.id,
						multiplier: Math.round((hz / Math.max(base, 1e-6)) * 100) / 100
					}
				}
			});
		}
	}

	const valueType = $derived(layer?.low.type ?? 'Float');
	const shapes = $derived(layer ? shapesFor(layer.low) : []);
	const floatSteps = $derived(layer ? gridLevels(layer.curve) : null);
	const colorSteps = $derived(
		layer && 'Steps' in layer.curve && layer.curve.Steps.every((s) => s.value.type === 'Color')
			? layer.curve.Steps.map((s) => (s.value.type === 'Color' ? s.value.value : null)!)
			: null
	);

	function setShape(shape: Shape) {
		edit({ curve: { Shape: shape } });
	}

	function useSteps() {
		if (!layer) return;
		if (valueType === 'Color') {
			const a = layer.low.type === 'Color' ? layer.low.value : { r: 1, g: 0, b: 0 };
			const b = layer.high.type === 'Color' ? layer.high.value : { r: 0, g: 0, b: 1 };
			edit({ curve: gradientCurve([a, b]) });
		} else {
			edit({ curve: gridCurve([1, 0, 0, 0, 1, 0, 0, 0], false) });
		}
	}

	function addLayer(key: string) {
		const param = shared.find((p) => p.key === key);
		if (!param) return;
		layers = [...layers, fresh(param.kind)];
		active = layers.length - 1;
	}

	function removeLayer(index: number) {
		layers = layers.filter((_, i) => i !== index);
		active = Math.max(0, Math.min(active, layers.length - 1));
	}

	function setKind(key: string) {
		const param = shared.find((p) => p.key === key);
		if (!param || !layer) return;
		const replaced = fresh(param.kind);
		const i = layers.indexOf(layer);
		layers = layers.map((l, j) => (j === i ? replaced : l));
	}

	// ── The pool ──────────────────────────────────────────────────────────────

	let naming = $state<string | null>(null);
	let notice = $state<string | null>(null);

	function load(template: EffectTemplate) {
		layers = template.layers.map((l) => ({ ...l }));
		// Open on a layer this selection can take: fire on a strip of pixels is its color.
		const reached = templateApply(template, order, fixtureById, typeById, eachFixture);
		active = Math.max(0, reached.length ? reached[0].layer : 0);
		from = template;
		notice = null;
	}

	const applies = $derived(
		templateApply({ id: from?.id ?? '', layers }, order, fixtureById, typeById, eachFixture)
	);
	/** The fixtures the layer being edited reaches, as itself or as a pixel's color. */
	const reaching = $derived(
		applies.filter((a) => layers[a.layer] === layer).flatMap((a) => a.order)
	);
	const unreached = $derived(layers.filter((_, i) => !applies.some((a) => a.layer === i)));

	async function apply() {
		const written = await applyLayers(applies);
		const reached = applies.flatMap((a) => a.order);
		const fixtures = new Set(reached.map((t) => splitTarget(t).fixture)).size;
		const cells = new Set(reached.filter((t) => splitTarget(t).cell !== null)).size;
		notice = written
			? cells > 0
				? `Applied to ${cells} cells of ${fixtures} fixture${fixtures === 1 ? '' : 's'}`
				: `Applied to ${fixtures} fixtures`
			: 'Nothing selected can take this effect';
	}

	async function applyTemplate(template: EffectTemplate) {
		load(template);
		await applyLayers(templateApply(template, order, fixtureById, typeById, eachFixture));
	}

	// "Open in Effects" from the menu, the Inspector or ⌘K lands here.
	$effect(() => {
		const asked = $openTemplate;
		if (!asked) return;
		load(asked);
		openTemplate.set(null);
	});

	async function store() {
		const name = naming?.trim();
		if (!name) return;
		from = await storeTemplate(name, layers);
		naming = null;
		notice = `Stored as “${name}”`;
	}

	async function update() {
		if (!from) return;
		await updateTemplate(from.id, layers);
		notice = `Updated “${from.name}”. Nothing already running changes.`;
	}

	async function remove() {
		if (!from) return;
		await deleteTemplate(from.id);
		from = null;
	}

	function tags(template: EffectTemplate): string {
		return template.layers.map((l) => displayLabel(l.kind)).join(' + ');
	}

	/** How many of the selection already carry an effect on this layer's parameter. */
	const running = $derived.by(() => {
		if (!layer) return 0;
		const key = parameterKey(layer.kind);
		const chosen = new Set(picked.map((f) => f.id));
		return $entries.filter(
			(e) =>
				e.effect &&
				chosen.has(e.fixture_id) &&
				parameterKey(e.parameter_kind).split('@')[0] === key
		).length;
	});

	// ── A strip's own controls ────────────────────────────────────────────────

	const strip = $derived(layer ? readPixelSpread(layer.spread) : { reverse: false, mirror: false });

	function setStrip(next: Partial<{ reverse: boolean; mirror: boolean }>) {
		edit({ spread: pixelSpread({ ...strip, ...next }) });
	}

	let stops = $state([
		{ r: 1, g: 0.05, b: 0 },
		{ r: 1, g: 0.5, b: 0 },
		{ r: 0.4, g: 0, b: 0.6 }
	]);

	function useGradient() {
		const color = shared.find((p) => p.kind === 'ColorRgb');
		if (!color) return;
		const gradient: EffectLayer = {
			...fresh(color.kind),
			curve: gradientCurve(stops),
			rate: { Hz: 0.1 },
			spread: pixelSpread(strip),
			spread_cycles: layer?.spread_cycles ?? 1
		};
		const at = layers.findIndex((l) => l.kind === 'ColorRgb');
		if (at >= 0) {
			layers = layers.map((l, i) => (i === at ? gradient : l));
			active = at;
		} else {
			layers = [...layers, gradient];
			active = layers.length - 1;
		}
	}

	// ── A strip the node draws ────────────────────────────────────────────────

	const effectParam = $derived(shared.find((p) => p.choices.length > 0));
	const nodeParams = $derived(shared.filter((p) => p !== effectParam));

	/** A node draws its own strip, so these are whole fixtures, never cells. */
	const nodeStripIds = $derived(picked.map((f) => f.id));

	function heldOr(key: string, fallback: ParameterValue): ParameterValue {
		const first = nodeStripIds[0];
		const held = first ? $byKey.get(`${first}/${key}`) : undefined;
		return held?.value ?? fallback;
	}

	function nodeEffect(name: string) {
		if (!effectParam) return;
		beginGesture();
		setValue(nodeStripIds, effectParam.kind, { type: 'Text', value: name });
		endGesture();
	}
</script>

<div class="effects">
	<section class="pool" aria-label="Effect pool">
		<div class="pool-head">
			<h3>Pool</h3>
			{#if naming !== null}
				<input
					class="input"
					placeholder="Name"
					bind:value={naming}
					onkeydown={(e) => {
						if (e.key === 'Enter') void store();
						if (e.key === 'Escape') naming = null;
					}}
				/>
				<button class="btn btn-primary" onclick={store} disabled={!naming?.trim()}>Store</button>
				<button class="btn btn-ghost" onclick={() => (naming = null)}>Cancel</button>
			{:else if $canEdit}
				<button class="btn" onclick={() => (naming = '')} disabled={layers.length === 0}>Store as new</button>
				{#if from}
					<button class="btn" onclick={update}>Update “{from.name}”</button>
					<button class="btn btn-ghost" onclick={remove}>Delete</button>
				{/if}
			{/if}
		</div>
		{#if $templates.length === 0}
			<p class="empty">The pool is empty. Build an effect and store it here.</p>
		{:else}
			<div class="tiles">
				{#each $templates as template (template.id)}
					<!-- Tap applies to the selection and loads it here, so what just happened
					     is what the editor below shows. -->
					<Card
						ref={{ kind: 'effect', id: template.id }}
						name={template.name}
						color={template.color}
						state={tags(template)}
						on={from?.id === template.id}
						dim={order.length === 0}
						onact={() => (order.length === 0 ? load(template) : applyTemplate(template))}
					/>
				{/each}
			</div>
		{/if}
	</section>

	{#if picked.length === 0}
		<p class="empty">Select some fixtures. An effect is a shape applied across a selection.</p>
	{:else if nodeStrips}
		<section class="node" aria-label="The node's own effects">
			<p class="note">
				These strips draw their own pixels, so they run the node's effects. Pick one and set it
				up; it runs on the node, in step with every other strip.
			</p>
			{#if effectParam}
				<div class="node-effects">
					{#each effectParam.choices as name (name)}
						{@const current = heldOr(effectParam.key, effectParam.defaultValue)}
						<button
							class="btn"
							class:on={current.type === 'Text' && current.value === name}
							onclick={() => nodeEffect(name)}
						>{name}</button>
					{/each}
				</div>
			{/if}
			<div class="node-params">
				{#each nodeParams as param (param.key)}
					<div class="field">
						<span>{param.label}</span>
						<ValueControl
							value={heldOr(param.key, param.defaultValue)}
							label={param.label}
							range={param.physical}
							oninput={(v) => setValue(nodeStripIds, param.kind, v)}
						/>
					</div>
				{/each}
			</div>
		</section>
	{:else if !layer}
		<p class="empty">These fixtures have no parameter in common to put an effect on.</p>
	{:else}
		<header class="head">
			<div class="layers" role="tablist" aria-label="Layers">
				{#each layers as l, i (i)}
					<span class="layer-tab" class:on={l === layer}>
						<button role="tab" aria-selected={l === layer} onclick={() => (active = i)}>
							{displayLabel(l.kind)}
						</button>
						{#if layers.length > 1}
							<button class="x" aria-label="Remove this layer" onclick={() => removeLayer(i)}>×</button>
						{/if}
					</span>
				{/each}
				<select
					class="select"
					value=""
					aria-label="Add a layer"
					onchange={(e) => {
						addLayer(e.currentTarget.value);
						e.currentTarget.value = '';
					}}
				>
					<option value="">+ layer</option>
					{#each shared.filter((p) => !layers.some((l) => parameterKey(l.kind) === p.key)) as param (param.key)}
						<option value={param.key}>{param.label}</option>
					{/each}
				</select>
			</div>
			<span class="count">{reaching.length} of {order.length} take {displayLabel(layer.kind)}</span>
			{#if multiCell}
				<label class="each">
					<input type="checkbox" bind:checked={eachFixture} />
					each fixture
				</label>
			{/if}
			{#if running > 0}
				<span class="already">{running} already running</span>
			{/if}
			<button class="btn btn-primary" onclick={apply} disabled={applies.length === 0}>Apply</button>
		</header>
		{#if notice}
			<p class="notice">{notice}</p>
		{/if}
		{#if unreached.length > 0}
			<p class="notice dim">
				Nothing selected has {unreached.map((l) => displayLabel(l.kind)).join(', ')}; that layer is left out.
			</p>
		{/if}

		<EffectPreview
			{order}
			{applies}
			{rateOf}
			{layer}
			{t0}
			strip={pixels}
			oncycles={(c) => edit({ spread_cycles: c })}
		/>

		{#if floatSteps}
			<StepGrid
				levels={floatSteps}
				soft={'Steps' in layer.curve && layer.curve.Steps.some((s) => s.easing !== 'Step')}
				onchange={(levels, soft) => edit({ curve: gridCurve(levels, soft) })}
			/>
		{/if}
		<EffectWave
			{layer}
			{rateHz}
			count={reaching.length}
			{t0}
			onrate={setRate}
			onwidth={(w) => edit({ width: w })}
			onlow={(v) => edit({ low: v })}
			onhigh={(v) => edit({ high: v })}
		/>

		<div class="controls">
			<div class="group" role="group" aria-label="Shape">
				{#each shapes as shape (shape)}
					<button
						class="btn"
						class:on={'Shape' in layer.curve && layer.curve.Shape === shape}
						onclick={() => setShape(shape)}
					>{shape === 'SawUp' ? 'Saw ↑' : shape === 'SawDown' ? 'Saw ↓' : shape}</button>
				{/each}
				<button class="btn" class:on={'Steps' in layer.curve} onclick={useSteps}>
					{valueType === 'Color' ? 'Colors' : 'Steps'}
				</button>
			</div>

			<div class="group" role="group" aria-label="Spread">
				{#each SPREADS as option (option.label)}
					<button
						class="btn"
						class:on={spreadLabel(layer.spread) === option.label}
						onclick={() => edit({ spread: option.make(reaching.length) as Spread })}
					>{option.label}</button>
				{/each}
			</div>

			<button
				class="btn"
				onclick={() => edit({ direction: layer.direction === 'Forward' ? 'Backward' : 'Forward' })}
				aria-label="Direction"
			>{layer.direction === 'Forward' ? '→ Forward' : '← Backward'}</button>
		</div>

		{#if colorSteps}
			<div class="stops" aria-label="Colors">
				{#each colorSteps as stop, i (i)}
					<input
						class="swatch"
						type="color"
						aria-label="Color {i + 1}"
						value={rgbToHex(stop)}
						oninput={(e) => {
							const rgb = hexToRgb(e.currentTarget.value);
							if (!rgb) return;
							edit({ curve: gradientCurve(colorSteps.map((s, j) => (j === i ? rgb : s))) });
						}}
					/>
				{/each}
				<button class="btn btn-ghost" onclick={() => edit({ curve: gradientCurve([...colorSteps, colorSteps[colorSteps.length - 1]]) })}>+ color</button>
				{#if colorSteps.length > 2}
					<button class="btn btn-ghost" onclick={() => edit({ curve: gradientCurve(colorSteps.slice(0, -1)) })}>− color</button>
				{/if}
			</div>
		{/if}

		{#if pixels}
			<section class="strip" aria-label="Along the strip">
				<h3>Along the strip</h3>
				<div class="controls">
					<div class="group" role="group" aria-label="Direction along the strip">
						<button class="btn" class:on={!strip.reverse && !strip.mirror} onclick={() => setStrip({ reverse: false, mirror: false })}>From the start</button>
						<button class="btn" class:on={strip.reverse} onclick={() => setStrip({ reverse: true, mirror: false })}>From the end</button>
						<button class="btn" class:on={strip.mirror} onclick={() => setStrip({ mirror: !strip.mirror })}>Mirror</button>
					</div>
					<div class="field">
						<span>Segments</span>
						<button class="btn btn-icon" aria-label="Fewer segments" onclick={() => edit({ spread_cycles: Math.max(1, Math.round(layer.spread_cycles) - 1) })}>−</button>
						<span class="count">{layer.spread_cycles}</span>
						<button class="btn btn-icon" aria-label="More segments" onclick={() => edit({ spread_cycles: Math.min(16, Math.round(layer.spread_cycles) + 1) })}>+</button>
					</div>
					<div class="field">
						<span>Lit part</span>
						<input
							class="range"
							type="range"
							min="0.05"
							max="0.95"
							step="0.05"
							value={layer.width}
							oninput={(e) => {
								if (!('Shape' in layer.curve) || layer.curve.Shape !== 'Square') setShape('Square');
								edit({ width: Number(e.currentTarget.value) });
							}}
						/>
						<span class="unit">{Math.round(layer.width * 100)}% of each segment</span>
					</div>
				</div>
				{#if shared.some((p) => p.kind === 'ColorRgb')}
					<div class="stops">
						{#each stops as stop, i (i)}
							<input
								class="swatch"
								type="color"
								aria-label="Gradient color {i + 1}"
								value={rgbToHex(stop)}
								oninput={(e) => {
									const rgb = hexToRgb(e.currentTarget.value);
									if (rgb) stops = stops.map((s, j) => (j === i ? rgb : s));
								}}
							/>
						{/each}
						<button class="btn" onclick={useGradient}>Gradient along the strip</button>
					</div>
				{/if}
			</section>
		{/if}

		<details class="numbers">
			<summary>Numbers</summary>
			<div class="controls">
				<div class="field">
					<span>Parameter</span>
					<select class="select" value={parameterKey(layer.kind)} onchange={(e) => setKind(e.currentTarget.value)}>
						{#each shared as param (param.key)}
							<option value={param.key}>{param.label}</option>
						{/each}
					</select>
				</div>
				<div class="field">
					<span>Rate</span>
					{#if 'Hz' in layer.rate}
						<input
							class="input narrow"
							type="number"
							min="0"
							max="20"
							step="0.05"
							value={layer.rate.Hz}
							onchange={(e) => edit({ rate: { Hz: Number(e.currentTarget.value) } })}
						/>
						<span class="unit">Hz</span>
					{:else}
						{@const named = layer.rate.Master}
						<input
							class="input narrow"
							type="number"
							min="0.05"
							max="8"
							step="0.05"
							value={named.multiplier}
							onchange={(e) => edit({ rate: { Master: { id: named.id, multiplier: Number(e.currentTarget.value) } } })}
						/>
						<span class="unit">×</span>
					{/if}
					<select
						class="select"
						value={'Hz' in layer.rate ? '' : layer.rate.Master.id}
						onchange={(e) => {
							const id = e.currentTarget.value;
							edit({ rate: id ? { Master: { id, multiplier: 1 } } : { Hz: rateHz || 0.5 } });
						}}
					>
						<option value="">Own rate</option>
						{#each $masters as master (master.id)}
							<option value={master.id}>{master.name}</option>
						{/each}
					</select>
				</div>
				<div class="field">
					<span>Width</span>
					<input class="input narrow" type="number" min="0.02" max="1" step="0.01" value={layer.width} onchange={(e) => edit({ width: Number(e.currentTarget.value) })} />
				</div>
				<div class="field">
					<span>Spread over</span>
					<input class="input narrow" type="number" min="0" max="16" step="0.25" value={layer.spread_cycles} onchange={(e) => edit({ spread_cycles: Number(e.currentTarget.value) })} />
					<span class="unit">cycles</span>
				</div>
				{#if 'Shape' in layer.curve}
					<div class="field">
						<span>Low</span>
						<ValueControl value={layer.low} label="Low" oninput={(v) => edit({ low: v })} />
					</div>
					<div class="field">
						<span>High</span>
						<ValueControl value={layer.high} label="High" oninput={(v) => edit({ high: v })} />
					</div>
				{/if}
			</div>
		</details>
	{/if}
</div>

<style>
	.effects {
		padding: 14px 18px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}
	h3 {
		margin: 0;
		font-size: var(--font-sm);
		color: var(--text-dim);
		font-weight: 600;
	}
	.empty,
	.note {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 52ch;
		line-height: 1.5;
		margin: 0;
	}
	.notice {
		margin: 0;
		font-size: var(--font-xs);
		color: var(--accent);
	}
	.notice.dim {
		color: var(--text-dim);
	}
	.pool {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.pool-head {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.pool-head h3 {
		margin-right: auto;
	}
	.tiles {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(124px, 1fr));
		gap: 6px;
	}
	.head {
		display: flex;
		align-items: center;
		gap: var(--pad);
		flex-wrap: wrap;
	}
	.layers {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.layer-tab {
		display: flex;
		border: 1px solid var(--line);
		border-radius: var(--radius);
	}
	.layer-tab.on {
		border-color: var(--accent);
	}
	.layer-tab button {
		border: none;
		background: transparent;
		color: inherit;
		padding: 4px 10px;
		cursor: pointer;
	}
	.layer-tab.on button {
		color: var(--accent);
	}
	.layer-tab .x {
		padding: 4px 6px;
		color: var(--text-dim);
	}
	.count {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.already {
		color: var(--live);
		font-size: var(--font-xs);
	}
	.each {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.head .btn-primary {
		margin-left: auto;
	}
	.controls {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--pad) 14px;
	}
	.group {
		display: flex;
		flex-wrap: wrap;
		gap: 2px;
	}
	.btn.on {
		border-color: var(--accent);
		color: var(--accent);
	}
	.field {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.field > span:first-child {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.unit {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.input.narrow {
		width: 5.5rem;
	}
	.stops {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.swatch {
		width: 44px;
		height: 32px;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: none;
		cursor: pointer;
	}
	.strip {
		display: flex;
		flex-direction: column;
		gap: 8px;
		border-top: 1px solid var(--line);
		padding-top: 10px;
	}
	.numbers summary {
		cursor: pointer;
		color: var(--text-dim);
		font-size: var(--font-sm);
		margin-bottom: 8px;
	}
	.node {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.node-effects {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.node-effects .btn {
		min-height: 40px;
		min-width: 80px;
		text-transform: capitalize;
	}
	.node-params {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
		gap: 8px 16px;
	}
</style>
