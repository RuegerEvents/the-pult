<script lang="ts">
	/**
	 * A modal, once.
	 *
	 * There were three hand-rolled copies of this — the delete prompt, the store menu
	 * and the restore confirmation — and they had already begun to disagree: one
	 * answered Escape and two did not, one closed on a backdrop click and two did,
	 * and each had its own scrim colour. What a dialog *is* is not a thing three
	 * components should each have an opinion about.
	 *
	 * `fixed` rather than `absolute`, because a modal belongs to the window and not to
	 * whichever tile happened to open it. The Escape handler is on the window for the
	 * same reason: the panel behind may still hold the focus, and an operator pressing
	 * Escape means the thing covering the screen.
	 */

	import type { Snippet } from 'svelte';

	let {
		title,
		size = 'small',
		onclose,
		children,
		actions,
		footer
	}: {
		/** Named for the assistive tree, and printed in the header unless `bare`. */
		title: string;
		/** How much of the window it wants. `full` is the setup mode. */
		size?: 'small' | 'wide' | 'full';
		onclose: () => void;
		children: Snippet;
		/**
		 * Controls that belong to the dialog's own chrome rather than to its content —
		 * an Edit toggle, say. Beside the title, where a tile puts the same thing.
		 */
		actions?: Snippet;
		/** The row of answers, if there is one. */
		footer?: Snippet;
	} = $props();

	function onkey(event: KeyboardEvent) {
		if (event.key !== 'Escape') return;
		event.stopPropagation();
		onclose();
	}
</script>

<svelte:window onkeydown={onkey} />

<!-- The backdrop closes, and the dialog does not: `currentTarget` rather than a
     `stopPropagation` on the panel, so the panel needs no handler of its own and
     nothing inside it has to remember to stop bubbling. -->
<div
	class="scrim"
	role="presentation"
	onclick={(e) => e.target === e.currentTarget && onclose()}
>
	<div class="dialog {size}" role="dialog" aria-modal="true" aria-label={title} tabindex="-1">
		<header>
			<h2>{title}</h2>
			<span class="spacer"></span>
			{#if actions}{@render actions()}{/if}
			<button class="icon" aria-label="Close" onclick={onclose}>✕</button>
		</header>
		<div class="body">
			{@render children()}
		</div>
		{#if footer}
			<footer>{@render footer()}</footer>
		{/if}
	</div>
</div>

<style>
	.scrim {
		position: fixed;
		inset: 0;
		z-index: 40;
		display: grid;
		place-items: center;
		background: rgb(0 0 0 / 55%);
		padding: 20px;
	}

	.dialog {
		display: flex;
		flex-direction: column;
		max-height: 100%;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 12px 40px rgb(0 0 0 / 60%);
		overflow: hidden;
	}
	.small {
		width: min(560px, 100%);
	}
	.wide {
		width: min(860px, 100%);
	}
	.full {
		width: 100%;
		height: 100%;
	}

	header {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--line);
		flex-shrink: 0;
	}
	h2 {
		font-size: var(--font-sm);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.spacer {
		flex: 1;
	}
	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		cursor: pointer;
		padding: 2px 4px;
	}
	.icon:hover {
		color: var(--bad);
	}

	.body {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
		overflow: auto;
	}

	footer {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--line);
		flex-shrink: 0;
	}
</style>
