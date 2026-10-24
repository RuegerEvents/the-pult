<script lang="ts">
	/**
	 * The command wing: whether one is here, whether this page drives it, what its
	 * encoders are on, and what its controls are bound to.
	 *
	 * A panel rather than a Setup errand because the encoder bar is what an operator
	 * reads while turning — the wing has no screen of its own — and a binding is made
	 * by touching the control, which is something done in the middle of programming.
	 */

	import type { BindingAction, ParameterKind, SurfaceControl, TileTarget } from '$lib/generated/index.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { editableParameters } from '$lib/programmer.js';
	import { selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openConsoleLine } from '$lib/stores/plugins.js';
	import {
		bind,
		controlKey,
		controlName,
		driving,
		drivesHere,
		encoderBar,
		learned,
		learning,
		selectedParameter,
		setDrivesHere,
		slotValue,
		unbind,
		wingBindings,
		wingLine,
		wingMap,
		wingState
	} from '$lib/stores/wing.js';
	import { output } from '$lib/stores/output.js';
	import { pressableTargets, type SurfaceShow } from '$lib/surface.js';
	import { slotReading } from '$lib/wing.js';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const sequences = collection('sequences');
	const timelines = collection('timelines');
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

	const phase = $derived($wingState.phase);
	const phaseText = $derived.by(() => {
		switch (phase.kind) {
			case 'Off':
				return 'This station was started without a wing (--no-wing, or a test).';
			case 'Absent':
				return 'No command wing is plugged into this station.';
			case 'Booting':
				return 'Starting the wing…';
			case 'Loading':
				return `Giving the wing its application: ${Math.round((phase.sent / Math.max(1, phase.total)) * 100)}%`;
			case 'Running':
				return `${$wingState.device ?? 'Command wing'} is up.`;
			case 'Fault':
				return phase.reason;
		}
	});
	const someoneElse = $derived($wingState.claimed_by !== null && !$driving);

	const fixtureName = (id: string) => $fixtures.find((f) => f.id === id)?.name ?? id.slice(0, 6);

	function describe(action: BindingAction): string {
		switch (action.kind) {
			case 'Press':
				return pressable.find((p) => JSON.stringify(p.target) === JSON.stringify(action.target))?.label ?? 'something no longer in the show';
			case 'CommandLine':
				return `types “${action.line}”`;
			case 'Flash':
				return `flashes ${fixtureName(action.fixture_id)} · ${displayLabel(action.parameter)}`;
			case 'SelectGroup':
				return `selects ${$groups.find((g) => g.id === action.group_id)?.name ?? 'a group no longer in the show'}`;
			case 'Parameter':
				return `${fixtureName(action.fixture_id)} · ${displayLabel(action.parameter)}`;
			case 'Master':
				return `level of ${$sequences.find((s) => s.id === action.sequence_id)?.name ?? 'a sequence no longer in the show'}`;
		}
	}

	// ── The editor for the control learning caught ──
	type Choice = 'Press' | 'CommandLine' | 'Flash' | 'SelectGroup' | 'Parameter' | 'Master';
	const choicesFor = (c: SurfaceControl): { kind: Choice; label: string }[] =>
		c.kind === 'Key'
			? [
					{ kind: 'Press', label: 'Press something' },
					{ kind: 'CommandLine', label: 'Run a command line' },
					{ kind: 'Flash', label: 'Flash a parameter' },
					{ kind: 'SelectGroup', label: 'Select a group' }
				]
			: c.kind === 'Fader'
				? [
						{ kind: 'Parameter', label: 'A parameter' },
						{ kind: 'Master', label: 'A sequence’s level' }
					]
				: [{ kind: 'Parameter', label: 'A parameter' }];

	let choice = $state<Choice>('Press');
	let pressIndex = $state(0);
	let line = $state('');
	let groupId = $state('');
	let sequenceId = $state('');
	let parameterKeyChosen = $state('');

	$effect(() => {
		const c = $learned;
		if (!c) return;
		const existing = $wingBindings.get(controlKey(c));
		choice = (existing?.action.kind as Choice | undefined) ?? choicesFor(c)[0].kind;
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
			case 'Press': {
				const target: TileTarget | undefined = pressable[pressIndex]?.target;
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
				const row = selectedRows.find((r) => r.key === parameterKeyChosen) ?? selectedRows[0];
				if (!row) return null;
				const picked = selectedParameter(row.kind as ParameterKind);
				return picked ? { kind: choice, ...picked } : null;
			}
		}
	}

	const formAction = $derived(actionFromForm());

	async function save() {
		const c = $learned;
		if (!c || !formAction) return;
		await bind(c, formAction);
		learned.set(null);
	}

	/** What a bound fader is at right now, for the list. */
	function faderReading(action: BindingAction): string {
		if (action.kind === 'Master') {
			const s = $sequences.find((q) => q.id === action.sequence_id);
			return s ? `${Math.round((s.master ?? 1) * 100)}%` : '';
		}
		if (action.kind === 'Parameter') {
			return slotReading($output.value(action.fixture_id, parameterKey(action.parameter)));
		}
		return '';
	}

	const typedHere = $derived(openConsoleLine() ? null : $wingLine);
</script>

<div class="wing">
	<section class="status" class:fault={phase.kind === 'Fault'}>
		<p>{phaseText}</p>
		{#if phase.kind !== 'Off'}
			<label class="drive">
				<input
					type="checkbox"
					checked={$drivesHere}
					onchange={(e) => void setDrivesHere(e.currentTarget.checked)}
				/>
				Drive the wing from this page
			</label>
			{#if someoneElse}
				<p class="dim">Another page ({$wingState.claimed_by}) is driving it. Ticking the box takes it over.</p>
			{:else if !$driving && $wingState.claimed_by === null && phase.kind === 'Running'}
				<p class="dim">No page is driving it, so it is dark and does nothing.</p>
			{/if}
		{/if}
	</section>

	{#if $driving}
		<section>
			<h3>
				Encoders
				<span class="dim">page {$encoderBar.page + 1} of {$encoderBar.pages} · UP and DOWN to turn the page</span>
			</h3>
			<div class="encoders">
				{#each $encoderBar.slots as slot, i (i)}
					<div class="encoder" class:empty={!slot}>
						{#if slot}
							<span class="name">{slot.label}</span>
							<span class="value mono">{slotReading(slotValue(slot), slot.component)}</span>
						{:else}
							<span class="name dim">—</span>
						{/if}
					</div>
				{/each}
			</div>
			{#if $selection.length === 0}
				<p class="dim">Select something and the encoders turn its attributes: outer ring coarse, inner knob fine.</p>
			{/if}
		</section>

		{#if typedHere !== null}
			<section>
				<h3>Command line <span class="dim">no command line panel is open, so the wing types here</span></h3>
				<pre class="line mono">› {typedHere}</pre>
			</section>
		{/if}

		<section>
			<div class="section-head">
				<h3>Bindings</h3>
				{#if $canEdit}
					<button
						class="btn {$learning ? 'btn-primary learning' : 'btn-ghost'}"
						onclick={() => learning.update((on) => !on)}
					>
						{$learning ? 'Touch a control on the wing…' : 'Learn'}
					</button>
				{/if}
			</div>

			{#if $learned}
				<div class="editor">
					<p class="how">
						<strong>{controlName($learned)}</strong> is waiting. Move what it should
						control — a parameter in the Programmer, a sequence's Level, Go or Off on a
						sequence, a surface tile — and it is bound. Or choose here:
					</p>
					<select class="input" bind:value={choice}>
						{#each choicesFor($learned) as c (c.kind)}
							<option value={c.kind}>{c.label}</option>
						{/each}
					</select>
					{#if choice === 'Press'}
						<select class="input" bind:value={pressIndex}>
							{#each pressable as p, i (i)}<option value={i}>{p.label}</option>{/each}
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
						<span class="dim">Select the fixture first: this binds to what is selected.</span>
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
						{#if $wingBindings.has(controlKey($learned))}
							<button class="btn btn-danger" onclick={() => { if ($learned) void unbind($learned); learned.set(null); }}>Unbind</button>
						{/if}
						<button class="btn btn-ghost" onclick={() => learned.set(null)}>Cancel</button>
					</div>
				</div>
			{/if}

			{#if $wingBindings.size === 0}
				<p class="dim">
					Nothing bound. Press LEARN on the wing (or Learn here), touch a key, fader or encoder, then
					move what it should control.
				</p>
			{:else}
				<ul class="bindings">
					{#each [...$wingBindings.values()] as b (b.id)}
						<li>
							<span class="control">{controlName(b.control)}</span>
							<span class="action">{b.label || describe(b.action)}</span>
							{#if b.control.kind === 'Fader'}<span class="mono dim">{faderReading(b.action)}</span>{/if}
							{#if $canEdit}
								<button class="btn btn-ghost" onclick={() => learned.set(b.control)}>Edit</button>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}
</div>

<style>
	.wing {
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 14px;
		font-size: 0.85rem;
	}
	h3 {
		font-size: 0.72rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim, #888);
		margin: 0 0 6px;
		display: flex;
		gap: 8px;
		align-items: center;
	}
	.section-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 6px;
	}
	.section-head h3 {
		margin: 0;
	}
	.dim {
		color: var(--text-dim, #888);
		text-transform: none;
		letter-spacing: 0;
	}
	.status p {
		margin: 0 0 6px;
	}
	.status.fault p:first-child {
		color: var(--danger, #ef4444);
	}
	.drive {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	.encoders {
		display: grid;
		grid-template-columns: repeat(5, 1fr);
		gap: 6px;
	}
	.encoder {
		border: 1px solid var(--border, #333);
		border-radius: 6px;
		padding: 6px;
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-height: 44px;
	}
	.encoder.empty {
		opacity: 0.4;
	}
	.encoder .name {
		font-size: 0.75rem;
	}
	.encoder .value {
		font-size: 1rem;
	}
	.line {
		margin: 0;
		padding: 6px 8px;
		background: var(--surface-2, #1a1a1a);
		border-radius: 4px;
	}
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
	.bindings {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.bindings li {
		display: flex;
		gap: 8px;
		align-items: center;
	}
	.bindings .control {
		min-width: 9em;
		color: var(--text-dim, #888);
	}
	.bindings .action {
		flex: 1;
	}
	.how {
		flex-basis: 100%;
		margin: 0;
		color: var(--text-dim, #999);
	}
	.how strong {
		color: var(--text, #e0e0e0);
	}
	.actions {
		display: flex;
		gap: 6px;
		margin-left: auto;
	}
	/* Learning is a mode the desk is in, so it says so the way the wing's own key does. */
	.learning {
		animation: blink 0.6s steps(2, start) infinite;
	}
	@keyframes blink {
		50% {
			opacity: 0.55;
		}
	}
</style>
