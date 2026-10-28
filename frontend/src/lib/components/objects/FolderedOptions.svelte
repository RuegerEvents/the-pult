<script lang="ts" generics="Row extends { id: string; name: string; folder?: string | null }">
	/**
	 * The `<option>`s of a pool, grouped by folder: what goes inside a `<select>` that
	 * picks a sequence, a timeline, anything filed. A picker is too narrow for the
	 * pool's tree, so a folder is an `<optgroup>` labeled with its path, and what is in
	 * no folder comes first, ungrouped. `byFolder` is the rule; this only draws it.
	 */
	import { byFolder } from '$lib/folders.js';
	import { folders } from '$lib/stores/folders.js';

	let { rows, label = (row: Row) => row.name }: { rows: Row[]; label?: (row: Row) => string } = $props();

	const groups = $derived(byFolder(rows, $folders));
</script>

{#each groups as group (group.path)}
	{#if group.path}
		<optgroup label={group.path}>
			{#each group.rows as row (row.id)}<option value={row.id}>{label(row)}</option>{/each}
		</optgroup>
	{:else}
		{#each group.rows as row (row.id)}<option value={row.id}>{label(row)}</option>{/each}
	{/if}
{/each}
