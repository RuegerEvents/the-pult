<script lang="ts">
	/**
	 * The transport, above the lanes: play or stop, back to the top, where the playhead
	 * is, how it ends, what it records and whether drags snap.
	 *
	 * The readout is arithmetic over the SYNCED anchor once an animation frame, the way
	 * the station works it out, and **shows a gap until this browser knows the station's
	 * clock** — a playhead out by however wrong this machine's clock is would be the
	 * plausible-wrong-number `ws/clock.ts` exists to remove.
	 *
	 * Three groups that wrap as groups — playing, how it ends and records, snapping — so
	 * a narrow panel breaks between them and never between a button and its label.
	 * Recording is a menu on its own chip: what it records from is set where it is
	 * shown, and while a take is running it says what the take holds. A grab is in it
	 * too — the same decoding of the wire, taken once rather than continuously.
	 */

	import Circle from '@lucide/svelte/icons/circle';
	import Magnet from '@lucide/svelte/icons/magnet';
	import Play from '@lucide/svelte/icons/play';
	import Repeat from '@lucide/svelte/icons/repeat';
	import SkipBack from '@lucide/svelte/icons/skip-back';
	import Square from '@lucide/svelte/icons/square';

	import type { Timeline } from '$lib/generated/index.js';
	import { selectSubject } from '$lib/objects.js';
	import { selection } from '$lib/stores/selection.js';
	import { collection, showClient, showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { countingIn, countInSeconds, lapOf, locate, now, playCounted, positionOf, stop, takes } from '$lib/stores/timelines.js';
	import { armArgs, armChoice, PROGRAMMER, releases, takeSummary } from '$lib/takes.js';
	import { addToast } from '$lib/toasts.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { timeline, snapping = $bindable() }: { timeline: Timeline; snapping: boolean } = $props();

	const inputs = collection('inputs');
	const position = $derived(positionOf(timeline, $now));
	const shown = $derived(position === null ? null : lapOf(timeline, position));
	let typing = $state(false);
	const take = $derived($takes?.[timeline.id] ?? null);
	const counting = $derived($countingIn?.id === timeline.id ? $countingIn.left : null);
	const COUNT_INS = [0, 2, 3, 5];
	let recordMenu = $state(false);

	const source = $derived(armChoice(timeline.recording));
	const sourceName = $derived(
		source === ''
			? 'off'
			: source === PROGRAMMER
				? 'programmer'
				: ($inputs.find((i) => i.id === source)?.name ?? 'an input')
	);
	const arm = (choice: string, release: boolean) =>
		void showData().timelines.byId(timeline.id).record(armArgs(choice, release));
	const listening = $derived($inputs.some((i) => i.enabled));

	/** What is on the wire now, into the programmer, for the selected fixtures. */
	async function grab() {
		recordMenu = false;
		try {
			const answer = (await showClient().call('input.grab', { fixtureIds: $selection })) as { written: number };
			if (answer.written === 0) addToast('Nothing on the wire reaches those fixtures.', 'warning');
			else addToast(`${answer.written} value${answer.written === 1 ? '' : 's'} grabbed into the programmer.`, 'success');
		} catch (e) {
			addToast(String(e));
		}
	}
</script>

<div class="transport">
	<div class="group">
		{#if timeline.running}
			<button class="btn btn-primary" onclick={() => stop(timeline.id)} title="Stop (Space)"><Square size={14} />Stop</button>
		{:else}
			<button
				class="btn btn-primary"
				class:counting={counting !== null}
				onclick={() => playCounted(timeline)}
				title={counting !== null ? 'Counting in — press again to cancel' : 'Play (Space)'}
				><Play size={14} />{counting !== null ? 'Cancel' : 'Play'}</button
			>
		{/if}
		<button class="btn btn-icon" title="Back to the top" aria-label="Back to the top" onclick={() => locate(timeline.id, 0)}><SkipBack size={14} /></button>

		{#if counting !== null}
			<span class="countdown" title="Recording starts when this reaches zero">● {counting}</span>
		{:else if typing}
			<!-- svelte-ignore a11y_autofocus -->
			<input
				class="input readout-input"
				placeholder="1:04.20"
				autofocus
				onkeydown={(e) => {
					e.stopPropagation();
					if (e.key === 'Escape') typing = false;
					if (e.key === 'Enter') {
						const ms = parsePosition(e.currentTarget.value);
						if (ms !== null) void locate(timeline.id, ms);
						typing = false;
					}
				}}
				onblur={() => (typing = false)}
			/>
		{:else}
			<button class="readout" class:rolling={timeline.running} title="Type a position to go there" onclick={() => (typing = true)}>
				{#if shown === null}
					<span class="gap" title="This browser has not worked out the station's clock yet">—:—.—</span>
				{:else}
					{clock(shown.position)}{#if shown.lap > 0}<small> lap {shown.lap + 1}</small>{/if}
				{/if}
			</button>
		{/if}
	</div>

	<div class="group">
		<button class="chip" title="How it ends — in the Inspector" onclick={() => selectSubject({ kind: 'timeline', id: timeline.id })}>
			{#if timeline.end?.then === 'Loop'}
				<Repeat size={12} />goes round at {clock(timeline.end.at_ms)}
			{:else if timeline.end}
				stops at {clock(timeline.end.at_ms)}
			{:else}
				runs until stopped
			{/if}
		</button>

		<span class="menu-anchor">
			<button
				class="chip"
				class:armed={!!timeline.recording}
				class:rolling={!!take}
				aria-haspopup="menu"
				aria-expanded={recordMenu}
				onclick={() => (recordMenu = !recordMenu)}
			>
				<Circle size={10} />{take ? 'Recording' : `Recording: ${sourceName}`}<span class="caret">▾</span>
			</button>
			{#if recordMenu}
				<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
				<div class="catch" onclick={() => (recordMenu = false)}></div>
				<div class="menu" role="menu">
					<span class="heading">Records from</span>
					<button role="menuitemradio" aria-checked={source === ''} class:on={source === ''} disabled={!$canEdit} onclick={() => arm('', false)}>Nothing</button>
					<button
						role="menuitemradio"
						aria-checked={source === PROGRAMMER}
						class:on={source === PROGRAMMER}
						disabled={!$canEdit}
						onclick={() => arm(PROGRAMMER, releases(timeline.recording))}>The programmer, and sequence Gos</button
					>
					{#each $inputs.filter((i) => i.enabled) as input (input.id)}
						<button role="menuitemradio" aria-checked={source === input.id} class:on={source === input.id} disabled={!$canEdit} onclick={() => arm(input.id, false)}>
							{input.name}
						</button>
					{/each}
					{#if source}
						<span class="heading rule">Count in</span>
						<div class="count-ins" role="group" aria-label="Count in before recording">
							{#each COUNT_INS as n (n)}
								<button
									class:on={$countInSeconds === n}
									aria-pressed={$countInSeconds === n}
									onclick={() => countInSeconds.set(n)}>{n === 0 ? 'off' : `${n} s`}</button
								>
							{/each}
						</div>
					{/if}
					{#if source === PROGRAMMER}
						<label class="check" title="When the take is stored, give the keys it recorded back. Off, the faders stay held and beat the take until you clear them.">
							<input
								type="checkbox"
								disabled={!$canEdit}
								checked={releases(timeline.recording)}
								onchange={(e) => arm(PROGRAMMER, e.currentTarget.checked)}
							/>
							Give the faders back after
						</label>
					{/if}
					<span class="heading rule">Once</span>
					<button
						role="menuitem"
						disabled={!listening || $selection.length === 0}
						title={!listening ? 'No input is switched on' : $selection.length === 0 ? 'Nothing is selected' : ''}
						onclick={grab}>Grab the selection from the wire</button
					>
					<p class="note">
						{#if take}
							{takeSummary(take)}
						{:else if source === PROGRAMMER}
							From the next Play to the next Stop: every programmer key that moves, from your first
							touch, every sequence level you move as a curve, and every Go, Back and Off as an event.
							On a loop each lap you touch is a pass.
						{:else if source}
							The station holding that input records from the next Play to the next Stop.
						{:else}
							Nothing is recorded.
						{/if}
					</p>
				</div>
			{/if}
		</span>
	</div>

	<div class="group end">
		<button class="chip" class:on={snapping} aria-pressed={snapping} title="Drags snap to the grid and the markers" onclick={() => (snapping = !snapping)}>
			<Magnet size={12} />snap
		</button>
	</div>
</div>

<style>
	.countdown {
		min-width: 5.5em;
		font-size: 18px;
		font-weight: 700;
		color: #f87171;
		font-variant-numeric: tabular-nums;
		text-align: center;
	}
	.count-ins { display: flex; gap: 4px; padding: 2px 8px 6px; }
	.count-ins button {
		background: none; border: 1px solid var(--line-strong, #3a3a3a); border-radius: 3px;
		color: inherit; font: inherit; font-size: 12px; padding: 2px 8px; cursor: pointer;
	}
	.count-ins button.on { border-color: #ef4444; color: #fecaca; }
	.transport {
		display: flex;
		align-items: center;
		gap: 6px 14px;
		flex-wrap: wrap;
		flex: 1;
		min-width: 0;
	}
	/* A group never breaks inside itself; the transport wraps between groups. */
	.group {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: nowrap;
	}
	.group.end {
		margin-left: auto;
	}
	.readout {
		min-width: 104px;
		background: none;
		border: none;
		color: var(--text);
		font: inherit;
		font-size: 20px;
		font-variant-numeric: tabular-nums;
		text-align: left;
		cursor: text;
		padding: 0 6px;
	}
	.readout.rolling {
		color: var(--good);
	}
	.readout small {
		font-size: var(--font-xs);
		color: var(--text-dim);
		margin-left: 4px;
	}
	.readout-input {
		width: 104px;
	}
	.gap {
		color: var(--text-faint);
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		min-height: 28px;
		padding: 0 9px;
		border: 1px solid var(--line-strong);
		border-radius: 14px;
		background: var(--bg-raised);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
		white-space: nowrap;
	}
	.chip:hover {
		color: var(--text);
		border-color: var(--line-input);
	}
	.chip.on {
		color: var(--accent);
		border-color: var(--accent-solid);
	}
	.chip.armed {
		color: var(--bad);
		border-color: var(--bad);
	}
	.chip.rolling {
		background: color-mix(in srgb, var(--bad) 20%, var(--bg-raised));
		animation: pulse 1.2s ease-in-out infinite;
	}
	@keyframes pulse {
		50% {
			background: color-mix(in srgb, var(--bad) 8%, var(--bg-raised));
		}
	}
	.caret {
		font-size: 9px;
		opacity: 0.7;
	}
	.menu-anchor {
		position: relative;
	}
	.catch {
		position: fixed;
		inset: 0;
		z-index: 19;
	}
	.menu {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		z-index: 20;
		width: 280px;
		padding: 6px;
		display: flex;
		flex-direction: column;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.45);
	}
	.menu .heading {
		padding: 4px 8px;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}
	.menu button {
		text-align: left;
		padding: 7px 8px;
		border: none;
		border-radius: 4px;
		background: none;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.menu button:hover:not(:disabled) {
		background: var(--bg-hover);
	}
	.menu .heading.rule {
		margin-top: 4px;
		padding-top: 8px;
		border-top: 1px solid var(--line);
	}
	.menu button:disabled {
		color: var(--text-faint);
		cursor: default;
	}
	.menu button.on {
		color: var(--accent);
	}
	.menu button.on::before {
		content: '● ';
	}
	.check {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 8px;
		font-size: var(--font-sm);
		color: var(--text-dim);
		border-top: 1px solid var(--line);
		margin-top: 4px;
	}
	.note {
		margin: 4px 8px 4px;
		font-size: var(--font-xs);
		line-height: 1.45;
		color: var(--text-faint);
	}
</style>
