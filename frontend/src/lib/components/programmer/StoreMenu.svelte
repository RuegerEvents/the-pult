<script lang="ts">
	/**
	 * The store menu.
	 *
	 * The spec asks for it to show "which fixtures, attributes and values are stored",
	 * and to let the operator deselect what should not be — so the list of what is
	 * about to be written is the menu, rather than a confirmation bolted onto a button.
	 *
	 * Merge is the default and Replace is only offered for a cue that already exists,
	 * because there is nothing to replace in a cue being made here and now.
	 *
	 * Two things it remembers, both per browser. **Which sequence** was last stored
	 * into, because an operator building act two stores into act two twenty times
	 * running and picking it each time is twenty pointless decisions. And **track or
	 * cue only**, because that is a way of working rather than a per-cue choice.
	 */

	import { untrack } from 'svelte';

	import type { Cue, Easing, ParameterCapture, Sequence, StoreOnto } from '$lib/generated/index.js';
	import { createCue, cueIdsThrough, defaultCueName, newCueNumber, cueOnlyCompensation, DEFAULT_FADE_MS, trackedThrough } from '$lib/cues.js';
	import { CURVE_LABELS, CURVES, curveForKey } from '$lib/fade.js';
	import { displayLabel, formatValue, parameterKey } from '$lib/patch.js';
	import { clear, entries, storeInto, storePreset, updatePreset } from '$lib/stores/programmer.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { bindOnto } from '$lib/stores/wing.js';
	import { collection, show, showData } from '$lib/stores/show.js';
	import { addToast } from '$lib/toasts.js';
	import { focusOnMount } from '$lib/actions.js';
	import Dialog from '$lib/components/Dialog.svelte';
	import { uuid } from '$lib/uuid.js';

	let {
		onclose,
		/**
		 * The entries to open with ticked. Everything, unless Update handed over the
		 * keys no cue is driving — which is the one case where a subset is the answer
		 * rather than a guess.
		 */
		preselect = null,
		/**
		 * The playback STORE was pressed onto on the wing: the dialog opens on its
		 * sequence, or on a new one that the playback is then bound to.
		 */
		onto = null
	}: { onclose: () => void; preselect?: string[] | null; onto?: StoreOnto | null } = $props();

	const fixtures = collection('fixtures');
	const sequences = collection('sequences');
	const cues = collection('cues');
	const presets = collection('presets');

	const SEQUENCE_KEY = 'pult.store.sequence';
	const TRACKING_KEY = 'pult.store.tracking';

	const remembered = (key: string): string | null => {
		try {
			return localStorage.getItem(key);
		} catch {
			return null;
		}
	};
	const remember = (key: string, value: string) => {
		try {
			localStorage.setItem(key, value);
		} catch {
			// A browser with storage off still stores cues; it just asks every time.
		}
	};

	let sequenceId = $state<string | null>(
		untrack(() => (onto ? (onto.sequence_id ?? '__new') : remembered(SEQUENCE_KEY)))
	);
	/**
	 * A sequence made by this store, for a show that has none or a cue that belongs in
	 * a new one. Made in the same gesture as the cue, so one Ctrl-Z takes both back —
	 * there used to be no way to store a first cue at all without leaving for the
	 * Sequences panel.
	 */
	const NEW_SEQUENCE = '__new';
	let sequenceName = $state(untrack(() => `Sequence ${$sequences.length + 1}`));
	const creatingSequence = $derived(sequenceId === NEW_SEQUENCE || $sequences.length === 0);
	/**
	 * A cue, or a preset.
	 *
	 * The third target is here rather than only on the Pools panel because the store
	 * dialog is where an operator already is when they have built something worth
	 * keeping, and "this is a look I will want again" is a decision made at exactly
	 * that moment.
	 */
	/**
	 * Onto a playback with exactly one cue, Store means that cue, merged: a playback
	 * with one cue is usually a look being built up. With more, it means a new cue.
	 */
	const ontoOnlyCue = untrack(() => {
		const s = onto?.sequence_id ? $sequences.find((q) => q.id === onto.sequence_id) : undefined;
		return s && s.cue_ids.length === 1 ? s.cue_ids[0] : null;
	});
	let target = $state<'new' | 'existing' | 'preset'>(ontoOnlyCue ? 'existing' : 'new');
	let presetTarget = $state<string | null>(null);
	let presetName = $state('');
	let cueId = $state<string | null>(ontoOnlyCue);
	/**
	 * The cue's name: what was typed, or "Cue <n>" for the number it is about to get,
	 * which follows the *After* choice until somebody types. Prefilled and selected,
	 * so Enter stores it as it is and typing replaces it.
	 */
	let typedName = $state<string | null>(null);
	let mode = $state<'merge' | 'replace'>('merge');
	/**
	 * Track or cue only.
	 *
	 * Track is preselected because tracking is what this playback does and what a
	 * designer means most of the time: a look built in cue 3 should still be there in
	 * cue 4. Cue only is the answer when one moment is being fixed rather than the
	 * look being changed — see `cueOnlyCompensation`.
	 */
	let tracking = $state<'track' | 'cueOnly'>(
		remembered(TRACKING_KEY) === 'cueOnly' ? 'cueOnly' : 'track'
	);
	let keep = $state(false);
	let storing = $state(false);

	/// What the operator has unticked. Everything else is stored, so an entry that
	/// arrives while the menu is open — another console programming alongside — is
	/// included rather than silently left out.
	// The initial value on purpose: `preselect` is what Update handed over at the
	// moment the dialog opened, and an entry arriving afterwards is included rather
	// than being silently dropped by a list that was made before it existed.
	let dropped = $state(
		untrack(
			() =>
				new Set(preselect ? $entries.filter((e) => !preselect.includes(e.id)).map((e) => e.id) : [])
		)
	);
	const include = $derived(
		new Set($entries.filter((entry) => !dropped.has(entry.id)).map((entry) => entry.id))
	);

	const sequence = $derived(
		creatingSequence
			? null
			: ($sequences.find((s) => s.id === sequenceId) ?? ($sequences[0] as Sequence | undefined) ?? null)
	);
	const cuesInSequence = $derived(
		sequence ? sequence.cue_ids.map((id) => $cues.find((c) => c.id === id)).filter((c): c is Cue => !!c) : []
	);
	const cue = $derived(cuesInSequence.find((c) => c.id === cueId) ?? null);
	/**
	 * Where a new cue goes: after the one that is up, which is where somebody
	 * programming a show in order means. Appended when nothing is up.
	 */
	const afterActive = $derived(
		sequence?.active_cue_index != null ? (sequence.cue_ids[sequence.active_cue_index] ?? null) : null
	);
	let after = $state<string | null | undefined>(undefined);
	const insertAfter = $derived(after === undefined ? afterActive : after);
	const name = $derived(
		typedName ??
			(sequence ? defaultCueName(newCueNumber(sequence, $cues, insertAfter)) : defaultCueName(1))
	);
	const nameOf = (fixtureId: string) =>
		$fixtures.find((f) => f.id === fixtureId)?.name ?? fixtureId.slice(0, 6);

	const canStore = $derived(
		include.size > 0 &&
			(target === 'preset'
				? presetTarget !== null || presetName.trim().length > 0
				: target === 'new'
					? (!!sequence || sequenceName.trim().length > 0) && name.trim().length > 0
					: !!sequence && cue !== null)
	);

	/**
	 * Per-capture timing, held here until Store.
	 *
	 * Zero means "use the cue's", which is what `Playback::start_cue` already does
	 * with `fade_in_ms`, so leaving every row alone gives exactly the behavior the
	 * console had before there was anywhere to type these.
	 */
	type Timing = { fade: number; delay: number; easing: Easing | null };
	let timing = $state<Record<string, Timing>>({});
	const timingFor = (id: string): Timing =>
		timing[id] ?? { fade: 0, delay: 0, easing: null };
	function setTiming(id: string, patch: Partial<Timing>) {
		timing = { ...timing, [id]: { ...timingFor(id), ...patch } };
	}

	/** Cue-level timing for a new cue. An existing one keeps its own. */
	let cueFadeIn = $state(DEFAULT_FADE_MS);
	let cueFadeOut = $state(DEFAULT_FADE_MS);
	/** And its curve. `null` is the show's own default, per parameter. */
	let cueEasing = $state<Easing | null>(null);

	/**
	 * What a row that says nothing will actually fade on, so the picker can name it
	 * rather than saying "inherited" and leaving an operator to go and look.
	 *
	 * The show's default for that parameter, or this menu's cue-level pick where one
	 * has been made — which is the same three steps the station resolves, minus the
	 * capture's own, since that is what the picker is for.
	 */
	const inheritedCurve = (kind: Cue['captures'][number]['parameter_kind']): Easing =>
		cueEasing ?? ($show ? curveForKey($show.fade_curves, parameterKey(kind)) : 'Linear');

	/// And whose answer it is, because otherwise the inherited option and the explicit
	/// one read identically and an operator cannot tell which they picked.
	const inheritedFrom = (kind: Cue['captures'][number]['parameter_kind']) =>
		`${CURVE_LABELS[inheritedCurve(kind)]} · ${cueEasing ? "cue's" : "show's"}`;
	// The picker's own two states, not the schema enum: a timecode follow needs a
	// timecode source, which does not exist yet.
	let followMode = $state<'Manual' | 'FollowAfter'>('Manual');
	let followDelay = $state(0);

	function tick(id: string, on: boolean) {
		const next = new Set(dropped);
		if (on) next.delete(id);
		else next.add(id);
		dropped = next;
	}

	/**
	 * The cue a compensating value would go into, and what it is tracking now.
	 *
	 * Read *before* the store, because cue only preserves what the next cue was
	 * showing — which is a fact about the show as it stands and not about the show the
	 * store is about to make.
	 */
	function nextCueAndTracked(
		storedInto: string
	): { next: Cue; before: Map<string, ParameterCapture> } | null {
		if (!sequence) return null;
		const at = sequence.cue_ids.indexOf(storedInto);
		const nextId = at >= 0 ? sequence.cue_ids[at + 1] : undefined;
		if (!nextId) return null;
		const next = $cues.find((c) => c.id === nextId);
		if (!next) return null;
		const byId = new Map($cues.map((c) => [c.id, c]));
		const before = new Map<string, ParameterCapture>();
		for (const entry of trackedThrough(cueIdsThrough(sequence, storedInto), (id) => byId.get(id))) {
			before.set(
				`${entry.capture.fixture_id}/${parameterKey(entry.capture.parameter_kind)}`,
				entry.capture
			);
		}
		return { next, before };
	}

	/**
	 * The other kind of store: a look somebody will want again.
	 *
	 * No cue-only, no timing, no target sequence — a preset is values and a name, and
	 * merging into one that exists is the same act as *Update from programmer* on the
	 * pool button, so it goes through the same function.
	 */
	async function storeAsPreset() {
		storing = true;
		try {
			const existing = $presets.find((p) => p.id === presetTarget) ?? null;
			if (existing) await updatePreset(existing, include);
			else await storePreset(presetName.trim(), include);
			if (!keep) await clear({ keepLocked: true });
			onclose();
		} catch (e) {
			addToast(e instanceof Error ? e.message : 'that would not store');
		} finally {
			storing = false;
		}
	}

	/** Which parameters this store actually changes, as `"fixture/key"`. */
	const changedKeys = () =>
		$entries
			.filter((entry) => include.has(entry.id))
			.map((entry) => `${entry.fixture_id}/${parameterKey(entry.parameter_kind)}`);

	async function store() {
		if (storing) return;
		if (target === 'preset') return storeAsPreset();
		if (!sequence && !(creatingSequence && target === 'new')) return;
		storing = true;
		remember(TRACKING_KEY, tracking);
		// One gesture over the cue being stored into *and* the compensation in the one
		// after it: cue only is a single act, and it is one Ctrl-Z.
		beginGesture();
		try {
			let into = sequence;
			if (!into) {
				into = {
					id: uuid(),
					name: sequenceName.trim(),
					cue_ids: [],
					active_cue_index: null,
					went_at: null,
					master: null
				} as Sequence;
				await showData().sequences.create(into);
			}
			remember(SEQUENCE_KEY, into.id);
			const changed = changedKeys();
			const compensation = tracking === 'cueOnly';
			// Taken before anything is written, because it is what the next cue was
			// tracking rather than what it will be.
			const ahead = compensation
				? nextCueAndTracked(target === 'new' ? (insertAfter ?? '') : (cueId ?? ''))
				: null;

			if (target === 'new') {
				await createCue(showData(), into, $cues, {
					name: name.trim(),
					fadeInMs: cueFadeIn,
					fadeOutMs: cueFadeOut,
					easing: cueEasing,
					after: insertAfter ?? undefined,
					followMode:
						followMode === 'Manual' ? 'Manual' : { FollowAfter: { delay_ms: followDelay } },
					captures: $entries
						.filter((entry) => include.has(entry.id))
						.map((entry) => {
							const t = timingFor(entry.id);
							return {
								fixture_id: entry.fixture_id,
								parameter_kind: entry.parameter_kind,
								value: entry.value,
								// A stored effect drops its anchor: the cue's `went_at` is
								// what it is measured from on every Go.
								effect: entry.effect ? { ...entry.effect, t0: null } : null,
								// The palette this value came from, if it came from one. The
								// literal beside it is what plays if the preset is deleted.
								preset: entry.preset ?? null,
								// `null` is not "linear": it is this capture saying nothing, so
								// the cue answers, and the show answers for the cue.
								easing: t.easing,
								fade_in_ms: t.fade,
								fade_out_ms: 0,
								delay_in_ms: t.delay
							};
						})
				});
			} else if (cue) {
				await storeInto(cue, mode, include);
			}

			if (ahead) {
				const compensated = cueOnlyCompensation(ahead.next, changed, ahead.before);
				if (compensated) await showData().cues.byId(ahead.next.id).captures.set(compensated);
			}
			// A playback nothing was on plays what was stored onto it, in the same gesture.
			if (onto && onto.sequence_id === null) await bindOnto(onto, into.id);
			if (!keep) await clear({ keepLocked: true });
			onclose();
		} catch (e) {
			addToast(e instanceof Error ? e.message : 'that would not store');
		} finally {
			endGesture();
			storing = false;
		}
	}
</script>

<Dialog title="Store" size="large" {onclose}>
	{#snippet footer()}
		<button class="ghost" onclick={onclose}>Cancel</button>
		<button class="primary" disabled={!canStore || storing} onclick={store}>
			{storing ? 'Storing…' : target === 'preset' ? 'Store preset' : target === 'existing' ? 'Store into cue' : 'Store cue'}
		</button>
	{/snippet}

	{#if $entries.length === 0}
		<p class="empty">The programmer is empty, so there is nothing to store.</p>
	{:else}
		<div class="store">
			<!-- What: every value the programmer holds, each one ticked or not, with the
			     timing it would take. -->
			<section class="what">
				<h3>What <span class="dim">{include.size} of {$entries.length}</span></h3>
				<div class="list">
					<table>
						<thead>
							<tr>
								<th></th><th>Fixture</th><th>Parameter</th><th>Value</th>
								{#if target !== 'preset'}
									<th title="Empty uses the cue's own fade">Fade</th>
									<th>Delay</th>
									<th>Curve</th>
								{/if}
							</tr>
						</thead>
						<tbody>
							{#each $entries as entry (entry.id)}
								<tr class:off={!include.has(entry.id)}>
									<td>
										<input
											type="checkbox"
											checked={include.has(entry.id)}
											aria-label="Store {nameOf(entry.fixture_id)} {displayLabel(entry.parameter_kind)}"
											onchange={(e) => tick(entry.id, e.currentTarget.checked)}
										/>
									</td>
									<td>{nameOf(entry.fixture_id)}</td>
									<td>{displayLabel(entry.parameter_kind)}</td>
									<td class="mono">
										{#if entry.effect}
											<!-- What is stored is the shape, not the value under it. -->
											<span class="chip-effect">
												{'Steps' in entry.effect.curve
													? `${entry.effect.curve.Steps.length} steps`
													: entry.effect.curve.Shape.toLowerCase()}
											</span>
										{:else}
											{formatValue(entry.value)}
										{/if}
									</td>
									{#if target !== 'preset'}
										<td>
											<input
												class="num"
												type="number"
												min="0"
												step="100"
												placeholder="cue"
												value={timingFor(entry.id).fade || ''}
												aria-label="Fade for {displayLabel(entry.parameter_kind)}"
												onchange={(e) => setTiming(entry.id, { fade: Number(e.currentTarget.value) || 0 })}
											/>
										</td>
										<td>
											<input
												class="num"
												type="number"
												min="0"
												step="100"
												placeholder="0"
												value={timingFor(entry.id).delay || ''}
												aria-label="Delay for {displayLabel(entry.parameter_kind)}"
												onchange={(e) => setTiming(entry.id, { delay: Number(e.currentTarget.value) || 0 })}
											/>
										</td>
										<td>
											<select
												value={timingFor(entry.id).easing ?? ''}
												aria-label="Curve for {displayLabel(entry.parameter_kind)}"
												onchange={(e) =>
													setTiming(entry.id, {
														easing: (e.currentTarget.value || null) as Easing | null
													})}
											>
												<!-- Named rather than left as "inherited": an operator
												     deciding whether to override wants to see what they
												     would be overriding. -->
												<option value="">{inheritedFrom(entry.parameter_kind)}</option>
												{#each CURVES as curve (curve)}
													<option value={curve}>{CURVE_LABELS[curve]}</option>
												{/each}
											</select>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</section>

			<!-- Where: a cue, new or existing, in a sequence — or a preset. -->
			<section class="where">
				<div class="seg" role="radiogroup" aria-label="Store as">
					<button role="radio" aria-checked={target !== 'preset'} class:on={target !== 'preset'} onclick={() => target !== 'preset' || (target = 'new')}>Cue</button>
					<button role="radio" aria-checked={target === 'preset'} class:on={target === 'preset'} onclick={() => (target = 'preset')}>Preset</button>
				</div>

				{#if target === 'preset'}
					<label class="field">
						<span>Into</span>
						<select value={presetTarget ?? ''} onchange={(e) => (presetTarget = e.currentTarget.value || null)}>
							<option value="">A new preset…</option>
							{#each $presets as p (p.id)}
								<option value={p.id}>{p.name}</option>
							{/each}
						</select>
					</label>
					{#if presetTarget === null}
						<label class="field">
							<span>Name</span>
							<input class="text" placeholder="Preset name…" bind:value={presetName} use:focusOnMount />
						</label>
					{:else}
						<p class="note">
							Merged into it: everything it already says about other parameters is kept, and every
							cue that uses it follows at once.
						</p>
					{/if}
				{:else}
					<label class="field">
						<span>Sequence</span>
						<select
							value={creatingSequence ? NEW_SEQUENCE : (sequence?.id ?? '')}
							onchange={(e) => {
								sequenceId = e.currentTarget.value;
								cueId = null;
								if (sequenceId === NEW_SEQUENCE) target = 'new';
							}}
						>
							{#each $sequences as s (s.id)}
								<option value={s.id}>{s.name}</option>
							{/each}
							<option value={NEW_SEQUENCE}>+ New sequence…</option>
						</select>
					</label>
					{#if creatingSequence}
						<label class="field">
							<span>Called</span>
							<input class="text" bind:value={sequenceName} placeholder="Sequence name…" />
						</label>
					{/if}
					{#if onto && onto.sequence_id === null}
						<p class="note">
							The wing's {onto.control.kind === 'Fader' ? 'fader' : 'key'} gets the sequence this is stored into,
							on page {onto.page}.
						</p>
					{/if}

					<div class="seg" role="radiogroup" aria-label="Which cue">
						<button role="radio" aria-checked={target === 'new'} class:on={target === 'new'} onclick={() => (target = 'new')}>New cue</button>
						<button
							role="radio"
							aria-checked={target === 'existing'}
							class:on={target === 'existing'}
							disabled={cuesInSequence.length === 0}
							title={cuesInSequence.length === 0 ? 'This sequence has no cues yet' : undefined}
							onclick={() => (target = 'existing')}>Existing cue</button
						>
					</div>

					{#if target === 'new'}
						<label class="field">
							<span>Name</span>
							<input
								class="text"
								placeholder="Cue name…"
								value={name}
								oninput={(e) => (typedName = e.currentTarget.value)}
								use:focusOnMount
								onfocus={(e) => e.currentTarget.select()}
								onkeydown={(e) => e.key === 'Enter' && canStore && store()}
							/>
						</label>
						<!-- Where it goes. Preselected after the cue that is up, which is where
						     somebody programming a show in order means. -->
						<label class="field">
							<span>After</span>
							<select value={insertAfter ?? ''} onchange={(e) => (after = e.currentTarget.value || null)}>
								<option value="">The end of the list</option>
								{#each cuesInSequence as c (c.id)}
									<option value={c.id}>
										{c.number.toFixed(1)} · {c.name}{c.id === afterActive ? ' (up now)' : ''}
									</option>
								{/each}
							</select>
						</label>
						<!-- The cue's own timing, which every value on the left falls back to. -->
						<fieldset class="timing">
							<legend>Timing</legend>
							<label class="field">
								<span>Fade in</span>
								<input class="num" type="number" min="0" step="100" bind:value={cueFadeIn} />
								<span class="unit">ms</span>
							</label>
							<label class="field" title="How long what this cue sets takes to go when the cue ends — going back past it, or Off">
								<span>Fade out</span>
								<input class="num" type="number" min="0" step="100" bind:value={cueFadeOut} />
								<span class="unit">ms</span>
							</label>
							<label class="field">
								<span>Curve</span>
								<select bind:value={cueEasing}>
									<!-- The show's own answer, which is different per parameter —
									     so this one says where it comes from rather than naming a
									     curve the intensities in this cue would not take. -->
									<option value={null}>The show's</option>
									{#each CURVES as curve (curve)}
										<option value={curve}>{CURVE_LABELS[curve]}</option>
									{/each}
								</select>
							</label>
							<label class="field">
								<span>Follow</span>
								<select bind:value={followMode}>
									<option value="Manual">On Go</option>
									<option value="FollowAfter">Automatically</option>
								</select>
							</label>
							{#if followMode !== 'Manual'}
								<label class="field">
									<span>after</span>
									<input class="num" type="number" min="0" step="100" bind:value={followDelay} />
									<span class="unit">ms</span>
								</label>
							{/if}
						</fieldset>
					{:else}
						<label class="field">
							<span>Cue</span>
							<select bind:value={cueId}>
								<option value={null}>Choose a cue…</option>
								{#each cuesInSequence as c (c.id)}
									<option value={c.id}>{c.number.toFixed(1)} · {c.name}</option>
								{/each}
							</select>
						</label>
						<div class="seg" role="radiogroup" aria-label="Merge or replace">
							<button role="radio" aria-checked={mode === 'merge'} class:on={mode === 'merge'} onclick={() => (mode = 'merge')}>Merge</button>
							<button role="radio" aria-checked={mode === 'replace'} class:on={mode === 'replace'} onclick={() => (mode = 'replace')}>Replace</button>
						</div>
						<p class="note">
							{mode === 'merge' ? 'Everything else the cue says is kept.' : 'The cue will say only what is ticked.'}
						</p>
					{/if}

					<!-- Track or cue only, which is the difference between changing the look
					     and fixing one moment of it. -->
					<div class="seg" role="radiogroup" aria-label="Track or cue only">
						<button role="radio" aria-checked={tracking === 'track'} class:on={tracking === 'track'} onclick={() => (tracking = 'track')}>Track</button>
						<button role="radio" aria-checked={tracking === 'cueOnly'} class:on={tracking === 'cueOnly'} onclick={() => (tracking = 'cueOnly')}>Cue only</button>
					</div>
					<p class="note">
						{tracking === 'track'
							? 'The change carries forward until a later cue says otherwise.'
							: 'The next cue is given what it is showing now, so the change stops here.'}
					</p>
				{/if}

				<label class="check">
					<input type="checkbox" bind:checked={keep} />
					Keep the programmer after storing
				</label>
			</section>
		</div>
	{/if}
</Dialog>

<style>
	.empty {
		padding: 20px 14px;
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
	}

	/* What on the left, where on the right: the list is what grows, and the choices
	   about where it goes are a fixed column beside it. */
	/* A fixed height, with each column scrolling inside it: switching between a cue
	   and a preset changes how much is on the right, and a dialog centered on the
	   screen that grew and shrank with it jumped under the pointer. */
	.store {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 340px;
		height: min(620px, calc(100dvh - 180px));
		min-height: 0;
	}
	@media (max-width: 860px) {
		.store {
			grid-template-columns: minmax(0, 1fr);
			height: auto;
		}
	}
	h3 {
		margin: 0 0 8px;
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	h3 .dim {
		color: var(--text-faint);
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
		margin-left: 6px;
	}
	.what {
		display: flex;
		flex-direction: column;
		padding: 14px;
		min-width: 0;
		min-height: 0;
		border-right: 1px solid var(--line);
	}
	.where {
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 14px;
		min-height: 0;
		overflow-y: auto;
		background: color-mix(in srgb, var(--bg-sunken) 40%, transparent);
	}

	.list {
		flex: 1;
		min-height: 0;
		overflow: auto;
		border: 1px solid var(--line);
		border-radius: var(--radius);
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	th {
		position: sticky;
		top: 0;
		z-index: 1;
		background: var(--bg-panel);
		text-align: left;
		font-weight: 500;
		font-size: var(--font-xs);
		color: var(--text-dim);
		padding: 6px 8px;
		border-bottom: 1px solid var(--line);
	}
	td {
		padding: 4px 8px;
		border-bottom: 1px solid #ffffff08;
	}
	tr.off td {
		color: var(--text-faint);
	}
	.mono {
		font-family: monospace;
	}

	/* A label beside what it sets, every one the same width, so the column reads down. */
	.field {
		display: grid;
		grid-template-columns: 72px minmax(0, 1fr) auto;
		align-items: center;
		gap: 8px;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.field > :nth-child(2):last-child {
		grid-column: 2 / -1;
	}

	/* Either/or, as one control: the choice is visible without opening anything. */
	.seg {
		display: flex;
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		overflow: hidden;
	}
	.seg button {
		flex: 1;
		padding: 7px 10px;
		border: none;
		border-right: 1px solid var(--line-strong);
		background: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.seg button:last-child {
		border-right: none;
	}
	.seg button:hover:not(:disabled) {
		background: var(--bg-hover);
		color: var(--text-bright);
	}
	.seg button.on {
		background: color-mix(in srgb, var(--accent) 22%, transparent);
		color: var(--text-bright);
	}
	.seg button:disabled {
		color: var(--text-faint);
		cursor: default;
	}

	.timing {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin: 0;
		padding: 10px 12px 12px;
		border: 1px solid var(--line);
		border-radius: var(--radius);
	}
	legend {
		padding: 0 4px;
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}

	.check {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--text-dim);
		font-size: var(--font-sm);
		cursor: pointer;
		margin-top: auto;
	}
	.note {
		margin: -4px 0 0;
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	.text {
		width: 100%;
		min-height: 28px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 0 8px;
	}
	.text:focus {
		outline: none;
		border-color: var(--accent);
	}
	.field select {
		width: 100%;
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 5px 14px;
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.ghost:hover {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.primary {
		background: var(--accent-solid);
		border: none;
		border-radius: var(--radius);
		color: #fff;
		padding: 6px 16px;
		font: inherit;
		font-size: var(--font-sm);
		font-weight: 500;
		cursor: pointer;
	}
	.primary:disabled {
		background: var(--line-strong);
		color: var(--text-faint);
		cursor: not-allowed;
	}
	.num {
		width: 5.5rem;
		min-height: 26px;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: 12px;
		padding: 0 6px;
	}
	td .num {
		width: 4.5rem;
	}
	.unit {
		color: var(--text-faint);
		font-size: 11px;
	}
	.chip-effect {
		font-size: 10px;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--live);
		color: var(--live);
	}
</style>
