<script lang="ts">
	/**
	 * A line of text, for a display module.
	 *
	 * Committed on Enter or on leaving the field rather than on every keystroke: a
	 * value here is a replicated write and a row in the oplog, and nobody needs to
	 * watch a sign spell out a word one letter at a time on the far side of a room.
	 */

	let { value, oninput }: { value: string; oninput: (value: string) => void } = $props();

	let draft = $state<string | null>(null);

	function commit() {
		if (draft !== null && draft !== value) oninput(draft);
		draft = null;
	}
</script>

<input
	class="text"
	value={draft ?? value}
	oninput={(e) => (draft = e.currentTarget.value)}
	onblur={commit}
	onkeydown={(e) => {
		if (e.key === 'Enter') commit();
		if (e.key === 'Escape') draft = null;
	}}
/>

<style>
	.text {
		width: 100%;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 3px 6px;
	}
	.text:focus {
		outline: none;
		border-color: var(--accent);
	}
</style>
