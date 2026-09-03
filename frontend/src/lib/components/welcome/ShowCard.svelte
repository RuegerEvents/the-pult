<script lang="ts">
	/**
	 * One show, as something to press.
	 *
	 * A show that is not where it was is still a card — greyed, and saying so. It is
	 * exactly the row somebody is looking for when the stick is in the other bag, and
	 * a list that hid it would be one nobody could rely on.
	 */

	import { asSize, describe, parentPath, type ShowSummary } from '$lib/shows.js';

	let {
		show,
		onopen
	}: { show: ShowSummary; onopen: (path: string) => void } = $props();

	const where = $derived(parentPath(show.path));
	const when = $derived.by(() => {
		if (!show.lastOpened) return '';
		const at = new Date(show.lastOpened);
		return Number.isNaN(at.getTime()) ? '' : at.toLocaleString();
	});
</script>

<button
	class="card"
	class:gone={show.missing}
	disabled={show.missing}
	onclick={() => onopen(show.path)}
	title={show.path}
>
	<span class="name">{show.name}</span>
	<span class="what" class:warn={show.missing || show.madeByAnotherBuild || show.problem}>
		{describe(show)}
	</span>
	<span class="where">{where}</span>
	<span class="foot">
		{#if when}<span>{when}</span>{/if}
		{#if show.bytes}<span>{asSize(show.bytes)}</span>{/if}
	</span>
</button>

<style>
	.card {
		display: flex;
		flex-direction: column;
		gap: 3px;
		align-items: flex-start;
		text-align: left;
		width: 100%;
		padding: 12px 14px;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius);
		color: var(--text);
		cursor: pointer;
		min-height: var(--hit, 44px);
	}
	.card:hover:not(:disabled) {
		border-color: var(--accent);
		background: var(--bg-hover);
	}
	.card.gone {
		cursor: default;
		opacity: 0.5;
	}

	.name {
		font-size: 0.95rem;
		font-weight: 600;
		color: var(--text-bright);
	}
	.what {
		font-size: var(--font-sm);
		color: var(--text-dim);
	}
	.what.warn {
		color: var(--live);
	}
	.where {
		font-family: monospace;
		font-size: var(--font-xs);
		color: var(--text-faint);
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		direction: rtl;
	}
	.foot {
		display: flex;
		gap: 10px;
		font-size: var(--font-xs);
		color: var(--text-faint);
	}
</style>
