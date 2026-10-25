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
		calculator,
		controlKey,
		controlName,
		driving,
		drivesHere,
		encoderBar,
		learned,
		learning,
		openCalculator,
		selectedParameter,
		setTyped,
		setDrivesHere,
		setWingAt,
		slotValue,
		unbind,
		wingBindings,
		wingLine,
		wingMap,
		wingAt,
		wingStations,
		wingState
	} from '$lib/stores/wing.js';
	import { output } from '$lib/stores/output.js';
	import { pressableTargets, type SurfaceShow } from '$lib/surface.js';
	import { DIGITAL_INPUTS, slotReading } from '$lib/wing.js';
	import ToggleControl from '$lib/components/programmer/controls/ToggleControl.svelte';

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
	/** The station being followed, when it is not this page's own. */
	const elsewhere = $derived($wingAt ? ($wingStations.find((s) => s.id === $wingAt)?.hostname ?? null) : null);
	/** Only worth asking which wing when there is a wing somewhere other than here. */
	const choosing = $derived($wingAt !== null || $wingStations.some((s) => !s.here));
	const phaseText = $derived.by(() => {
		switch (phase.kind) {
			case 'Off':
				return $wingAt
					? `Waiting for ${elsewhere ?? 'that station'} to answer.`
					: 'This station was started without a wing (--no-wing, or a test).';
			case 'Absent':
				return `No command wing is plugged into ${$wingAt ? (elsewhere ?? 'that station') : 'this station'}.`;
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

	/** The phase as a badge: a word and a color, read at a glance. */
	const badge = $derived.by((): { text: string; tone: 'good' | 'busy' | 'idle' | 'bad' } => {
		switch (phase.kind) {
			case 'Running':
				return { text: 'Up', tone: 'good' };
			case 'Booting':
				return { text: 'Starting', tone: 'busy' };
			case 'Loading':
				return { text: `Loading ${Math.round((phase.sent / Math.max(1, phase.total)) * 100)}%`, tone: 'busy' };
			case 'Absent':
				return { text: 'Not plugged in', tone: 'idle' };
			case 'Off':
				return { text: $wingAt ? 'Not answering' : 'Off', tone: 'idle' };
			case 'Fault':
				return { text: 'Fault', tone: 'bad' };
		}
	});
	const present = $derived(phase.kind !== 'Off' && phase.kind !== 'Absent' && phase.kind !== 'Fault');

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
		c.kind === 'Key' || c.kind === 'Digital'
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
	<!-- One line that says what the wing is doing and what this page is doing about
	     it; the sentences only where there is something to explain. -->
	<section class="status">
		<div class="head">
			<span class="badge {badge.tone}">{badge.text}</span>
			<span class="device">{phase.kind === 'Running' ? ($wingState.device ?? 'Command wing') : 'Command wing'}</span>
			{#if elsewhere}<span class="dim">on {elsewhere}</span>{/if}
			<span class="spacer"></span>
			{#if choosing}
				<label class="at">
					<span class="dim">Wing on</span>
					<select value={$wingAt ?? ''} onchange={(e) => setWingAt(e.currentTarget.value || null)}>
						<option value="">this station</option>
						{#each $wingStations.filter((s) => !s.here) as s (s.id)}
							<option value={s.id}>{s.hostname} ({s.state.phase.kind.toLowerCase()})</option>
						{/each}
						{#if $wingAt && !elsewhere}
							<option value={$wingAt}>a station that has gone</option>
						{/if}
					</select>
				</label>
			{/if}
			<!-- Only a wing that is here can be driven: offering to drive an absent or
			     faulty one would claim nothing and say it had. -->
			{#if present}
				<span class="drive">
					<span class="dim">Drive from this page</span>
					<ToggleControl label="Drive the wing from this page" value={$drivesHere} oninput={(on) => void setDrivesHere(on)} />
				</span>
			{/if}
		</div>
		{#if phase.kind === 'Fault'}
			<p class="fault">{phase.reason}</p>
		{:else if phase.kind !== 'Running'}
			<p class="dim">{phaseText}</p>
		{:else if someoneElse}
			<p class="dim">Another page ({$wingState.claimed_by}) is driving it. Switching Drive on takes it over.</p>
		{:else if !$driving && $wingState.claimed_by === null}
			<p class="dim">No page is driving it, so it is dark and does nothing.</p>
		{/if}
		{#if $wingState.digital !== null}
			<!-- The rear panel's contacts, live, so where they rest can be read before
			     any of them is bound to anything. -->
			<div class="digital">
				<span class="dim">Rear contacts</span>
				{#each Array.from({ length: DIGITAL_INPUTS }, (_, i) => i) as i (i)}
					<span class="contact" class:closed={(($wingState.digital ?? 0) >> i) & 1} title={`Digital in ${i + 1}`}>{i + 1}</span>
				{/each}
			</div>
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
					<!-- Pressing the knob, or clicking its card, opens a calculator on it: MA's. -->
					<div
						class="encoder"
						class:empty={!slot}
						class:typing={$calculator?.slot === i}
						role="button"
						tabindex={slot ? 0 : -1}
						onclick={() => slot && $calculator?.slot !== i && openCalculator(i)}
						onkeydown={(e) => slot && e.key === 'Enter' && $calculator?.slot !== i && openCalculator(i)}
					>
						{#if slot}
							<span class="name">{slot.label}</span>
							{#if $calculator?.slot === i}
								<!-- svelte-ignore a11y_autofocus -->
								<input
									class="calc mono"
									inputmode="decimal"
									autofocus
									value={$calculator.text}
									oninput={(e) => calculator.set({ slot: i, text: e.currentTarget.value })}
									onkeydown={(e) => {
										e.stopPropagation();
										if (e.key === 'Enter') setTyped(e.currentTarget.value);
										if (e.key === 'Escape') calculator.set(null);
									}}
								/>
							{:else}
								<span class="value mono">{slotReading(slotValue(slot), slot.component)}</span>
							{/if}
						{:else}
							<span class="name dim">—</span>
						{/if}
					</div>
				{/each}
			</div>
			{#if $calculator}
				<p class="dim">
					Typing {$encoderBar.slots[$calculator.slot]?.label ?? 'a value'}: the keypad types it, PLEASE sets
					it on the selection, ESC or pressing the knob again closes it.
				</p>
			{/if}
			{#if $selection.length === 0}
				<p class="dim">Select something and the encoders turn its attributes: inner knob coarse, outer ring fine.</p>
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
				<table class="bindings">
					<thead>
						<tr><th>Control</th><th>Does</th><th>Now</th><th></th></tr>
					</thead>
					<tbody>
						{#each [...$wingBindings.values()] as b (b.id)}
							<tr>
								<td class="control">{controlName(b.control)}</td>
								<td>{b.label || describe(b.action)}</td>
								<td class="mono dim">{b.control.kind === 'Fader' ? faderReading(b.action) : ''}</td>
								<td class="acts">
									{#if $canEdit}
										<button class="btn btn-ghost btn-small" onclick={() => learned.set(b.control)}>Edit</button>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
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
	.status {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.status p {
		margin: 0;
		font-size: 0.8rem;
	}
	.head {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px 10px;
	}
	.device {
		font-weight: 500;
	}
	.spacer {
		flex: 1;
	}
	.badge {
		font-size: 0.7rem;
		font-weight: 600;
		padding: 2px 8px;
		border-radius: 10px;
		border: 1px solid;
		white-space: nowrap;
	}
	.badge.good { color: #4ade80; border-color: #14532d; background: #14532d44; }
	.badge.busy { color: #fbbf24; border-color: #78350f; background: #78350f44; }
	.badge.idle { color: #9ca3af; border-color: #374151; background: #37415144; }
	.badge.bad { color: #f87171; border-color: #7f1d1d; background: #7f1d1d44; }
	.fault {
		color: #f87171;
	}
	.digital {
		display: flex;
		gap: 4px;
		align-items: center;
	}
	.digital .dim {
		margin-right: 4px;
		font-size: 0.75rem;
	}
	.contact {
		width: 18px;
		height: 18px;
		border-radius: 50%;
		border: 1px solid var(--border, #333);
		font-size: 0.65rem;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		color: var(--text-dim, #888);
	}
	.contact.closed {
		background: var(--accent, #3b82f6);
		color: white;
	}
	.at,
	.drive {
		display: inline-flex;
		gap: 8px;
		align-items: center;
		font-size: 0.8rem;
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
	.encoder.typing {
		border-color: var(--accent, #3b82f6);
	}
	.calc {
		width: 100%;
		font-size: 1rem;
		background: transparent;
		border: none;
		border-bottom: 1px solid var(--accent, #3b82f6);
		color: inherit;
		padding: 0;
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
		width: 100%;
		border-collapse: collapse;
		font-size: 0.8rem;
	}
	.bindings th {
		text-align: left;
		font-weight: 500;
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint, #666);
		padding: 0 10px 6px 0;
	}
	.bindings td {
		padding: 5px 10px 5px 0;
		border-top: 1px solid var(--line, #262626);
	}
	.bindings .control {
		color: var(--text-dim, #888);
		white-space: nowrap;
	}
	.bindings .acts {
		text-align: right;
	}
	.btn-small {
		min-height: 24px;
		padding: 0 10px;
		font-size: 12px;
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
