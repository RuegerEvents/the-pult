<script lang="ts">
	/**
	 * One sequence, as a list of what its cues actually do.
	 *
	 * The Playback panel is the *runner* — every sequence at once, a Go under each
	 * thumb. This is the **editor** of one: numbers, names, both fade times, the curve,
	 * the follow, and how many parameters each cue holds. Two panels rather than one
	 * because the two questions are asked in different rooms — running a show and
	 * building one — and a panel that answered both would be worse at each.
	 *
	 * **A click shows and does not take.** Clicking a cue to see what is in it is the
	 * commonest thing anybody does with a cue list, and on a desk where that took the
	 * cue it would be the commonest way to put the wrong look on stage. So a click sets
	 * `cueInView` — which is what the fixture sheet draws — and a double-click, or the
	 * Go column, takes.
	 *
	 * Editing is behind the tile's lock, for the reason the Patch panel is: a fade time
	 * changed by a mis-hit is a look that arrives at the wrong moment, and nothing on
	 * stage says why. Go, Back and Off stay live, because they are what a show is run
	 * with.
	 */

	import type { Cue, Sequence } from '$lib/generated/index.js';
	import { focusOnMount } from '$lib/actions.js';
	import { createCue, orderedCues, reorderCueIds } from '$lib/cues.js';
	import { CURVE_LABELS, CURVES } from '$lib/fade.js';
	import { collection } from '$lib/stores/show.js';
	import { cueInView, focusedCueSheet, viewCue } from '$lib/stores/cues.js';
	import { editing } from '$lib/stores/editing.js';
	import { beginEdit, editingCue } from '$lib/stores/programmer.js';
	import { getDataContext } from '$lib/ws/context.js';

	const data = getDataContext();
	const unlocked = editing('cues');
	const sequences = collection('sequences');
	const cues = collection('cues');
	/** How many of a cue's captures are references, for the Caps column. */
	const referencing = (cue: Cue) => cue.captures.filter((c) => c.preset).length;

	/** Which sequence's cues are on screen. Falls back to the first the show has. */
	let chosen = $state<string | null>(null);
	const sequence = $derived(
		$sequences.find((s) => s.id === chosen) ?? ($sequences[0] as Sequence | undefined) ?? null
	);
	const rows = $derived(sequence ? orderedCues(sequence, $cues) : []);
	const activeIndex = $derived(sequence?.active_cue_index ?? null);

	let renaming = $state<string | null>(null);
	let draft = $state('');
	let adding = $state(false);
	let newName = $state('');
	let insertAfter = $state<string | null>(null);
	let dragFrom = $state<number | null>(null);
	let dragOver = $state<number | null>(null);

	// ── Running it ──────────────────────────────────────────────────────────────

	// The time goes with the command, the way it does everywhere else: a Go that
	// carries when it happened anchors the cue's fades at one millisecond on every
	// station rather than at whenever each of them got the message.
	const go = () => sequence && data.sequences.byId(sequence.id).goNext({ at: Date.now() });
	const goToCue = (cueId: string) =>
		sequence && data.sequences.byId(sequence.id).goToCue({ cueId, at: Date.now() });
	const off = () => sequence && data.sequences.byId(sequence.id).off({ at: Date.now() });

	/**
	 * Back: the cue before the one that is up.
	 *
	 * A jump rather than an undo — `goToCue` releases every key only the later cues
	 * capture, over the cue's down time, which is what "go back one" means on every
	 * other desk. At the top of the list there is nowhere to go and the button is out.
	 */
	function back() {
		if (activeIndex === null || activeIndex <= 0 || !sequence) return;
		void goToCue(sequence.cue_ids[activeIndex - 1]);
	}

	// ── Editing it ──────────────────────────────────────────────────────────────

	async function setTiming(cueId: string, patch: Partial<Cue>) {
		const entity = data.cues.byId(cueId);
		if (patch.fade_in_ms !== undefined) await entity.fade_in_ms.set(patch.fade_in_ms);
		if (patch.fade_out_ms !== undefined) await entity.fade_out_ms.set(patch.fade_out_ms);
		// `null` is a real value here and not "unset": it is the cue saying nothing, so
		// the show's own default answers per parameter. Checked against `undefined`.
		if (patch.easing !== undefined) await entity.easing.set(patch.easing);
		if (patch.follow_mode !== undefined) await entity.follow_mode.set(patch.follow_mode);
	}

	async function rename(cueId: string) {
		const trimmed = draft.trim();
		if (trimmed) await data.cues.byId(cueId).name.set(trimmed);
		renaming = null;
	}

	async function add() {
		if (!sequence || !newName.trim()) return;
		await createCue(data, sequence, $cues, {
			name: newName.trim(),
			after: insertAfter ?? undefined
		});
		newName = '';
		adding = false;
		insertAfter = null;
	}

	async function remove(cueId: string) {
		if (!sequence) return;
		await data.sequences.byId(sequence.id).cue_ids.set(sequence.cue_ids.filter((id) => id !== cueId));
		await data.cues.byId(cueId).delete();
	}

	/**
	 * Drop a dragged cue somewhere else. Only the order changes: `Cue.number` is left
	 * alone, so a cue an operator calls "cue 5" is still cue 5 after somebody moved
	 * cue 2.
	 */
	async function drop(from: number, to: number) {
		if (sequence) {
			const next = reorderCueIds(sequence.cue_ids, from, to);
			if (next !== sequence.cue_ids) await data.sequences.byId(sequence.id).cue_ids.set(next);
		}
		dragFrom = null;
		dragOver = null;
	}

	/** A double-click takes; the timer is what tells one apart from the click that shows. */
	let clickTimer: ReturnType<typeof setTimeout> | null = null;
	function clickRow(cueId: string) {
		if (clickTimer !== null) {
			clearTimeout(clickTimer);
			clickTimer = null;
			void goToCue(cueId);
			return;
		}
		clickTimer = setTimeout(() => {
			clickTimer = null;
			viewCue(cueId);
		}, 220);
	}

	const followLabel = (cue: Cue) =>
		typeof cue.follow_mode === 'object' && 'FollowAfter' in cue.follow_mode
			? cue.follow_mode.FollowAfter.delay_ms
			: null;
</script>

<!-- Pointerdown anywhere in the panel says "this is the cue sheet Space means". Last
     touched rather than a mode of its own: a console with three of these open has to
     answer which, and that is the answer an operator already has in their head. -->
<div
	class="sheet"
	role="presentation"
	onpointerdown={() => sequence && focusedCueSheet.set(sequence.id)}
>
	<header>
		<div class="tabs">
			{#each $sequences as s (s.id)}
				<button class="tab" class:on={s.id === sequence?.id} onclick={() => (chosen = s.id)}>
					{s.name}
					{#if s.active_cue_index !== null}<span class="dot">●</span>{/if}
				</button>
			{/each}
		</div>
		<span class="spacer"></span>
		<button class="go" onclick={go} disabled={!sequence || rows.length === 0}>GO</button>
		<button class="chip" onclick={back} disabled={activeIndex === null || activeIndex <= 0}>
			Back
		</button>
		<button class="chip" onclick={off} disabled={activeIndex === null}>Off</button>
	</header>

	{#if !sequence}
		<p class="empty">No sequences yet. The Playback panel is where one is made.</p>
	{:else if rows.length === 0}
		<p class="empty">This sequence has no cues.</p>
	{:else}
		<div class="scroll">
			<table>
				<thead>
					<tr>
						<th class="num">Cue</th>
						<th>Name</th>
						<th class="ms" title="What this cue's captures take on the way up">In</th>
						<th class="ms" title="And on the way down. Empty means the in time.">Out</th>
						<th>Curve</th>
						<th>Follow</th>
						<th class="count" title="Parameters this cue captures">Caps</th>
						<th class="acts"></th>
					</tr>
				</thead>
				<tbody>
					{#each rows as cue, index (cue.id)}
						<tr
							class:active={activeIndex === index}
							class:shown={$cueInView === cue.id}
							class:editing={$editingCue === cue.id}
							class:drop-here={dragOver === index && dragFrom !== null && dragFrom !== index}
							draggable={$unlocked}
							ondragstart={() => (dragFrom = index)}
							ondragover={(e) => {
								e.preventDefault();
								dragOver = index;
							}}
							ondragleave={() => {
								if (dragOver === index) dragOver = null;
							}}
							ondrop={(e) => {
								e.preventDefault();
								if (dragFrom !== null) drop(dragFrom, index);
							}}
							ondragend={() => {
								dragFrom = null;
								dragOver = null;
							}}
						>
							<td class="num">
								<!-- The Go column: this one takes, where the row shows. -->
								<button
									class="take"
									title="Take this cue"
									aria-label="Take cue {cue.number.toFixed(1)}"
									onclick={() => goToCue(cue.id)}
								>
									{cue.number.toFixed(1)}
								</button>
							</td>
							<td class="name">
								{#if renaming === cue.id && $unlocked}
									<form
										onsubmit={(e) => {
											e.preventDefault();
											rename(cue.id);
										}}
									>
										<input
											class="text"
											bind:value={draft}
											use:focusOnMount
											onblur={() => rename(cue.id)}
											onkeydown={(e) => {
												if (e.key === 'Escape') renaming = null;
												e.stopPropagation();
											}}
										/>
									</form>
								{:else}
									<button
										class="row"
										ondblclick={() => {
											if (!$unlocked) return;
											renaming = cue.id;
											draft = cue.name;
										}}
										onclick={() => clickRow(cue.id)}
									>
										{cue.name}
									</button>
								{/if}
							</td>
							<td class="ms">
								{#if $unlocked}
									<input
										class="num-in"
										type="number"
										min="0"
										step="100"
										value={cue.fade_in_ms}
										aria-label="Fade in for cue {cue.number.toFixed(1)}"
										onchange={(e) => setTiming(cue.id, { fade_in_ms: Number(e.currentTarget.value) })}
									/>
								{:else}
									{cue.fade_in_ms}
								{/if}
							</td>
							<td class="ms">
								{#if $unlocked}
									<input
										class="num-in"
										type="number"
										min="0"
										step="100"
										placeholder="in"
										value={cue.fade_out_ms || ''}
										aria-label="Fade out for cue {cue.number.toFixed(1)}"
										onchange={(e) =>
											setTiming(cue.id, { fade_out_ms: Number(e.currentTarget.value) || 0 })}
									/>
								{:else}
									{cue.fade_out_ms || '—'}
								{/if}
							</td>
							<td>
								{#if $unlocked}
									<select
										value={cue.easing ?? ''}
										aria-label="Curve for cue {cue.number.toFixed(1)}"
										onchange={(e) =>
											setTiming(cue.id, { easing: (e.currentTarget.value || null) as Cue['easing'] })}
									>
										<!-- The show answers differently per parameter, so this says
										     whose answer it is rather than naming one of them. -->
										<option value="">show's</option>
										{#each CURVES as curve (curve)}
											<option value={curve}>{CURVE_LABELS[curve]}</option>
										{/each}
									</select>
								{:else}
									{cue.easing ? CURVE_LABELS[cue.easing] : "show's"}
								{/if}
							</td>
							<td class="follow">
								{#if $unlocked}
									<select
										value={cue.follow_mode === 'Manual' ? 'Manual' : 'FollowAfter'}
										aria-label="Follow for cue {cue.number.toFixed(1)}"
										onchange={(e) =>
											setTiming(cue.id, {
												follow_mode:
													e.currentTarget.value === 'Manual'
														? 'Manual'
														: { FollowAfter: { delay_ms: followLabel(cue) ?? 0 } }
											})}
									>
										<option value="Manual">On Go</option>
										<option value="FollowAfter">After</option>
									</select>
									{#if followLabel(cue) !== null}
										<input
											class="num-in"
											type="number"
											min="0"
											step="100"
											value={followLabel(cue)}
											aria-label="Follow delay for cue {cue.number.toFixed(1)}"
											onchange={(e) =>
												setTiming(cue.id, {
													follow_mode: { FollowAfter: { delay_ms: Number(e.currentTarget.value) } }
												})}
										/>
									{/if}
								{:else if followLabel(cue) !== null}
									after {followLabel(cue)}
								{:else}
									On Go
								{/if}
							</td>
							<td class="count">
								{cue.captures.length}
								<!-- How many of them are references rather than numbers, which is
								     what says at a glance that editing a palette will move this
								     cue. A capture whose preset is gone still counts: it is still
								     a reference, and the sheet says "preset missing" where it is
								     looked at. -->
								{#if referencing(cue) > 0}
									<span
										class="refs"
										title="{referencing(cue)} of them reference a preset"
									>◇{referencing(cue)}</span>
								{/if}
							</td>
							<td class="acts">
								<button
									class="icon"
									class:on={$editingCue === cue.id}
									title="Load this cue into the programmer to change it"
									onclick={() => beginEdit(cue, sequence)}>Edit</button
								>
								{#if $unlocked}
									<button
										class="icon"
										title="Insert a cue after this one"
										aria-label="Insert after {cue.name}"
										onclick={() => {
											adding = true;
											insertAfter = cue.id;
										}}>⤵</button
									>
									<button
										class="icon danger"
										title="Delete cue"
										aria-label="Delete {cue.name}"
										onclick={() => remove(cue.id)}>✕</button
									>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}

	{#if $unlocked && sequence}
		<footer>
			{#if adding}
				<form
					onsubmit={(e) => {
						e.preventDefault();
						add();
					}}
				>
					<input
						class="text"
						placeholder="Cue name…"
						bind:value={newName}
						use:focusOnMount
						onkeydown={(e) => {
							if (e.key === 'Escape') {
								adding = false;
								newName = '';
							}
						}}
					/>
					<button class="chip" type="submit">Add</button>
					<button
						class="chip"
						type="button"
						onclick={() => {
							adding = false;
							newName = '';
							insertAfter = null;
						}}>Cancel</button
					>
				</form>
			{:else}
				<button
					class="chip"
					onclick={() => {
						adding = true;
						insertAfter = null;
					}}>+ Cue</button
				>
			{/if}
		</footer>
	{/if}
</div>

<style>
	.sheet {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	header,
	footer {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 10px;
		flex-shrink: 0;
	}
	header {
		border-bottom: 1px solid var(--line);
	}
	footer {
		border-top: 1px solid var(--line);
	}
	footer form {
		display: flex;
		gap: 6px;
		align-items: center;
	}

	.tabs {
		display: flex;
		gap: 2px;
		overflow-x: auto;
	}
	.tab {
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.tab:hover {
		color: var(--text);
	}
	.tab.on {
		color: var(--text-bright);
		border-bottom-color: var(--accent);
	}
	.dot {
		color: var(--live);
		font-size: 8px;
		margin-left: 4px;
	}

	.spacer {
		flex: 1;
	}

	.go {
		font-size: var(--font-sm);
		font-weight: 700;
		letter-spacing: 0.1em;
		padding: 4px 16px;
		border-radius: var(--radius);
		border: 2px solid var(--live);
		background: none;
		color: var(--live);
		cursor: pointer;
	}
	.go:hover:not(:disabled) {
		background: #f59e0b22;
	}
	.go:disabled {
		border-color: var(--line-strong);
		color: var(--text-faint);
		cursor: not-allowed;
	}

	.chip {
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 4px 10px;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		color: var(--text-bright);
		border-color: var(--line-input);
	}
	.chip:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}

	.empty {
		padding: 20px 10px;
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
	}

	.scroll {
		flex: 1;
		min-height: 0;
		overflow: auto;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	thead th {
		position: sticky;
		top: 0;
		background: var(--bg-panel);
		text-align: left;
		font-weight: 500;
		color: var(--text-dim);
		padding: 5px 8px;
		border-bottom: 1px solid var(--line-strong);
		white-space: nowrap;
	}
	td {
		padding: 2px 8px;
		border-bottom: 1px solid #ffffff08;
		color: var(--text-dim);
	}
	tr.active td {
		background: #f59e0b18;
		color: #f0d090;
	}
	/* Being looked at and being up are different things, and a cue can be both. */
	tr.shown td {
		box-shadow: inset 0 0 0 1px var(--accent);
	}
	tr.editing td {
		box-shadow: inset 0 0 0 1px #4a9eff88;
	}
	tr.drop-here td {
		border-top: 2px solid var(--accent);
	}

	.num {
		width: 4rem;
	}
	.take {
		background: none;
		border: none;
		font-family: monospace;
		font-size: var(--font-xs);
		color: var(--text-faint);
		cursor: pointer;
		padding: 2px 4px;
	}
	.take:hover {
		color: var(--live);
	}
	tr.active .take {
		color: var(--live);
	}

	.name {
		min-width: 8rem;
	}
	.row {
		background: none;
		border: none;
		color: inherit;
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		cursor: pointer;
		padding: 2px 0;
		width: 100%;
	}

	.ms,
	.count {
		width: 5rem;
		font-family: monospace;
		font-size: var(--font-xs);
	}
	.count {
		width: 4.5rem;
		text-align: right;
	}
	.refs {
		margin-left: 4px;
		color: var(--src-tracked);
	}
	.follow {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.num-in {
		width: 4.5rem;
	}
	.num-in,
	select,
	.text {
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 5px;
	}
	.text {
		font-size: var(--font-sm);
	}
	.num-in:focus,
	select:focus,
	.text:focus {
		outline: none;
		border-color: var(--accent);
	}

	.acts {
		white-space: nowrap;
		text-align: right;
	}
	.icon {
		background: none;
		border: 1px solid transparent;
		border-radius: 3px;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 5px;
		cursor: pointer;
	}
	.icon:hover,
	.icon.on {
		border-color: var(--accent);
		color: var(--accent);
	}
	.icon.danger:hover {
		border-color: var(--bad);
		color: var(--bad);
	}
</style>
