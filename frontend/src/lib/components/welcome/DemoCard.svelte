<script lang="ts">
	/**
	 * One of the shows the console can make for itself.
	 *
	 * The list comes from the station rather than from a table in here, so a card
	 * cannot offer a demo this build does not have.
	 */

	import type { DemoCardInfo } from '$lib/shows.js';

	let {
		demo,
		busy,
		onopen
	}: { demo: DemoCardInfo; busy: boolean; onopen: (id: string) => void } = $props();
</script>

<button class="card" disabled={busy} onclick={() => onopen(demo.id)}>
	<span class="name">{demo.title}</span>
	<span class="blurb">{demo.blurb}</span>
</button>

<style>
	.card {
		display: flex;
		flex-direction: column;
		gap: 5px;
		align-items: flex-start;
		text-align: left;
		width: 100%;
		height: 100%;
		padding: 14px;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius);
		color: var(--text);
		cursor: pointer;
	}
	.card:hover:not(:disabled) {
		border-color: var(--accent);
		background: var(--bg-hover);
	}
	.card:disabled {
		cursor: progress;
		opacity: 0.6;
	}

	.name {
		font-size: 0.95rem;
		font-weight: 600;
		color: var(--text-bright);
	}
	.blurb {
		font-size: var(--font-sm);
		line-height: 1.45;
		color: var(--text-dim);
	}
</style>
