<script lang="ts">
	/**
	 * Building an effect for the selection.
	 *
	 * No Edit toggle: this panel *is* an editor, and everything in it writes to the
	 * programmer, which is the scratch buffer — nothing here reaches the show until
	 * it is stored into a cue.
	 *
	 * The waveform is the point. An operator asking for "a chase across these six"
	 * wants to see six dots moving round one curve, not read six phase numbers, and
	 * the dots move at the rate the lights are actually moving because they are drawn
	 * from the same arithmetic the engine renders with.
	 */

	import type {
		EffectSpec,
		ParameterKind,
		ParameterValue,
		Shape,
		Spread,
		Step
	} from '$lib/generated/index.js';
	import {
		curveAt,
		cyclePosition,
		defaultSpec,
		phases,
		shapesFor,
		specsFor,
		SPREADS,
		spreadLabel
	} from '$lib/effects.js';
	import { editableParameters } from '$lib/programmer.js';
	import { parameterKey } from '$lib/patch.js';
	import { effectiveHz } from '$lib/speedmasters.js';
	import { entries, setEffect } from '$lib/stores/programmer.js';
	import { selected } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import ValueControl from '../programmer/controls/ValueControl.svelte';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const masters = collection('speed_masters');

	const picked = $derived($fixtures.filter((f) => $selected.has(f.id)));
	/** What every selected fixture has in common, which is what can be effected. */
	const shared = $derived(editableParameters($types, picked));

	let kindKey = $state<string | null>(null);
	const kind = $derived<ParameterKind | null>(
		shared.find((p) => parameterKey(p.kind) === kindKey)?.kind ?? shared[0]?.kind ?? null
	);
	const fallback = $derived<ParameterValue>(
		shared.find((p) => p.kind === kind)?.defaultValue ?? { type: 'Float', value: 0 }
	);

	/** The effect being built. Kept here, not in the show, until Apply. */
	let draft = $state<EffectSpec | null>(null);
	let spread = $state<Spread>('Linear');

	// A new selection or a different parameter means a different effect. Rebuilt
	// rather than adapted, because a sine's endpoints mean nothing on a relay.
	$effect(() => {
		const k = kind;
		if (!k) {
			draft = null;
			return;
		}
		draft = defaultSpec(k, fallback);
	});

	/** A clock for the dots. One for the panel, as in the speed masters. */
	let now = $state(Date.now());
	$effect(() => {
		let frame = 0;
		const tick = () => {
			now = Date.now();
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	const shapes = $derived(shapesFor(fallback));
	const usingSteps = $derived(!!draft && 'Steps' in draft.curve);
	const draftSteps = $derived<Step[]>(draft && 'Steps' in draft.curve ? draft.curve.Steps : []);

	/** What the draft would run at, resolving a master if it names one. */
	const rateHz = $derived.by(() => {
		const spec = draft;
		if (!spec) return 0;
		if ('Hz' in spec.rate) return spec.rate.Hz;
		const named = spec.rate.Master;
		const master = $masters.find((m) => m.id === named.id);
		return master ? effectiveHz(master) * named.multiplier : 0;
	});

	/** Where each selected fixture would sit, for the dots. */
	const dots = $derived.by(() => {
		const spec = draft;
		if (!spec) return [];
		const offsets = phases(spread, picked.length);
		const backward = spec.direction === 'Backward';
		return picked.map((fixture, i) => ({
			fixture,
			x: cyclePosition(rateHz, backward, offsets[i] ?? 0, spec.t0 ?? 0, now)
		}));
	});

	/** The outline, sampled once across the cycle. */
	const outline = $derived.by(() => {
		const spec = draft;
		if (!spec) return '';
		const steps = 120;
		const points: string[] = [];
		for (let i = 0; i <= steps; i++) {
			const x = i / steps;
			const level = curveAt(spec.curve, spec.width, x);
			points.push(`${(x * 100).toFixed(2)},${((1 - level) * 100).toFixed(2)}`);
		}
		return points.join(' ');
	});

	/**
	 * Change one thing about the draft.
	 *
	 * One guard rather than one per control: every handler below is a closure the
	 * compiler cannot narrow, and eighteen `if (!draft) return` lines would say the
	 * same thing eighteen times.
	 */
	function edit(patch: Partial<EffectSpec>) {
		if (draft) draft = { ...draft, ...patch };
	}

	function setShape(shape: Shape) {
		edit({ curve: { Shape: shape } });
	}

	function useSteps() {
		if (!draft) return;
		// Somewhere to start: two steps between the ends, which is a hard chase and
		// what most people mean by "steps".
		edit({
			curve: {
				Steps: [
					{ at: 0, value: draft.low, easing: 'Step' },
					{ at: 0.5, value: draft.high, easing: 'Step' }
				]
			}
		});
	}

	function addStep() {
		if (!draft || !('Steps' in draft.curve)) return;
		const steps = draft.curve.Steps;
		const at = steps.length / (steps.length + 1);
		edit({ curve: { Steps: [...steps, { at, value: draft.high, easing: 'Step' }] } });
	}

	function updateStep(index: number, patch: Partial<Step>) {
		if (!draft || !('Steps' in draft.curve)) return;
		const steps = draft.curve.Steps.map((s, i) => (i === index ? { ...s, ...patch } : s));
		edit({ curve: { Steps: steps } });
	}

	function removeStep(index: number) {
		if (!draft || !('Steps' in draft.curve)) return;
		edit({ curve: { Steps: draft.curve.Steps.filter((_, i) => i !== index) } });
	}

	async function apply() {
		if (!draft || !kind || picked.length === 0) return;
		/* eslint-disable-next-line @typescript-eslint/no-unused-vars */
		const { effect_id, phase, ...base } = draft;
		await setEffect(
			kind,
			specsFor(
				picked.map((f) => f.id),
				base,
				spread
			)
		);
	}

	/** How many of the selection already carry an effect on this parameter. */
	const running = $derived.by(() => {
		if (!kind) return 0;
		const key = parameterKey(kind);
		return $entries.filter(
			(e) => e.effect && $selected.has(e.fixture_id) && parameterKey(e.parameter_kind) === key
		).length;
	});
</script>

<div class="effects">
	{#if picked.length === 0}
		<p class="empty">Select some fixtures. An effect is a shape applied across a selection.</p>
	{:else if !draft || !kind}
		<p class="empty">These fixtures have no parameter in common to put an effect on.</p>
	{:else}
		<header class="head">
			<!-- Shows the parameter actually in use, not `kindKey`: that is null until
			     something is picked, and a blank box over a working effect reads as a
			     panel that has not decided what it is doing. -->
			<select
				class="select"
				value={parameterKey(kind)}
				onchange={(e) => (kindKey = e.currentTarget.value)}
			>
				{#each shared as param (parameterKey(param.kind))}
					<option value={parameterKey(param.kind)}>{parameterKey(param.kind)}</option>
				{/each}
			</select>
			<span class="count">{picked.length} selected</span>
			{#if running > 0}
				<span class="already">{running} already running</span>
			{/if}
			<button class="btn btn-primary" onclick={apply}>Apply</button>
		</header>

		<!-- One cycle, with a dot per fixture where it currently sits. Six dots moving
		     round one curve is what "chase across these" looks like; six phase numbers
		     is not. -->
		<div class="wave">
			<svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-label="The effect's shape">
				<polyline points={outline} />
			</svg>
			{#each dots as dot (dot.fixture.id)}
				<span
					class="dot"
					style:left="{dot.x * 100}%"
					style:bottom="{curveAt(draft.curve, draft.width, dot.x) * 100}%"
					title={dot.fixture.name}
				></span>
			{/each}
		</div>

		<div class="controls">
			<div class="group" role="group" aria-label="Shape">
				{#each shapes as shape (shape)}
					<button
						class="btn"
						class:on={!usingSteps && 'Shape' in draft.curve && draft.curve.Shape === shape}
						onclick={() => setShape(shape)}
					>{shape}</button>
				{/each}
				<button class="btn" class:on={usingSteps} onclick={useSteps}>Steps</button>
			</div>

			<div class="field">
				<span>Rate</span>
				{#if 'Hz' in draft.rate}
					{@const hz = draft.rate.Hz}
					<input
						class="input narrow"
						type="number"
						min="0"
						max="20"
						step="0.05"
						value={hz}
						onchange={(e) => edit({ rate: { Hz: Number(e.currentTarget.value) } })}
					/>
					<span class="unit">Hz</span>
				{:else}
					{@const named = draft.rate.Master}
					<input
						class="input narrow"
						type="number"
						min="0.05"
						max="8"
						step="0.05"
						value={named.multiplier}
						onchange={(e) =>
							edit({ rate: { Master: { id: named.id, multiplier: Number(e.currentTarget.value) } } })}
					/>
					<span class="unit">×</span>
				{/if}
				<select
					class="select"
					value={'Hz' in draft.rate ? '' : draft.rate.Master.id}
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

			{#if !usingSteps}
				<div class="field">
					<span>Low</span>
					<ValueControl value={draft.low} label="Low" oninput={(v) => edit({ low: v })} />
				</div>
				<div class="field">
					<span>High</span>
					<ValueControl value={draft.high} label="High" oninput={(v) => edit({ high: v })} />
				</div>
			{/if}

			{#if !usingSteps && 'Shape' in draft.curve && draft.curve.Shape === 'Square'}
				<div class="field">
					<span>Width</span>
					<input
						class="input narrow"
						type="number"
						min="0.05"
						max="0.95"
						step="0.05"
						value={draft.width}
						onchange={(e) => edit({ width: Number(e.currentTarget.value) })}
					/>
				</div>
			{/if}

			<div class="group" role="group" aria-label="Direction">
				<button
					class="btn"
					class:on={draft.direction === 'Forward'}
					onclick={() => edit({ direction: 'Forward' })}
				>Forward</button>
				<button
					class="btn"
					class:on={draft.direction === 'Backward'}
					onclick={() => edit({ direction: 'Backward' })}
				>Backward</button>
			</div>

			<div class="field">
				<span>Spread</span>
				<select
					class="select"
					value={spreadLabel(spread)}
					onchange={(e) => {
						const chosen = SPREADS.find((s) => s.label === e.currentTarget.value);
						if (chosen) spread = chosen.make(picked.length);
					}}
				>
					{#each SPREADS as option (option.label)}
						<option value={option.label}>{option.label}</option>
					{/each}
				</select>
				{#if typeof spread === 'object' && 'Wings' in spread}
					{@const wings = spread.Wings}
					<input
						class="input narrow"
						type="number"
						min="2"
						max="8"
						value={wings}
						onchange={(e) => (spread = { Wings: Number(e.currentTarget.value) })}
					/>
				{:else if typeof spread === 'object' && 'Groups' in spread}
					{@const groups = spread.Groups}
					<input
						class="input narrow"
						type="number"
						min="2"
						max="8"
						value={groups}
						onchange={(e) => (spread = { Groups: Number(e.currentTarget.value) })}
					/>
				{:else if typeof spread === 'object' && 'Random' in spread}
					<!-- A random spread is a seed, not a roll: it is stored, so the same
					     phases come back on every console and after a reload. Reseeding is
					     how you ask for a different arrangement. -->
					<button
						class="btn btn-ghost"
						onclick={() => (spread = { Random: { seed: (Math.random() * 2 ** 32) >>> 0 } })}
					>Reseed</button>
				{/if}
			</div>
		</div>

		{#if usingSteps}
			<div class="steps">
				{#each draftSteps as step, i (i)}
					<div class="step row-touch">
						<input
							class="input narrow"
							type="number"
							min="0"
							max="0.99"
							step="0.01"
							value={step.at}
							onchange={(e) => updateStep(i, { at: Number(e.currentTarget.value) })}
						/>
						<ValueControl
							value={step.value}
							label="Step {i + 1}"
							oninput={(v) => updateStep(i, { value: v })}
						/>
						<select
							class="select"
							value={step.easing}
							onchange={(e) => updateStep(i, { easing: e.currentTarget.value as Step['easing'] })}
						>
							<option value="Step">Jump</option>
							<option value="Linear">Crossfade</option>
							<option value="EaseIn">Ease in</option>
							<option value="EaseOut">Ease out</option>
							<option value="EaseInOut">Ease both</option>
						</select>
						<button class="btn btn-danger btn-icon" onclick={() => removeStep(i)}>×</button>
					</div>
				{/each}
				<button class="btn btn-ghost" onclick={addStep}>+ Step</button>
			</div>
		{/if}
	{/if}
</div>

<style>
	.effects {
		padding: 16px 20px;
		display: flex;
		flex-direction: column;
		gap: 14px;
	}

	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 44ch;
		line-height: 1.5;
	}

	.head {
		display: flex;
		align-items: center;
		gap: var(--pad);
	}

	.count {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}

	.already {
		color: var(--live);
		font-size: var(--font-xs);
	}

	.head .btn-primary {
		margin-left: auto;
	}

	.wave {
		position: relative;
		height: 110px;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-sunken);
	}

	.wave svg {
		width: 100%;
		height: 100%;
		display: block;
	}

	.wave polyline {
		fill: none;
		stroke: var(--accent);
		stroke-width: 1.5;
		vector-effect: non-scaling-stroke;
	}

	.dot {
		position: absolute;
		width: 10px;
		height: 10px;
		margin: 5px 0 -5px -5px;
		border-radius: 50%;
		background: var(--live);
		pointer-events: none;
	}

	.controls {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--pad) 16px;
	}

	.group {
		display: flex;
		gap: 2px;
	}
	.group .btn.on {
		border-color: var(--accent);
		color: var(--accent);
	}

	.field {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.field > span {
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

	.steps {
		display: flex;
		flex-direction: column;
		gap: 4px;
		border-top: 1px solid var(--line);
		padding-top: var(--pad);
	}
	.step {
		gap: var(--pad);
	}
</style>
