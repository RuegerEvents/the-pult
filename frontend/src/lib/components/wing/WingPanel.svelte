<script lang="ts">
	/**
	 * The command wing: whether one is here, whether this page drives it, what its
	 * encoders are on, and what its controls are bound to.
	 *
	 * A panel rather than a Setup errand because the encoder bar is what an operator
	 * reads while turning — the wing has no screen of its own — and a binding is made
	 * by touching the control, which is something done in the middle of programming.
	 */

	import type { BindingAction } from '$lib/generated/index.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { openConsoleLine } from '$lib/stores/plugins.js';
	import {
		calculator,
		controlName,
		driving,
		drivesHere,
		encoderBar,
		learned,
		learning,
		openCalculator,
		setLearning,
		setTyped,
		setDrivesHere,
		setWingAt,
		slotValue,
		typeCalculator,
		wingBindings,
		wingEditAsk,
		wingGoSequence,
		wingLine,
		wingMap,
		wingPage,
		wingAt,
		wingStations,
		wingState
	} from '$lib/stores/wing.js';
	import { output } from '$lib/stores/output.js';
	import { pressableTargets, type SurfaceShow } from '$lib/surface.js';
	import { DIGITAL_INPUTS, slotReading } from '$lib/wing.js';
	import ToggleControl from '$lib/components/programmer/controls/ToggleControl.svelte';
	import WingPages from './WingPages.svelte';
	import BindingEditor from './BindingEditor.svelte';

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
			case 'FlashSequence':
				return `flashes ${$sequences.find((s) => s.id === action.sequence_id)?.name ?? 'a sequence no longer in the show'}`;
			case 'GoToCue': {
				const seq = $sequences.find((s) => s.id === action.sequence_id);
				const cue = $cues.find((c) => c.id === action.cue_id);
				return seq && cue ? `goes to cue ${cue.number.toFixed(1)} · ${cue.name} of ${seq.name}` : 'a cue no longer in the show';
			}
		}
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
	const goName = $derived(
		$wingGoSequence ? ($sequences.find((s) => s.id === $wingGoSequence)?.name ?? null) : null
	);
</script>

<div class="wing">
	<!-- One line that says what the wing is doing and what this page is doing about
	     it; the sentences only where there is something to explain. -->
	<section class="status">
		<div class="head">
			<span class="badge {badge.tone}">{badge.text}</span>
			<span class="device">{phase.kind === 'Running' ? ($wingState.device ?? 'Command wing') : 'Command wing'}</span>
			{#if elsewhere}<span class="dim">on {elsewhere}</span>{/if}
			{#if phase.kind === 'Running'}<span class="page" title="The page of bindings the controls act on">Page {$wingPage}</span>{/if}
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
			<p class="dim">The station runs it with no page showing it. Switch Drive on to share this page's selection with it and see what it types.</p>
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
									oninput={(e) => typeCalculator(i, e.currentTarget.value)}
									onkeydown={(e) => {
										e.stopPropagation();
										if (e.key === 'Enter') setTyped(i, e.currentTarget.value);
										if (e.key === 'Escape') openCalculator(null);
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
				<p class="dim">GO goes on {goName ?? 'nothing yet: click a cue sheet, or take a cue'}.</p>
			</section>
		{/if}

		<section>
			<div class="section-head">
				<!-- Every control acts on one page of bindings at a time, and what is learned
				     goes on the page the wing is on. -->
				<h3>Bindings <span class="page">Page {$wingPage}</span> <span class="dim">PAGE+ and PAGE− to turn it</span></h3>
				{#if $canEdit}
					<button
						class="btn {$learning ? 'btn-primary learning' : 'btn-ghost'}"
						onclick={() => setLearning(!$learning)}
					>
						{$learning ? 'Touch a control on the wing…' : 'Learn'}
					</button>
				{/if}
			</div>

			{#if $wingMap}
				<WingPages map={$wingMap} {describe} />
			{/if}

			<!-- EDIT on the wing opens the same editor in its own dialog. -->
			{#if $learned && !$wingEditAsk}
				<BindingEditor control={$learned} onclose={() => learned.set(null)} />
			{/if}

			{#if $wingBindings.size === 0}
				<p class="dim">
					Nothing bound on page {$wingPage}. Press LEARN on the wing (or Learn here), touch a key, fader or encoder, then
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
	.page {
		font-size: var(--font-sm, 12px);
		font-weight: 600;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--bg-raised, #2a2a2a);
		color: var(--text-bright, #fff);
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
