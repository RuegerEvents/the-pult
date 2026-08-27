<script lang="ts">
	/**
	 * The store menu.
	 *
	 * The spec asks for it to show "which fixtures, attributes and values are stored",
	 * and to let the operator deselect what should not be — so the list of what is
	 * about to be written is the menu, rather than a confirmation bolted onto a button.
	 *
	 * Merge is the default and Replace is only offered for a cue that already exists,
	 * because there is nothing to replace in a cue being made here and now.
	 */

	import type { Cue, Sequence } from '$lib/generated/index.js';
	import { createCue } from '$lib/cues.js';
	import { formatValue, kindLabel } from '$lib/patch.js';
	import { clear, entries, storeInto } from '$lib/stores/programmer.js';
	import { collection, showData } from '$lib/stores/show.js';
	import { addToast } from '$lib/toasts.js';
	import { focusOnMount } from '$lib/actions.js';

	let { onclose }: { onclose: () => void } = $props();

	const fixtures = collection('fixtures');
	const sequences = collection('sequences');
	const cues = collection('cues');

	let sequenceId = $state<string | null>(null);
	let target = $state<'new' | 'existing'>('new');
	let cueId = $state<string | null>(null);
	let name = $state('');
	let mode = $state<'merge' | 'replace'>('merge');
	let keep = $state(false);
	let storing = $state(false);

	/// What the operator has unticked. Everything else is stored, so an entry that
	/// arrives while the menu is open — another console programming alongside — is
	/// included rather than silently left out.
	let dropped = $state(new Set<string>());
	const include = $derived(
		new Set($entries.filter((entry) => !dropped.has(entry.id)).map((entry) => entry.id))
	);

	const sequence = $derived(
		$sequences.find((s) => s.id === sequenceId) ?? ($sequences[0] as Sequence | undefined) ?? null
	);
	const cuesInSequence = $derived(
		sequence ? sequence.cue_ids.map((id) => $cues.find((c) => c.id === id)).filter((c): c is Cue => !!c) : []
	);
	const cue = $derived(cuesInSequence.find((c) => c.id === cueId) ?? null);
	const nameOf = (fixtureId: string) =>
		$fixtures.find((f) => f.id === fixtureId)?.name ?? fixtureId.slice(0, 6);

	const canStore = $derived(
		include.size > 0 &&
			!!sequence &&
			(target === 'new' ? name.trim().length > 0 : cue !== null)
	);

	function tick(id: string, on: boolean) {
		const next = new Set(dropped);
		if (on) next.delete(id);
		else next.add(id);
		dropped = next;
	}

	async function store() {
		if (!sequence || storing) return;
		storing = true;
		try {
			if (target === 'new') {
				await createCue(showData(), sequence, {
					name: name.trim(),
					captures: $entries
						.filter((entry) => include.has(entry.id))
						.map((entry) => ({
							fixture_id: entry.fixture_id,
							parameter_kind: entry.parameter_kind,
							value: entry.value,
							fade_in_ms: 0,
							fade_out_ms: 0,
							delay_in_ms: 0
						}))
				});
			} else if (cue) {
				await storeInto(cue, mode, include);
			}
			if (!keep) await clear({ keepLocked: true });
			onclose();
		} catch (e) {
			addToast(e instanceof Error ? e.message : 'that would not store');
		} finally {
			storing = false;
		}
	}
</script>

<div
	class="scrim"
	role="presentation"
	onclick={onclose}
	onkeydown={(e) => e.key === 'Escape' && onclose()}
>
	<div class="menu" role="dialog" tabindex="-1" aria-label="Store" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
		<header>
			<h2>Store</h2>
			<button class="icon" aria-label="Close" onclick={onclose}>✕</button>
		</header>

		{#if $entries.length === 0}
			<p class="empty">The programmer is empty, so there is nothing to store.</p>
		{:else}
			<div class="list">
				<table>
					<thead>
						<tr><th></th><th>Fixture</th><th>Parameter</th><th>Value</th></tr>
					</thead>
					<tbody>
						{#each $entries as entry (entry.id)}
							<tr class:off={!include.has(entry.id)}>
								<td>
									<input
										type="checkbox"
										checked={include.has(entry.id)}
										aria-label="Store {nameOf(entry.fixture_id)} {kindLabel(entry.parameter_kind)}"
										onchange={(e) => tick(entry.id, e.currentTarget.checked)}
									/>
								</td>
								<td>{nameOf(entry.fixture_id)}</td>
								<td>{kindLabel(entry.parameter_kind)}</td>
								<td class="mono">{formatValue(entry.value)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>

			<div class="target">
				<label class="field">
					Sequence
					<select
						value={sequence?.id ?? ''}
						onchange={(e) => {
							sequenceId = e.currentTarget.value;
							cueId = null;
						}}
					>
						{#each $sequences as s (s.id)}
							<option value={s.id}>{s.name}</option>
						{/each}
					</select>
				</label>

				<div class="choice">
					<label>
						<input type="radio" value="new" bind:group={target} />
						New cue
					</label>
					<label>
						<input type="radio" value="existing" bind:group={target} disabled={cuesInSequence.length === 0} />
						Existing cue
					</label>
				</div>

				{#if target === 'new'}
					<input
						class="text"
						placeholder="Cue name…"
						bind:value={name}
						use:focusOnMount
						onkeydown={(e) => e.key === 'Enter' && canStore && store()}
					/>
				{:else}
					<select bind:value={cueId}>
						<option value={null}>Choose a cue…</option>
						{#each cuesInSequence as c (c.id)}
							<option value={c.id}>{c.number.toFixed(1)} · {c.name}</option>
						{/each}
					</select>
					<div class="choice">
						<label><input type="radio" value="merge" bind:group={mode} /> Merge</label>
						<label><input type="radio" value="replace" bind:group={mode} /> Replace</label>
					</div>
					<p class="note">
						{mode === 'merge'
							? 'Everything else the cue says is kept.'
							: 'The cue will say only what is ticked above.'}
					</p>
				{/if}

				<label class="check">
					<input type="checkbox" bind:checked={keep} />
					Keep the programmer after storing
				</label>
			</div>
		{/if}

		<footer>
			<button class="ghost" onclick={onclose}>Cancel</button>
			<button class="primary" disabled={!canStore || storing} onclick={store}>
				{storing ? 'Storing…' : 'Store'}
			</button>
		</footer>
	</div>
</div>

<style>
	.scrim {
		position: fixed;
		inset: 0;
		z-index: 40;
		display: grid;
		place-items: center;
		background: #000a;
		padding: 20px;
	}

	.menu {
		display: flex;
		flex-direction: column;
		width: min(560px, 100%);
		max-height: 100%;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		overflow: hidden;
	}

	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 14px;
		border-bottom: 1px solid var(--line);
	}
	h2 {
		font-size: var(--font-sm);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}

	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		cursor: pointer;
	}
	.icon:hover {
		color: var(--bad);
	}

	.empty {
		padding: 20px 14px;
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
	}

	.list {
		overflow: auto;
		max-height: 40vh;
		border-bottom: 1px solid var(--line);
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	th {
		position: sticky;
		top: 0;
		background: var(--bg-panel);
		text-align: left;
		font-weight: 500;
		color: var(--text-dim);
		padding: 6px 10px;
		border-bottom: 1px solid var(--line);
	}
	td {
		padding: 4px 10px;
		border-bottom: 1px solid #ffffff08;
	}
	tr.off td {
		color: var(--text-faint);
	}
	.mono {
		font-family: monospace;
	}

	.target {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 12px 14px;
	}
	.field {
		display: flex;
		align-items: center;
		gap: 8px;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.choice {
		display: flex;
		gap: 14px;
		color: var(--text);
		font-size: var(--font-sm);
	}
	.choice label,
	.check {
		display: flex;
		align-items: center;
		gap: 5px;
		cursor: pointer;
	}
	.check {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	select,
	.text {
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 6px;
	}
	select:focus,
	.text:focus {
		outline: none;
		border-color: var(--accent);
	}

	footer {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--line);
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 4px 12px;
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.ghost:hover {
		border-color: var(--line-input);
		color: var(--text-bright);
	}

	.primary {
		background: var(--accent-solid);
		border: none;
		border-radius: var(--radius);
		color: #fff;
		padding: 5px 14px;
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.primary:disabled {
		background: var(--line-strong);
		color: var(--text-faint);
		cursor: not-allowed;
	}
</style>
