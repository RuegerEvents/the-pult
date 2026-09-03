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
	 */

	import type { UniverseTraffic } from '$lib/generated/index.js';
	import { channelWeight, universeFocus } from '$lib/wire.js';

	type Props = {
		of: UniverseTraffic;
		focus: string | null;
		ask: (focus: string | null) => void;
	};
	const { of, focus, ask }: Props = $props();

	const COLUMNS = 16;

	const showing = $derived(of.focused?.universe ?? null);
	const rows = $derived.by(() => {
		const channels = of.focused?.channels ?? [];
		const out: { from: number; values: number[] }[] = [];
		for (let at = 0; at < channels.length; at += COLUMNS) {
			out.push({ from: at + 1, values: channels.slice(at, at + COLUMNS) });
		}
		return out;
	});

	/** "moving", or how long ago it stopped. */
	const activity = (ms: number) => {
		if (ms < 500) return 'moving';
		if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
		return `${Math.round(ms / 60_000)}m`;
	};
</script>

{#if of.universes.length === 0}
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

	{#if of.focused}
		<!-- The one place a universe number is spelled as a focus string is
		     `universeFocus`; nothing here invents its own spelling. -->
		{#if focus && focus !== universeFocus(of.focused.universe)}
			<p class="note">
				Showing universe {of.focused.universe}; this output does not carry universe {focus}.
			</p>
		{/if}
		<div class="sheet" role="table" aria-label="Universe {of.focused.universe}">
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
						<span
							class="cell"
							class:zero={value === 0}
							role="cell"
							style="--weight: {channelWeight(value)}"
							title="Channel {row.from + column}">{value}</span
						>
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
	.empty { color: #777; font-size: 13px; }
	.note { color: #c9a227; font-size: 12px; margin: 0 0 8px; }
</style>
