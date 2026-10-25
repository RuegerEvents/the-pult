<script lang="ts">
	/**
	 * A universe, as 512 bytes.
	 *
	 * The values are what the connector last actually put on the wire, read off its
	 * dedup cache — so a universe the dedup skipped shows what a receiver is still
	 * holding, which is the truth about that wire and not a re-render of the show.
	 *
	 * One universe at a time, deliberately. The sheet shows one, and asking for forty
	 * of them at panel rate would be a megabyte a second for a picture nobody can
	 * read; the chips above carry what the others are doing in one figure each, which
	 * is what tells an operator where to look next.
	 *
	 * **On an output's own wire it is also a place to put a hand.** *Test* turns a click
	 * into a selection, and the level controls force the selection onto the wire — the
	 * sheet then shows the byte that actually went out, so what is drawn is the proof
	 * and not a promise. It works on a universe nothing is patched to, which is the
	 * first thing anybody checks a cable with, so the sheet can be drawn for a universe
	 * the connector has not carried yet: all zeros, until the first forced frame lands.
	 * Everything about *what a test is* lives in `dmxtest.ts`; this file only draws it.
	 */

	import { untrack } from 'svelte';

	import type { UniverseTraffic } from '$lib/generated/index.js';
	import {
		CHANNELS,
		describe,
		force,
		forcedIn,
		isSolo,
		percent,
		range,
		release,
		releaseUniverse,
		setSolo,
		shifted
	} from '$lib/dmxtest.js';
	import { channelWeight, universeFocus } from '$lib/wire.js';
	import type { Tester } from './views.js';

	type Props = {
		of: UniverseTraffic;
		focus: string | null;
		ask: (focus: string | null) => void;
		tester?: Tester;
	};
	const { of, focus, ask, tester }: Props = $props();

	const COLUMNS = 16;

	let testing = $state(false);
	/** The channels picked on the sheet, ascending. */
	let selected = $state<number[]>([]);
	/** Where a shift-click range starts from. */
	let anchor = $state<number | null>(null);
	/** What the controls will force the selection to. Full, because that is the question. */
	let level = $state(255);
	let universeInput = $state('');

	/** The universe the viewer was asked for, whether or not the connector has it yet. */
	const asked = $derived.by(() => {
		const n = Number(focus);
		return Number.isInteger(n) && n >= 1 ? n : null;
	});
	/**
	 * Which universe the sheet is of. Testing, the one that was *asked for*: the
	 * connector answers a universe it has never carried with some other universe it
	 * has, which is right for reading and wrong for a hand about to put a level on
	 * channel 3 — it would land on a line the operator is not looking at.
	 */
	const showing = $derived(testing && asked !== null ? asked : (of.focused?.universe ?? null));
	const forced = $derived(
		tester && showing !== null ? forcedIn(tester.test, showing) : ({} as Record<number, number>)
	);
	const solo = $derived(tester && showing !== null ? isSolo(tester.test, showing) : false);

	const rows = $derived.by(() => {
		const channels =
			of.focused && of.focused.universe === showing
				? of.focused.channels
				: Array.from({ length: CHANNELS }, () => 0);
		const out: { from: number; values: number[] }[] = [];
		for (let at = 0; at < channels.length; at += COLUMNS) {
			out.push({ from: at + 1, values: channels.slice(at, at + COLUMNS) });
		}
		return out;
	});

	function pick(channel: number, event: MouseEvent) {
		if (!testing) return;
		if (event.shiftKey && anchor !== null) {
			selected = range(anchor, channel);
		} else if (event.metaKey || event.ctrlKey) {
			selected = selected.includes(channel)
				? selected.filter((each) => each !== channel)
				: [...selected, channel].sort((a, b) => a - b);
			anchor = channel;
		} else {
			selected = selected.length === 1 && selected[0] === channel ? [] : [channel];
			anchor = channel;
		}
		// Looking at a forced channel shows what it is forced to, so a slider is never
		// a lie about the thing under it.
		const first = selected.find((each) => forced[each] !== undefined);
		if (first !== undefined) level = forced[first];
	}

	function apply(next: number) {
		level = Math.max(0, Math.min(255, Math.round(next)));
		if (!tester || showing === null || selected.length === 0) return;
		tester.set(force(tester.test, showing, selected, level));
	}

	function letGo() {
		if (!tester || showing === null) return;
		tester.set(release(tester.test, showing, selected));
	}

	/** Move the selection along and take the forcing with it: the channel-check walk. */
	function walk(by: number) {
		if (!tester || showing === null || selected.length === 0) return;
		const next = shifted(selected, by);
		if (!next) return;
		tester.set(force(release(tester.test, showing, selected), showing, next, level));
		selected = next;
		anchor = next[0];
	}

	function goToUniverse() {
		const n = Number(universeInput);
		if (!Number.isInteger(n) || n < 1 || n > 63999) return;
		selected = [];
		anchor = null;
		ask(universeFocus(n));
		universeInput = '';
	}

	/** Leaving test mode lets go of everything: a mode you cannot see is not one to leave on. */
	function stopTesting() {
		testing = false;
		selected = [];
		anchor = null;
		tester?.set({ universes: [] });
	}

	// A different universe is a different set of channels: carrying a selection across
	// would put a level on a channel the operator is no longer looking at.
	let lastShown: number | null = null;
	$effect(() => {
		const now = showing;
		if (now !== lastShown) {
			lastShown = now;
			untrack(() => {
				selected = [];
				anchor = null;
			});
		}
	});

	/** "moving", or how long ago it stopped. */
	const activity = (ms: number) => {
		if (ms < 500) return 'moving';
		if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
		return `${Math.round(ms / 60_000)}m`;
	};
</script>

{#if tester}
	<div class="testbar">
		<button
			class="mode"
			class:on={testing}
			onclick={() => (testing ? stopTesting() : (testing = true))}
			title="Click channels on the sheet and force them to a level"
		>
			{testing ? 'Testing' : 'Test'}
		</button>
		{#if testing}
			<form
				class="goto"
				onsubmit={(e) => {
					e.preventDefault();
					goToUniverse();
				}}
			>
				<label>
					Universe
					<input
						type="number"
						min="1"
						max="63999"
						placeholder={showing !== null ? String(showing) : ''}
						bind:value={universeInput}
					/>
				</label>
				<button type="submit">Go</button>
			</form>
		{/if}
	</div>
{/if}

{#if of.universes.length === 0 && !testing}
	<p class="empty">This output has not carried a universe yet.</p>
{:else}
	<div class="chips">
		{#each of.universes as universe (universe.universe)}
			<button
				class="chip"
				class:on={universe.universe === showing}
				class:live={universe.changed_ms_ago < 500}
				onclick={() => ask(universeFocus(universe.universe))}
				title="{universe.live_channels} channels above zero · last change {activity(
					universe.changed_ms_ago
				)} ago · last sent {activity(universe.sent_ms_ago)} ago"
			>
				<span class="number">{universe.universe}</span>
				<span class="count">{universe.live_channels}</span>
			</button>
		{/each}
	</div>

	{#if testing && tester && showing !== null}
		<div class="controls">
			<span class="selection">
				{#if selected.length === 0}
					Click a channel · shift for a range · ⌘ to add
				{:else}
					Universe {showing}, channel {describe(selected)}
				{/if}
			</span>
			<div class="level">
				<input
					type="range"
					min="0"
					max="255"
					value={level}
					disabled={selected.length === 0}
					oninput={(e) => apply(Number(e.currentTarget.value))}
					aria-label="Level"
				/>
				<input
					type="number"
					min="0"
					max="255"
					value={level}
					disabled={selected.length === 0}
					oninput={(e) => apply(Number(e.currentTarget.value))}
					aria-label="Level, 0 to 255"
				/>
				<span class="pct">{percent(level)}</span>
			</div>
			<div class="buttons">
				<button disabled={selected.length === 0} onclick={() => apply(255)}>Full</button>
				<button disabled={selected.length === 0} onclick={() => apply(128)}>50%</button>
				<button disabled={selected.length === 0} onclick={() => apply(0)}>Dark</button>
				<button disabled={selected.length === 0} onclick={letGo} title="Back to the show">
					Release
				</button>
				<span class="gap"></span>
				<button
					disabled={selected.length === 0 || shifted(selected, -1) === null}
					onclick={() => walk(-1)}
					title="Move the forcing to the channel below"
				>
					◀
				</button>
				<button
					disabled={selected.length === 0 || shifted(selected, 1) === null}
					onclick={() => walk(1)}
					title="Move the forcing to the channel above"
				>
					▶
				</button>
			</div>
			<label class="solo" title="Every channel of this universe not forced goes to zero">
				<input
					type="checkbox"
					checked={solo}
					onchange={(e) => tester && tester.set(setSolo(tester.test, showing, e.currentTarget.checked))}
				/>
				Solo this universe
			</label>
			<button
				class="all"
				disabled={Object.keys(forced).length === 0 && !solo}
				onclick={() => tester && tester.set(releaseUniverse(tester.test, showing))}
			>
				Release universe {showing}
			</button>
		</div>
	{/if}

	{#if testing && tester && showing === null}
		<p class="note">Enter the universe to test above.</p>
	{/if}

	{#if showing !== null}
		<!-- The one place a universe number is spelled as a focus string is
		     `universeFocus`; nothing here invents its own spelling. -->
		{#if !testing && focus && focus !== universeFocus(showing)}
			<p class="note">
				Showing universe {showing}; this output does not carry universe {focus}.
			</p>
		{/if}
		{#if testing && of.focused?.universe !== showing}
			<p class="note">
				Nothing has gone out on universe {showing} yet; the first forced channel puts it on the
				wire.
			</p>
		{/if}
		<div class="sheet" role="table" aria-label="Universe {showing}">
			<div class="head" role="row">
				<span class="addr"></span>
				{#each Array(COLUMNS) as _, column (column)}
					<span class="col" role="columnheader">{column + 1}</span>
				{/each}
			</div>
			{#each rows as row (row.from)}
				<div class="row" role="row">
					<span class="addr">{row.from}</span>
					{#each row.values as value, column (column)}
						{@const channel = row.from + column}
						{#if testing}
							<button
								class="cell testable"
								class:zero={value === 0}
								class:forced={forced[channel] !== undefined}
								class:picked={selected.includes(channel)}
								style="--weight: {channelWeight(value)}"
								onclick={(e) => pick(channel, e)}
								title={forced[channel] !== undefined
									? `Channel ${channel}: forced to ${forced[channel]}`
									: `Channel ${channel}`}>{value}</button
							>
						{:else}
							<span
								class="cell"
								class:zero={value === 0}
								role="cell"
								style="--weight: {channelWeight(value)}"
								title="Channel {channel}">{value}</span
							>
						{/if}
					{/each}
				</div>
			{/each}
		</div>
	{/if}
{/if}

<style>
	.chips { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 10px; }
	.chip {
		display: flex; align-items: baseline; gap: 5px;
		background: #171717; border: 1px solid #333; border-radius: 3px;
		color: #bbb; padding: 3px 8px; font: inherit; font-size: 12px; cursor: pointer;
	}
	.chip:hover { border-color: #555; color: #fff; }
	.chip.on { border-color: #2f6fd0; color: #fff; background: #16243a; }
	.chip .number { font-variant-numeric: tabular-nums; }
	.chip .count { color: #777; font-size: 11px; font-variant-numeric: tabular-nums; }
	.chip.live .count { color: #4ade80; }
	.sheet { font-variant-numeric: tabular-nums; font-size: 11px; overflow-x: auto; }
	.head, .row { display: flex; gap: 1px; }
	.head { margin-bottom: 2px; }
	.row { margin-bottom: 1px; }
	.addr { width: 34px; flex: none; color: #666; text-align: right; padding-right: 6px; }
	.col { width: 26px; flex: none; text-align: center; color: #666; }
	.cell {
		width: 26px; flex: none; text-align: center; border-radius: 2px;
		padding: 1px 0;
		color: rgba(235, 235, 235, calc(0.25 + 0.75 * var(--weight)));
		background: rgba(96, 165, 250, calc(0.16 * var(--weight)));
	}
	.cell.zero { color: #3c3c3c; background: none; }
	button.cell { border: 0; font: inherit; line-height: inherit; cursor: pointer; }
	.cell.testable:hover { outline: 1px solid #555; }
	.cell.forced { outline: 1px solid #d99a2b; background: rgba(217, 154, 43, 0.28); color: #ffd58a; }
	.cell.picked { outline: 2px solid #4f9cff; }
	.cell.forced.picked { outline: 2px solid #4f9cff; }
	.testbar { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
	.mode, .controls button, .goto button {
		background: #171717; border: 1px solid #333; border-radius: 3px;
		color: #bbb; padding: 3px 10px; font: inherit; font-size: 12px; cursor: pointer;
	}
	.mode.on { border-color: #d99a2b; color: #ffd58a; background: #2a2010; }
	.controls button:hover:not(:disabled), .goto button:hover { border-color: #555; color: #fff; }
	.controls button:disabled { opacity: 0.4; cursor: default; }
	.goto { display: flex; align-items: center; gap: 6px; font-size: 12px; color: #999; }
	.goto label { display: flex; align-items: center; gap: 6px; }
	.goto input, .level input[type='number'] {
		width: 64px; background: #171717; border: 1px solid #333; border-radius: 3px;
		color: #e0e0e0; padding: 2px 6px; font: inherit; font-size: 12px;
	}
	.controls {
		display: flex; flex-wrap: wrap; align-items: center; gap: 10px 16px;
		padding: 8px 10px; margin-bottom: 10px; background: #14110a;
		border: 1px solid #3a2f14; border-radius: 4px; font-size: 12px;
	}
	.selection { color: #c9a227; min-width: 180px; }
	.level { display: flex; align-items: center; gap: 8px; }
	.level input[type='range'] { width: 160px; }
	.pct { color: #888; width: 36px; font-variant-numeric: tabular-nums; }
	.buttons { display: flex; align-items: center; gap: 4px; }
	.buttons .gap { width: 10px; }
	.solo { display: flex; align-items: center; gap: 6px; color: #bbb; }
	.all { margin-left: auto; }
	.empty { color: #777; font-size: 13px; }
	.note { color: #c9a227; font-size: 12px; margin: 0 0 8px; }
</style>
