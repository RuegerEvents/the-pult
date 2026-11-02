<script lang="ts">
	/**
	 * The transport, above the lanes: play or stop, back to the top, where the playhead
	 * is, record, listening and snapping. How it ends is the timeline's Inspector and the
	 * end handle on the lanes; it took a wide button here for a setting.
	 *
	 * The readout is arithmetic over the SYNCED anchor once an animation frame, the way
	 * the station works it out, and **shows a gap until this browser knows the station's
	 * clock** — a playhead out by however wrong this machine's clock is would be the
	 * plausible-wrong-number `ws/clock.ts` exists to remove.
	 *
	 * Every control is a button of one kind, and a toggle says it is on by its color.
	 * Record is one press: it arms from what this browser last chose in the Inspector —
	 * the programmer unless somebody picked an input — and gives the faders back when the
	 * take stops. The count-in and a grab from the wire are in the Inspector too.
	 */

	import Circle from '@lucide/svelte/icons/circle';
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import Ear from '@lucide/svelte/icons/ear';
	import Magnet from '@lucide/svelte/icons/magnet';
	import Play from '@lucide/svelte/icons/play';
	import SkipBack from '@lucide/svelte/icons/skip-back';
	import Square from '@lucide/svelte/icons/square';

	import type { Timeline } from '$lib/generated/index.js';
	import { collection, showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { countdownOf, lapOf, locate, now, playCounted, positionOf, recordFrom, stop, takes } from '$lib/stores/timelines.js';
	import { armArgs, PROGRAMMER, takeSummary } from '$lib/takes.js';
	import { pageId } from '$lib/stores/takeReview.js';
	import { monitorSettings, monitorStates, setMonitorLevel, toggleListening } from '$lib/stores/monitor.js';
	import { dropdown } from '$lib/actions.js';
	import { clock, parsePosition } from '$lib/waveform.js';

	let { timeline, snapping = $bindable() }: { timeline: Timeline; snapping: boolean } = $props();

	const inputs = collection('inputs');
	const position = $derived(positionOf(timeline, $now));
	const shown = $derived(position === null ? null : lapOf(timeline, position));
	let typing = $state(false);
	const take = $derived($takes?.[timeline.id] ?? null);
	const counting = $derived(countdownOf(timeline, $now));

	/** The input chosen in the Inspector, while it is still switched on; the programmer otherwise. */
	const source = $derived(
		$recordFrom !== PROGRAMMER && $inputs.some((i) => i.id === $recordFrom && i.enabled) ? $recordFrom : PROGRAMMER
	);
	const sourceName = $derived(source === PROGRAMMER ? 'the programmer' : ($inputs.find((i) => i.id === source)?.name ?? 'an input'));
	function toggleRecord() {
		// Says which page armed it, so this is the page the review opens on.
		const args = timeline.recording ? armArgs('', true) : armArgs(source, true);
		void showData().timelines.byId(timeline.id).record({ ...args, armedBy: pageId });
	}
	/** Whether this tab's speaker is on this timeline: see `stores/monitor.ts`. */
	const hearing = $derived($monitorStates[timeline.id] ?? 'off');
	let levelMenu = $state(false);
	/** Held open while the slider is dragged, which can take the mouse out of the menu. */
	let sliding = $state(false);
</script>

<div class="transport">
	<div class="group">
		{#if counting !== null}
			<button class="btn btn-primary counting" onclick={() => stop(timeline.id)} title="Counting in — Stop cancels it (Space)"
				><Square size={14} />Cancel</button
			>
		{:else if timeline.running}
			<button class="btn btn-primary" onclick={() => stop(timeline.id)} title="Stop (Space)"><Square size={14} />Stop</button>
		{:else}
			<button class="btn btn-primary" onclick={() => playCounted(timeline)} title="Play (Space)"><Play size={14} />Play</button>
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
		<button
			class="btn rec"
			class:armed={!!timeline.recording}
			class:rolling={!!take}
			aria-pressed={!!timeline.recording}
			disabled={!$canEdit}
			title={take
				? takeSummary(take)
				: timeline.recording
					? 'Armed: the next Play records until Stop. Press to disarm'
					: `Arm a recording from ${sourceName}`}
			onclick={toggleRecord}
		>
			<Circle size={12} />{take ? 'Recording' : timeline.recording ? 'Armed' : 'Record'}
		</button>
	</div>

	<div class="group">
		<!-- This tab's speaker, for listening while programming: never a show output.
		     A tap is what lets a browser start sound, so a reloaded page is "armed". -->
		<span class="split">
		<button
			class="btn btn-icon"
			class:on={hearing === 'listening' || hearing === 'loading'}
			class:armed={hearing === 'armed'}
			aria-pressed={hearing === 'listening' || hearing === 'loading'}
			title={hearing === 'armed'
				? 'Tap to carry on listening on this tab'
				: hearing === 'no-clock'
					? 'Waiting for the station clock before anything can be heard in step'
					: hearing === 'loading'
						? 'The station is still loading some of this timeline’s sound; it joins in when it is ready'
						: 'Hear this timeline on this tab, in step with the playhead. Only this speaker: the show is never played from a browser'}
			onclick={() => toggleListening(timeline.id)}
		>
			<Ear size={14} />{hearing === 'loading' ? 'Loading…' : 'Listen'}
		</button>
		<button
			class="btn btn-icon caret"
			class:on={hearing === 'listening' || hearing === 'loading'}
			aria-haspopup="true"
			aria-expanded={levelMenu}
			aria-label="How loud this tab plays it"
			title="How loud this tab plays it"
			onclick={() => (levelMenu = !levelMenu)}><ChevronDown size={12} /></button
		>
		{#if levelMenu}
			<div class="menu" use:dropdown={{ close: () => (levelMenu = false), keep: () => sliding }}>
				<span class="heading">Volume on this tab</span>
				<input
					class="level"
					type="range"
					min="0"
					max="1"
					step="0.01"
					aria-label="How loud this tab plays it"
					value={$monitorSettings.level}
					onpointerdown={() => (sliding = true)}
					onpointerup={() => (sliding = false)}
					onpointercancel={() => (sliding = false)}
					oninput={(e) => setMonitorLevel(Number(e.currentTarget.value))}
				/>
				<span class="figure">{Math.round($monitorSettings.level * 100)}%</span>
			</div>
		{/if}
		</span>
		<button class="btn btn-icon" class:on={snapping} aria-pressed={snapping} title="Drags snap to the grid and the markers" onclick={() => (snapping = !snapping)}>
			<Magnet size={14} />Snap
		</button>
	</div>
</div>

<style>
	/* Listen and its caret read as one button with two halves. */
	.split {
		position: relative;
		display: inline-flex;
	}
	.split > .btn:first-child {
		border-top-right-radius: 0;
		border-bottom-right-radius: 0;
	}
	.split > .caret {
		min-width: 0;
		padding: 0 6px;
		margin-left: -1px;
		border-top-left-radius: 0;
		border-bottom-left-radius: 0;
	}
	.menu {
		position: absolute;
		top: calc(100% + 4px);
		right: 0;
		z-index: 20;
		display: grid;
		grid-template-columns: 1fr auto;
		align-items: center;
		gap: 6px 10px;
		width: 220px;
		padding: 10px 12px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.45);
	}
	.menu .heading {
		grid-column: 1 / -1;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}
	.level {
		width: 100%;
	}
	.figure {
		min-width: 3em;
		text-align: right;
		font-size: var(--font-xs);
		font-variant-numeric: tabular-nums;
		color: var(--text-dim);
	}
	.countdown {
		min-width: 5.5em;
		font-size: 18px;
		font-weight: 700;
		color: #f87171;
		font-variant-numeric: tabular-nums;
		text-align: center;
	}
	/* Its groups are the bar's own items, so they wrap with the panel's buttons as one
	   row of groups rather than as two blocks side by side. */
	.transport {
		display: contents;
	}
	/* A group never breaks inside itself; the transport wraps between groups. */
	.group {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: nowrap;
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
	.btn.on {
		color: var(--accent);
		border-color: var(--accent-solid);
	}
	.btn.armed {
		color: var(--text-dim);
		border-style: dashed;
	}
	.rec :global(svg) {
		color: var(--bad);
	}
	.rec.armed {
		color: var(--bad);
		border-color: var(--bad);
		border-style: solid;
	}
	.rec.armed :global(svg) {
		fill: var(--bad);
	}
	.rec.rolling {
		background: color-mix(in srgb, var(--bad) 20%, var(--bg-raised));
		animation: pulse 1.2s ease-in-out infinite;
	}
	@keyframes pulse {
		50% {
			background: color-mix(in srgb, var(--bad) 8%, var(--bg-raised));
		}
	}
</style>
