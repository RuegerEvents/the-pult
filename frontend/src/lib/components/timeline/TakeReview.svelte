<script lang="ts">
	/**
	 * What a recording caught, and which of it to keep.
	 *
	 * Mounted once at the root, beside the delete prompt, because a take stops wherever
	 * the operator happens to be looking — the timeline panel need not be open. The page
	 * that armed the recording opens it by itself (`stores/takeReview.ts`); every other
	 * page shows the chip in the top bar, so nothing waits on a tablet that went to sleep.
	 *
	 * A row per parameter, with its take and its flashes ticked apart: a second pass on
	 * which the jaw went wrong and the eyes went right keeps the eyes. Merge is one act and
	 * one Ctrl-Z, which puts the take back up for review; Discard throws the lot away,
	 * also undoably. *Later* closes this and keeps the take waiting.
	 */

	import Dialog from '$lib/components/Dialog.svelte';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { discardTake, mergeTake, reviewRows, reviewing, waiting, type ReviewRow } from '$lib/stores/takeReview.js';
	import { addToast } from '$lib/toasts.js';
	import { clock } from '$lib/waveform.js';
	import type { TimelineAction } from '$lib/generated/index.js';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');
	const sequences = collection('sequences');
	const cues = collection('cues');

	const open = $derived($waiting.find((w) => w.timeline.id === $reviewing) ?? null);
	const rows = $derived(open ? reviewRows(open.pending) : []);

	/** Ticked keys, `take:` or `flash:` and the parameter. Everything starts ticked. */
	let unticked = $state(new Set<string>());
	let busy = $state(false);
	let shownFor = '';
	$effect(() => {
		const id = open?.timeline.id ?? '';
		if (id !== shownFor) {
			shownFor = id;
			unticked = new Set();
		}
	});

	const id = (what: 'take' | 'flash', row: ReviewRow) => `${what}:${row.fixtureId}/${row.key}`;
	const cueKey = (eventId: string) => `cue:${eventId}`;

	/** The cues taken while it rolled, in the order they were taken. */
	const caughtCues = $derived([...(open?.pending.events ?? [])].sort((a, b) => a.at_ms - b.at_ms));

	function cueName(action: TimelineAction): string {
		const sequence = (sid: string) => $sequences.find((s) => s.id === sid)?.name ?? 'a sequence';
		if ('GoToCue' in action) {
			const { sequence_id, cue_id } = action.GoToCue;
			return `${sequence(sequence_id)} → ${$cues.find((c) => c.id === cue_id)?.name ?? 'a cue'}`;
		}
		if ('Off' in action) return `${sequence(action.Off.sequence_id)} off`;
		if ('GoNext' in action) return `Go on ${sequence(action.GoNext.sequence_id)}`;
		return Object.keys(action)[0] ?? 'a cue';
	}

	function toggle(key: string) {
		const next = new Set(unticked);
		if (next.has(key)) next.delete(key);
		else next.add(key);
		unticked = next;
	}

	function name(row: ReviewRow): string {
		const sequence = $sequences.find((s) => s.id === row.fixtureId);
		if (sequence) return `${sequence.name} · level`;
		const fixture = $fixtures.find((f) => f.id === row.fixtureId);
		const type = fixture ? $types.find((t) => t.id === fixture.fixture_type_id) : undefined;
		const parameter = type?.parameters.find((p) => parameterKey(p.kind) === row.key);
		return `${fixture?.name ?? 'A fixture'} · ${parameter ? displayLabel(parameter.kind) : row.key}`;
	}

	async function answer(merge: boolean) {
		if (!open || busy) return;
		busy = true;
		try {
			if (merge) {
				const chosen = (what: 'take' | 'flash') =>
					rows
						.filter((r) => (what === 'take' ? r.takes : r.flashes) > 0 && !unticked.has(id(what, r)))
						.map((r): [string, string] => [r.fixtureId, r.key]);
				const events = caughtCues.filter((e) => !unticked.has(cueKey(e.id))).map((e) => e.id);
				await mergeTake(open.timeline.id, chosen('take'), chosen('flash'), events);
			} else {
				await discardTake(open.timeline.id);
			}
			reviewing.set(null);
		} catch (e) {
			addToast(`The take: ${e instanceof Error ? e.message : e}`);
		} finally {
			busy = false;
		}
	}

	const nothingTicked = $derived(
		rows.every((r) => (r.takes === 0 || unticked.has(id('take', r))) && (r.flashes === 0 || unticked.has(id('flash', r)))) &&
			caughtCues.every((e) => unticked.has(cueKey(e.id)))
	);
</script>

{#if $waiting.length > 0 && $reviewing === null}
	<button
		class="chip"
		title="A recording stopped and is waiting for somebody to say what to keep"
		onclick={() => reviewing.set($waiting.find((w) => w.mine)?.timeline.id ?? $waiting[0].timeline.id)}
	>
		{$waiting.length === 1 ? `Take waiting · ${$waiting[0].timeline.name}` : `${$waiting.length} takes waiting`}
	</button>
{/if}

{#if open}
	<Dialog title={`Keep the take on ${open.timeline.name}?`} size="wide" onclose={() => reviewing.set(null)} onaccept={() => answer(true)}>
		<div class="review">
			<table>
				<thead>
					<tr><th>Parameter</th><th>From</th><th>Take</th><th>Flashes</th></tr>
				</thead>
				<tbody>
					{#each rows as row (`${row.fixtureId}/${row.key}`)}
						<tr>
							<td>{name(row)}</td>
							<td class="num">{clock(row.fromMs)}</td>
							<td>
								{#if row.takes > 0}
									<label>
										<input type="checkbox" checked={!unticked.has(id('take', row))} onchange={() => toggle(id('take', row))} />
										replaces from the touch{row.takes > 1 ? ` · ${row.takes} passes` : ''}
									</label>
								{:else}<span class="none">—</span>{/if}
							</td>
							<td>
								{#if row.flashes > 0}
									<label>
										<input type="checkbox" checked={!unticked.has(id('flash', row))} onchange={() => toggle(id('flash', row))} />
										adds {row.flashes}
									</label>
								{:else}<span class="none">—</span>{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
			{#if caughtCues.length > 0}
				<table class="cues">
					<thead>
						<tr><th>Cue taken</th><th>At</th><th>Keep</th></tr>
					</thead>
					<tbody>
						{#each caughtCues as event (event.id)}
							<tr>
								<td>{cueName(event.action)}</td>
								<td class="num">{clock(event.at_ms)}</td>
								<td>
									<label>
										<input type="checkbox" checked={!unticked.has(cueKey(event.id))} onchange={() => toggle(cueKey(event.id))} />
										an event
									</label>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</div>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => reviewing.set(null)}>Later</button>
			<span class="spacer"></span>
			<button class="btn btn-danger" disabled={busy || !$canEdit} onclick={() => answer(false)}>Discard</button>
			<button class="btn btn-primary" disabled={busy || !$canEdit || nothingTicked} onclick={() => answer(true)}>Merge</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.chip {
		background: #3a2a10;
		border: 1px solid #a16207;
		border-radius: 12px;
		color: #fde68a;
		padding: 2px 10px;
		font: inherit;
		font-size: var(--font-xs, 11px);
		cursor: pointer;
		white-space: nowrap;
	}
	.chip:hover {
		border-color: #f59e0b;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 12px;
	}
	th {
		text-align: left;
		color: var(--text-dim, #888);
		font-weight: 500;
		padding: 4px 8px;
		border-bottom: 1px solid var(--line, #222);
	}
	td {
		padding: 5px 8px;
		border-bottom: 1px solid var(--line, #222);
		color: #ddd;
	}
	td.num {
		font-variant-numeric: tabular-nums;
		color: #aaa;
	}
	label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		cursor: pointer;
	}
	table.cues {
		margin-top: 12px;
	}
	.none {
		color: #555;
	}
	.spacer {
		flex: 1;
	}
</style>
