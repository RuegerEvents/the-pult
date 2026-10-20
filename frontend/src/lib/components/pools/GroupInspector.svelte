<script lang="ts">
	/** What a saved group is: its question, in words, and what it answers right now. */

	import type { Group } from '$lib/generated/index.js';
	import { describe } from '$lib/selection.js';
	import { recall } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';

	let { rows }: { rows: Group[] } = $props();

	const types = collection('fixture_types');
	const objects = collection('scene_objects');
</script>

{#each rows as group (group.id)}
	<div class="detail">
		<p class="query"><span class="label">Selects</span> {describe(group.query, $types, $objects)}</p>
		<button class="btn" onclick={() => recall(group.query)}>Recall {rows.length > 1 ? group.name : ''}</button>
	</div>
{/each}

<style>
	.detail {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 8px;
	}
	.query {
		font-size: var(--font-sm);
		color: var(--text);
	}
	.label {
		color: var(--text-dim);
		margin-right: 4px;
	}
</style>
