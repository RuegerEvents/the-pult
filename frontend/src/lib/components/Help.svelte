<script lang="ts">
	/**
	 * A circled question mark that says what something means, when asked.
	 *
	 * Panels used to carry their explanations as paragraphs under every table, read
	 * once and then scrolled past for the rest of the show. This keeps them one tap
	 * away instead: a tap rather than a hover, because a tablet has no hover, and the
	 * text closes on Escape or a tap anywhere else.
	 */

	import type { Snippet } from 'svelte';
	import CircleHelp from '@lucide/svelte/icons/circle-question-mark';

	let { label = 'What this means', children }: { label?: string; children: Snippet } = $props();

	let open = $state(false);
	let root: HTMLSpanElement | undefined = $state();

	function away(event: PointerEvent) {
		if (open && root && !root.contains(event.target as Node)) open = false;
	}
	function key(event: KeyboardEvent) {
		if (open && event.key === 'Escape') {
			event.stopPropagation();
			open = false;
		}
	}
</script>

<svelte:window onpointerdown={away} onkeydown={key} />

<span class="help" bind:this={root}>
	<button
		class="hit"
		type="button"
		aria-label={label}
		aria-expanded={open}
		onclick={() => (open = !open)}
	>
		<CircleHelp size={14} />
	</button>
	{#if open}
		<span class="text" role="note">{@render children()}</span>
	{/if}
</span>

<style>
	.help {
		position: relative;
		display: inline-flex;
		vertical-align: middle;
	}
	button {
		display: inline-flex;
		padding: 0;
		border: none;
		background: none;
		color: var(--text-dim, #777);
		cursor: pointer;
	}
	button:hover,
	button[aria-expanded='true'] {
		color: var(--text, #ddd);
	}
	.text {
		position: absolute;
		top: calc(100% + 6px);
		left: 0;
		z-index: 20;
		width: max-content;
		max-width: 340px;
		padding: 10px 12px;
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 6px);
		background: var(--bg-raised, #222);
		color: var(--text, #ddd);
		font-size: 12px;
		font-weight: 400;
		font-style: normal;
		line-height: 1.5;
		text-transform: none;
		letter-spacing: 0;
		box-shadow: 0 6px 20px rgb(0 0 0 / 0.4);
	}
	.text :global(p + p) {
		margin-top: 6px;
	}
</style>
