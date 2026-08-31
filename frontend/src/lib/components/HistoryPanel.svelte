<script lang="ts">
	/**
	 * What has changed, and who changed it.
	 *
	 * Shared on purpose. Undo is per person — you can only take back your own work —
	 * but *seeing* is not: on a two-operator tech the useful question is usually
	 * "what just happened", and the answer is often somebody else. So the list shows
	 * everyone, colour-coded and named, and marks the entries that are yours to take
	 * back.
	 *
	 * Read rather than subscribed. The oplog is infrastructure, not a replicated
	 * collection, so there is nothing to watch — the panel re-reads when it is opened
	 * and after anything this browser does.
	 */

	import type { HistoryEntry } from '$lib/generated/index.js';
	import { ago, colourOf, describeChange, pluginDatumName } from '$lib/users.js';
	import { historyVersion, readHistory, undo } from '$lib/stores/undo.js';
	import { collection, show } from '$lib/stores/show.js';
	import { users, userId } from '$lib/stores/user.js';

	const fixtures = collection('fixtures');
	const cues = collection('cues');
	const sequences = collection('sequences');
	const pluginData = collection('plugin_data');

	let entries = $state<HistoryEntry[]>([]);

	/**
	 * How far back this show keeps its history.
	 *
	 * Asked for in full rather than a round hundred, because the log is now pruned to
	 * this number and the end of the list is a real end. The backend clamps it to the
	 * same value, so asking for more than the show keeps is not a way to see more.
	 */
	const depth = $derived($show?.history_depth ?? 500);

	/**
	 * Whether the list has reached the oldest change the show still holds.
	 *
	 * Read from the shape of the answer rather than from a new API: the log is pruned
	 * to `depth` authored changes, so a full answer is one that ends where the show
	 * does. Before pruning existed the rows past here were still on disk, invisible;
	 * now they are gone, and an empty scroll would look like a bug rather than a
	 * boundary.
	 */
	const atTheEnd = $derived(entries.length >= depth);
	/** Redrawn on a timer so "2m ago" does not sit there saying "just now". */
	let now = $state(Date.now());

	/**
	 * Names for the ids in a path.
	 *
	 * A uuid in a change list is a wall of hex that hides the two words either side
	 * of it. Anything the show can name gets named; anything deleted since keeps its
	 * short id, which is honest — the thing is gone and there is nothing to call it.
	 */
	const names = $derived.by(() => {
		const map = new Map<string, string>();
		for (const f of $fixtures) map.set(f.id, f.name);
		for (const c of $cues) map.set(c.id, c.name);
		for (const s of $sequences) map.set(s.id, s.name);
		// A store row's id is a hash of what it names, so it has no name of its
		// own to fall back on — it gets one made of the three things it is.
		for (const d of $pluginData) map.set(d.id, pluginDatumName(d));
		return map;
	});

	// Every entry has an author — the backend sends only what somebody asked for — so
	// the fallback is for a user row deleted since, not for the console's own writes.
	const nameOf = (id: string | null | undefined) =>
		$users.find((u) => u.id === id)?.name ?? 'somebody';

	$effect(() => {
		// Re-read whenever this browser has changed something.
		void $historyVersion;
		readHistory(depth).then((h) => (entries = h));
	});

	$effect(() => {
		const timer = setInterval(() => (now = Date.now()), 15_000);
		return () => clearInterval(timer);
	});
</script>

<div class="history">
	<header>
		<h2>History</h2>
		<button class="ghost" onclick={() => readHistory(depth).then((h) => (entries = h))}>Refresh</button>
	</header>

	{#if entries.length === 0}
		<p class="empty">
			Nothing changed yet. Every edit turns up here with who made it — the console's
			own doing, like a fade running, does not, because nobody did it.
		</p>
	{:else}
		<ul>
			{#each entries as entry (entry.id)}
				{@const mine = !!entry.user_id && entry.user_id === $userId}
				<li class:mine class:reversal={!!entry.undoes}>
					<span class="who" style:background={colourOf($users, entry.user_id)}></span>
					<span class="what">
						{#if entry.undoes}
							<!-- An undo is a change like any other and shows as itself, which
							     is what makes the list a true account rather than a tidy one. -->
							<span class="tag">undo</span>
						{/if}
						{describeChange(entry, names)}
					</span>
					<span class="by">{nameOf(entry.user_id)}</span>
					<span class="when">{ago(entry.at, now)}</span>
				</li>
			{/each}
		</ul>
		{#if atTheEnd}
			<!-- Where Ctrl-Z stops, which is what `history_depth` promises. Said out
			     loud because the changes past here are deleted rather than merely
			     unlisted, and a list that simply ended would read as a bug. -->
			<p class="boundary">
				This is as far back as the show keeps — {depth} changes. Anything older has
				been let go.
			</p>
		{/if}
		<button class="ghost take-back" onclick={undo}>Take back my last change</button>
	{/if}
</div>

<style>
	.boundary {
		color: var(--text-faint);
		font-size: var(--font-xs);
		text-align: center;
		margin: 2px 0 0;
	}

	.history {
		padding: 12px 14px;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}

	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 48ch;
		line-height: 1.5;
	}

	ul {
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 1px;
	}

	li {
		display: grid;
		grid-template-columns: 8px minmax(0, 1fr) auto auto;
		align-items: center;
		gap: 10px;
		padding: 6px 4px;
		font-size: var(--font-sm);
		border-radius: 3px;
	}
	li + li {
		border-top: 1px solid var(--line);
	}
	/* Yours reads brighter, because those are the ones you can do something about. */
	li.mine .what {
		color: var(--text);
	}
	li .what {
		color: var(--text-dim);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	li.reversal .what {
		font-style: italic;
	}

	.who {
		width: 8px;
		height: 8px;
		border-radius: 50%;
	}

	.tag {
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--live);
		margin-right: 4px;
	}

	.by,
	.when {
		color: var(--text-faint);
		font-size: var(--font-xs);
		white-space: nowrap;
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 4px 10px;
		cursor: pointer;
	}
	.ghost:hover {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.take-back {
		align-self: flex-start;
	}
</style>
