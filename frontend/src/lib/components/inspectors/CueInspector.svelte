<script lang="ts">
	/**
	 * A cue's values, as a list: every capture, which fixture and parameter, what it is
	 * set to and its own timing — and the one place a value is taken out of a cue
	 * without loading the cue into the programmer.
	 *
	 * Ticked rows go together, in one write of the column, so one Ctrl-Z. A value taken
	 * out of the cue on stage lets the light go where the stack says (task 93).
	 */

	import type { Cue, ParameterCapture } from '$lib/generated/index.js';
	import { displayLabel, formatValue, parameterKey } from '$lib/patch.js';
	import { collection, showData } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';

	let { rows }: { rows: Cue[] } = $props();

	const fixtures = collection('fixtures');
	const nameOf = (id: string) => $fixtures.find((f) => f.id === id)?.name ?? id.slice(0, 6);
	const keyOf = (c: ParameterCapture) => `${c.fixture_id}/${parameterKey(c.parameter_kind)}`;

	/** The captures in reading order: by fixture name, then as the cue lists them. */
	const listed = (cue: Cue) =>
		[...cue.captures].sort((a, b) => nameOf(a.fixture_id).localeCompare(nameOf(b.fixture_id), undefined, { numeric: true }));

	/** Ticked, per cue, by `fixture/key`. */
	let ticked = $state<Record<string, Set<string>>>({});
	const tickedIn = (cue: Cue) => ticked[cue.id] ?? new Set<string>();
	function tick(cue: Cue, key: string, on: boolean) {
		const next = new Set(tickedIn(cue));
		if (on) next.add(key);
		else next.delete(key);
		ticked = { ...ticked, [cue.id]: next };
	}

	async function remove(cue: Cue, keys: Set<string>) {
		if (keys.size === 0) return;
		await showData().cues.byId(cue.id).captures.set(cue.captures.filter((c) => !keys.has(keyOf(c))));
		ticked = { ...ticked, [cue.id]: new Set() };
	}

	const timing = (c: ParameterCapture) => {
		const parts: string[] = [];
		if (c.fade_in_ms) parts.push(`in ${c.fade_in_ms}`);
		if (c.fade_out_ms) parts.push(`out ${c.fade_out_ms}`);
		if (c.delay_in_ms) parts.push(`delay ${c.delay_in_ms}`);
		return parts.join(' · ');
	};
</script>

{#each rows as cue (cue.id)}
	{@const picked = tickedIn(cue)}
	<div class="cue">
		{#if rows.length > 1}<span class="label">{cue.number.toFixed(1)} · {cue.name}</span>{/if}
		{#if cue.captures.length === 0}
			<p class="note">This cue sets nothing: everything shown during it is tracked or resting.</p>
		{:else}
			<div class="head">
				<span class="note">{cue.captures.length} value{cue.captures.length === 1 ? '' : 's'}</span>
				<span class="spacer"></span>
				{#if $canEdit}
					<button class="btn btn-ghost btn-small" disabled={picked.size === 0} onclick={() => void remove(cue, picked)}
						>Remove {picked.size || ''} from cue</button
					>
				{/if}
			</div>
			<table>
				<tbody>
					{#each listed(cue) as capture (keyOf(capture))}
						{@const key = keyOf(capture)}
						<tr class:off={picked.has(key)}>
							{#if $canEdit}
								<td class="tick">
									<input
										type="checkbox"
										checked={picked.has(key)}
										aria-label="Pick {nameOf(capture.fixture_id)} {displayLabel(capture.parameter_kind)}"
										onchange={(e) => tick(cue, key, e.currentTarget.checked)}
									/>
								</td>
							{/if}
							<td class="fixture">{nameOf(capture.fixture_id)}</td>
							<td>{displayLabel(capture.parameter_kind)}</td>
							<td class="mono">
								{#if capture.effect}
									<span class="chip-effect">effect</span>
								{:else}
									{formatValue(capture.value)}
								{/if}
								{#if capture.preset}<span class="ref" title="From a preset">◇</span>{/if}
							</td>
							<td class="timing">{timing(capture)}</td>
							{#if $canEdit}
								<td class="acts">
									<button class="x" title="Remove from this cue" aria-label="Remove {nameOf(capture.fixture_id)} {displayLabel(capture.parameter_kind)} from this cue" onclick={() => void remove(cue, new Set([key]))}>✕</button>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</div>
{/each}

<style>
	.cue {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.head {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.spacer {
		flex: 1;
	}
	.label {
		font-weight: 600;
	}
	.note {
		color: var(--text-dim);
		font-size: var(--font-sm);
		margin: 0;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	td {
		padding: 3px 6px 3px 0;
		border-bottom: 1px solid var(--line);
		vertical-align: middle;
	}
	tr.off td {
		opacity: 0.5;
	}
	.fixture {
		color: var(--text);
		white-space: nowrap;
	}
	.mono {
		font-variant-numeric: tabular-nums;
	}
	.timing {
		color: var(--text-faint);
		font-size: var(--font-xs);
		white-space: nowrap;
	}
	.ref {
		margin-left: 4px;
		color: var(--text-dim);
	}
	.chip-effect {
		font-size: var(--font-xs);
		padding: 0 6px;
		border-radius: 8px;
		background: var(--bg-raised, #2a2a2a);
	}
	.tick,
	.acts {
		width: 1%;
	}
	.x {
		background: none;
		border: none;
		color: var(--text-dim);
		cursor: pointer;
		padding: 0 4px;
	}
	.x:hover {
		color: var(--bad, #f87171);
	}
	.btn-small {
		min-height: 24px;
		padding: 0 10px;
		font-size: 12px;
	}
</style>
