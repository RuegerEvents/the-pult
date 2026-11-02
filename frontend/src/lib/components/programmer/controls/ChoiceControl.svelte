<script lang="ts">
	/**
	 * One of a fixed list of words: a strip's effect, which the node names and nothing
	 * else can spell. A value the list does not have stays shown, so a cue stored
	 * against a newer node's effect is not silently turned into the first one here.
	 */
	let {
		choices,
		value,
		oninput
	}: { choices: string[]; value: string; oninput: (value: string) => void } = $props();

	const listed = $derived(choices.includes(value) || value === '' ? choices : [value, ...choices]);
</script>

<select class="choice" {value} onchange={(e) => oninput(e.currentTarget.value)}>
	{#each listed as choice (choice)}
		<option value={choice}>{choice}</option>
	{/each}
</select>

<style>
	.choice {
		width: 100%;
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 3px 6px;
	}
</style>
