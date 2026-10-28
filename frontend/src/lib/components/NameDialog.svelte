<script lang="ts">
	/**
	 * Name something new: a sequence, a timeline, a surface, a master.
	 *
	 * A dialog rather than a field that opens inside the panel, because a field that
	 * appears pushes the panel's own content down under the pointer, and every panel
	 * that made something had grown its own slightly different one. Enter or PLEASE
	 * makes it, Escape or ESC does not.
	 */

	import type { Snippet } from 'svelte';
	import { focusOnMount, selectOnMount } from '$lib/actions.js';
	import Dialog from './Dialog.svelte';

	let {
		title,
		placeholder = 'Name…',
		initial = '',
		confirm = 'Create',
		hint,
		children,
		onsubmit,
		onclose
	}: {
		title: string;
		placeholder?: string;
		/** Prefilled and selected, so typing replaces it and Enter keeps it. */
		initial?: string;
		confirm?: string;
		/** One line under the field, where a name needs explaining. */
		hint?: string;
		/** Anything else a new one needs decided, under its name. */
		children?: Snippet;
		onsubmit: (name: string) => unknown | Promise<unknown>;
		onclose: () => void;
	} = $props();

	// The prefilled name, once: the field is the operator's from here on.
	// svelte-ignore state_referenced_locally
	let name = $state(initial);
	let busy = $state(false);

	async function submit() {
		const trimmed = name.trim();
		if (!trimmed || busy) return;
		busy = true;
		try {
			await onsubmit(trimmed);
			onclose();
		} finally {
			busy = false;
		}
	}
</script>

<Dialog {title} {onclose} onaccept={submit}>
	<form
		class="name"
		onsubmit={(e) => {
			e.preventDefault();
			submit();
		}}
	>
		<input class="input" {placeholder} bind:value={name} use:focusOnMount use:selectOnMount />
		{#if children}{@render children()}{/if}
		{#if hint}<p class="hint">{hint}</p>{/if}
	</form>
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		<button class="btn btn-primary" disabled={!name.trim() || busy} onclick={submit}>{confirm}</button>
	{/snippet}
</Dialog>

<style>
	.name {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.name .input {
		width: 100%;
	}
	.hint {
		margin: 0;
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
</style>
