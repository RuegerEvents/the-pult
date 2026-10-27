<script lang="ts">
	/**
	 * What one wing control does, as a form: what kind of thing, which one, and — for a
	 * sequence — Go, Off, Flash or Go to cue. Opened by LEARN in the Wing panel and by
	 * EDIT on the wing; both are the same question about the same control.
	 */

	import { untrack } from 'svelte';
	import type { BindingAction, Cue, ParameterKind, SurfaceControl, TileTarget } from '$lib/generated/index.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { editableParameters } from '$lib/programmer.js';
	import { selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { bind, controlKey, controlName, selectedParameter, unbind, wingBindings } from '$lib/stores/wing.js';
	import { pressableTargets, type SurfaceShow } from '$lib/surface.js';

	let { control, onclose }: { control: SurfaceControl; onclose: () => void } = $props();

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const sequences = collection('sequences');
	const timelines = collection('timelines');
	const cues = collection('cues');
	const variables = collection('variables');
	const flows = collection('flows');
	const flowNodes = collection('flow_nodes');
	const groups = collection('groups');

	const show: SurfaceShow = $derived({
		variables: $variables,
		timelines: $timelines,
		sequences: $sequences,
		flows: $flows,
		flowNodes: $flowNodes,
		fixtures: $fixtures,
		types: $types
	});
	const pressable = $derived(pressableTargets(show));
	const fixtureName = (id: string) => $fixtures.find((f) => f.id === id)?.name ?? id.slice(0, 6);

		// What a control does, chosen in steps: what kind of thing, which one, and — for a
	// sequence — Go, Off, Flash or Go to cue, so the sequences are listed once and not
	// once per act. Opening a control that is bound fills every step from what it does
	// now, rather than starting over on the first sequence's Go.
	type Choice = 'Sequence' | 'Timeline' | 'Press' | 'CommandLine' | 'Flash' | 'SelectGroup' | 'Parameter' | 'Master';
	type SequenceAct = 'Go' | 'Off' | 'Flash' | 'GoToCue';
	const SEQUENCE_ACTS: [SequenceAct, string][] = [
		['Go', 'Go'],
		['Off', 'Off'],
		['Flash', 'Flash'],
		['GoToCue', 'Go to cue']
	];
	const choicesFor = (c: SurfaceControl): { kind: Choice; label: string }[] =>
		c.kind === 'Key' || c.kind === 'Digital'
			? [
					{ kind: 'Sequence', label: 'A sequence' },
					{ kind: 'Timeline', label: 'A timeline' },
					{ kind: 'Press', label: 'Something else' },
					{ kind: 'CommandLine', label: 'Run a command line' },
					{ kind: 'Flash', label: 'Flash a parameter' },
					{ kind: 'SelectGroup', label: 'Select a group' }
				]
			: [
					{ kind: 'Parameter', label: 'A parameter' },
					{ kind: 'Master', label: 'A sequence’s level' }
				];

	/** "Something else": what a surface tile can press, less what has its own choice. */
	const others = $derived(
		pressable.filter(
			(p) => p.target.kind !== 'SequenceGo' && p.target.kind !== 'SequenceOff' && p.target.kind !== 'Timeline'
		)
	);

	let choice = $state<Choice>('Sequence');
	let sequenceId = $state('');
	let sequenceAct = $state<SequenceAct>('Go');
	let cueId = $state('');
	let timelineId = $state('');
	let fromStart = $state(false);
	let pressIndex = $state(0);
	let line = $state('');
	let groupId = $state('');
	let parameterKeyChosen = $state('');
	/** The parameter a control is bound to now, kept until the selection offers another. */
	let boundParameter = $state<{ fixture_id: string; parameter: ParameterKind } | null>(null);

	$effect(() => {
		const c = control;
		untrack(() => fill(c, $wingBindings.get(controlKey(c))?.action ?? null));
	});

	/** The form, set to what a control does now — or to the start, for one bound to nothing. */
	function fill(c: SurfaceControl, action: BindingAction | null) {
		choice = choicesFor(c)[0].kind;
		sequenceId = '';
		sequenceAct = 'Go';
		cueId = '';
		timelineId = '';
		fromStart = false;
		pressIndex = 0;
		line = '';
		groupId = '';
		parameterKeyChosen = '';
		boundParameter = null;
		if (!action) return;
		switch (action.kind) {
			case 'Press': {
				const t = action.target;
				if (t.kind === 'SequenceGo' || t.kind === 'SequenceOff') {
					choice = 'Sequence';
					sequenceId = t.sequence_id;
					sequenceAct = t.kind === 'SequenceGo' ? 'Go' : 'Off';
				} else if (t.kind === 'Timeline') {
					choice = 'Timeline';
					timelineId = t.timeline_id;
					fromStart = t.from_start;
				} else {
					choice = 'Press';
					pressIndex = Math.max(0, others.findIndex((p) => JSON.stringify(p.target) === JSON.stringify(t)));
				}
				return;
			}
			case 'FlashSequence':
				choice = 'Sequence';
				sequenceId = action.sequence_id;
				sequenceAct = 'Flash';
				return;
			case 'GoToCue':
				choice = 'Sequence';
				sequenceId = action.sequence_id;
				sequenceAct = 'GoToCue';
				cueId = action.cue_id;
				return;
			case 'CommandLine':
				choice = 'CommandLine';
				line = action.line;
				return;
			case 'SelectGroup':
				choice = 'SelectGroup';
				groupId = action.group_id;
				return;
			case 'Master':
				choice = 'Master';
				sequenceId = action.sequence_id;
				return;
			case 'Flash':
			case 'Parameter':
				choice = action.kind;
				boundParameter = { fixture_id: action.fixture_id, parameter: action.parameter };
				parameterKeyChosen = parameterKey(action.parameter);
				return;
		}
	}

	/** The cues of the sequence chosen, in its order, for Go to cue. */
	const cuesOfSequence = $derived.by((): Cue[] => {
		const seq = $sequences.find((s) => s.id === sequenceId);
		if (!seq) return [];
		const byId = new Map($cues.map((c) => [c.id, c]));
		return seq.cue_ids.map((id) => byId.get(id)).filter((c): c is Cue => !!c);
	});

	/** The selection's parameters, for "bind to what is selected". */
	const selectedRows = $derived(
		editableParameters(
			$types,
			$fixtures.filter((f) => $selection.includes(f.id))
		)
	);

	function actionFromForm(): BindingAction | null {
		switch (choice) {
			case 'Sequence': {
				if (!sequenceId) return null;
				switch (sequenceAct) {
					case 'Go':
						return { kind: 'Press', target: { kind: 'SequenceGo', sequence_id: sequenceId } };
					case 'Off':
						return { kind: 'Press', target: { kind: 'SequenceOff', sequence_id: sequenceId } };
					case 'Flash':
						return { kind: 'FlashSequence', sequence_id: sequenceId };
					case 'GoToCue':
						return cueId ? { kind: 'GoToCue', sequence_id: sequenceId, cue_id: cueId } : null;
				}
				return null;
			}
			case 'Timeline':
				return timelineId
					? { kind: 'Press', target: { kind: 'Timeline', timeline_id: timelineId, from_start: fromStart } }
					: null;
			case 'Press': {
				const target: TileTarget | undefined = others[pressIndex]?.target;
				return target ? { kind: 'Press', target } : null;
			}
			case 'CommandLine':
				return line.trim() ? { kind: 'CommandLine', line: line.trim() } : null;
			case 'SelectGroup':
				return groupId ? { kind: 'SelectGroup', group_id: groupId } : null;
			case 'Master':
				return sequenceId ? { kind: 'Master', sequence_id: sequenceId } : null;
			case 'Flash':
			case 'Parameter': {
				// What is selected, where it offers the parameter; otherwise what the control
				// is bound to now, so opening it and pressing Bind changes nothing.
				const row = selectedRows.find((r) => r.key === parameterKeyChosen) ?? selectedRows[0];
				const picked = row ? selectedParameter(row.kind as ParameterKind) : null;
				const target = picked ?? boundParameter;
				return target ? { kind: choice, ...target } : null;
			}
		}
	}

	const formAction = $derived(actionFromForm());

	async function save() {
		if (!formAction) return;
		await bind(control, formAction);
		onclose();
	}
</script>

	<div class="editor">
		<p class="how">
			<strong>{controlName(control)}</strong> is waiting. Move what it should
			control — a parameter in the Programmer, a sequence's Level, Go or Off on a
			sequence, a surface tile — and it is bound. Or choose here:
		</p>
		<select class="input" bind:value={choice}>
			{#each choicesFor(control) as c (c.kind)}
				<option value={c.kind}>{c.label}</option>
			{/each}
		</select>
		{#if choice === 'Sequence'}
			<select class="input" bind:value={sequenceId} onchange={() => (cueId = '')}>
				<option value="">Which sequence?</option>
				{#each $sequences as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
			</select>
			<div class="segmented" role="group" aria-label="What the key does">
				{#each SEQUENCE_ACTS as [act, label] (act)}
					<button class:on={sequenceAct === act} aria-pressed={sequenceAct === act} onclick={() => (sequenceAct = act)}
						>{label}</button
					>
				{/each}
			</div>
			{#if sequenceAct === 'GoToCue'}
				<select class="input" bind:value={cueId} disabled={!sequenceId}>
					<option value="">Which cue?</option>
					{#each cuesOfSequence as c (c.id)}<option value={c.id}>{c.number.toFixed(1)} · {c.name}</option>{/each}
				</select>
			{:else if sequenceAct === 'Flash'}
				<span class="dim">Held: on at full. Let go: off again, on the cue's own times.</span>
			{/if}
		{:else if choice === 'Timeline'}
			<select class="input" bind:value={timelineId}>
				<option value="">Which timeline?</option>
				{#each $timelines as t (t.id)}<option value={t.id}>{t.name}</option>{/each}
			</select>
			<div class="segmented" role="group" aria-label="What the key does">
				<button class:on={!fromStart} aria-pressed={!fromStart} onclick={() => (fromStart = false)}>Play / stop</button>
				<button class:on={fromStart} aria-pressed={fromStart} onclick={() => (fromStart = true)}>Start from the beginning</button>
			</div>
		{:else if choice === 'Press'}
			<select class="input" bind:value={pressIndex}>
				{#each others as p, i (i)}<option value={i}>{p.label}</option>{/each}
			</select>
		{:else if choice === 'CommandLine'}
			<input class="input mono" placeholder="group 3 at 50" bind:value={line} />
		{:else if choice === 'SelectGroup'}
			<select class="input" bind:value={groupId}>
				<option value="">Which group?</option>
				{#each $groups as g (g.id)}<option value={g.id}>{g.name}</option>{/each}
			</select>
		{:else if choice === 'Master'}
			<select class="input" bind:value={sequenceId}>
				<option value="">Which sequence?</option>
				{#each $sequences as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
			</select>
		{:else if selectedRows.length === 0}
			{#if boundParameter}
				<span class="dim"
					>{fixtureName(boundParameter.fixture_id)} · {displayLabel(boundParameter.parameter)} — select fixtures
					to bind it to something else.</span
				>
			{:else}
				<span class="dim">Select the fixture first: this binds to what is selected.</span>
			{/if}
		{:else}
			<select class="input" bind:value={parameterKeyChosen}>
				{#each selectedRows as r (r.key)}<option value={r.key}>{r.label}</option>{/each}
			</select>
			{#if formAction && (formAction.kind === 'Parameter' || formAction.kind === 'Flash')}
				<span class="dim">{fixtureName(formAction.fixture_id)}</span>
			{/if}
		{/if}
		<div class="actions">
			<button class="btn btn-primary" disabled={!formAction} onclick={() => void save()}>Bind</button>
			{#if $wingBindings.has(controlKey(control))}
				<button class="btn btn-danger" onclick={() => { void unbind(control); onclose(); }}>Unbind</button>
			{/if}
			<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		</div>
	</div>

<style>
	.editor {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		padding: 8px;
		border: 1px solid var(--accent, #4a9eff);
		border-radius: 6px;
		margin-bottom: 8px;
	}
	.how {
		flex-basis: 100%;
		margin: 0;
		color: var(--text-dim, #999);
	}
	.how strong {
		color: var(--text, #e0e0e0);
	}
	.dim {
		color: var(--text-dim, #888);
	}
	.actions {
		display: flex;
		gap: 6px;
		margin-left: auto;
	}
</style>
