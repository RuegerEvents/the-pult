<script lang="ts">
	/**
	 * Edit ⇄ Done, in the tile chrome.
	 *
	 * In the chrome rather than inside the panel on purpose: it is the same control
	 * with the same meaning on every panel that has one, and a panel's own buttons
	 * are exactly what it must not be mistaken for.
	 *
	 * Amber while unlocked, because amber is already this console's word for "this is
	 * live and it is yours" — an active cue, a held value. An unlocked panel is the
	 * same kind of fact.
	 */

	import { editing } from '$lib/stores/editing.js';

	let { panel }: { panel: string } = $props();

	const unlocked = $derived(editing(panel));
</script>

<button
	class="chip edit"
	class:on={$unlocked}
	title={$unlocked ? 'Stop editing this panel' : 'Edit this panel'}
	aria-pressed={$unlocked}
	onclick={() => unlocked.set(!$unlocked)}
>
	{$unlocked ? 'Done' : 'Edit'}
</button>

<style>
	.edit {
		font-size: var(--font-xs);
		padding: 0 8px;
		min-width: 44px;
	}

	.edit.on {
		border-color: var(--live);
		color: var(--live);
	}
</style>
