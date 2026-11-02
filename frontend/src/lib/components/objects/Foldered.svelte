<script lang="ts" generics="Row extends { id: string; folder?: string | null }">
	/**
	 * A pool, in its folders: collapsible groups of whatever the pool draws its objects
	 * as, then the objects in no folder.
	 *
	 * The panel keeps drawing each object its own way — a card, an executor, a row — and
	 * hands that over as `item`; this decides only where each one goes. A folder's header
	 * is an object of its own: a tap opens or closes it (remembered on this screen), a
	 * right-click or a long-press opens the one context menu with Rename, New folder
	 * inside, Move to folder… and Delete. Dropping an object of this pool's kind
	 * anywhere on a folder — its header or what is inside it — files it there, and on
	 * the pool outside every folder takes it out — one write each, so one Ctrl-Z. A
	 * header can be dragged onto another to nest it. While something is dragged, a
	 * dashed outline goes round wherever it would land, the innermost folder under the
	 * pointer or the whole pool. The Show lock hides all of the editing and leaves the
	 * opening and closing.
	 *
	 * Only the header used to take a drop. Let go over the objects inside an open
	 * folder, and the drop fell through to the pool and took the object *out* of every
	 * folder — the opposite of what was meant, and nothing on screen said so.
	 */

	import type { Snippet } from 'svelte';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';

	import { longPress } from '$lib/actions.js';
	import { buildTree, canMoveFolder, countIn, type FolderNode } from '$lib/folders.js';
	import { OBJECT_MIME, openMenu, readDropped, renaming, type ObjectRef } from '$lib/objects.js';
	import { closedFolders, fileObject, folders, moveFolder, newFolder, renameFolder, toggleFolder } from '$lib/stores/folders.js';
	import { canEdit } from '$lib/stores/showLock.js';

	let {
		kind,
		rows,
		item,
		layout = 'wrap',
		min = 124
	}: {
		/** The pool's kind, as the registry names it. */
		kind: string;
		rows: Row[];
		item: Snippet<[Row]>;
		/** How each group lays its objects out, to match the panel's own. */
		layout?: 'wrap' | 'grid' | 'column';
		/** The narrowest an object gets, for `grid`. */
		min?: number;
	} = $props();

	const tree = $derived(buildTree($folders, kind, rows));

	/** Where a drop would land: a folder's id, `ROOT` for the pool, or nowhere. */
	const ROOT = '';
	let over = $state<string | null>(null);

	/** Leaving an element for one of its own children is not leaving it. */
	const left = (event: DragEvent) =>
		!(event.relatedTarget instanceof Node && (event.currentTarget as HTMLElement).contains(event.relatedTarget));

	const accepts = (event: DragEvent) => !!event.dataTransfer?.types.includes(OBJECT_MIME) && $canEdit;

	async function dropInto(event: DragEvent, folder: string | null) {
		const ref = readDropped(event);
		over = null;
		// The innermost folder under the pointer takes it; the ones round it do not.
		event.stopPropagation();
		if (!ref || !$canEdit) return;
		if (ref.kind === kind) {
			event.preventDefault();
			event.stopPropagation();
			await fileObject(ref, folder);
		} else if (ref.kind === 'folder' && ref.id !== folder && canMoveFolder($folders, ref.id, folder)) {
			event.preventDefault();
			event.stopPropagation();
			await moveFolder(ref.id, folder);
		}
	}

	function dragFolder(event: DragEvent, id: string) {
		if (!$canEdit || !event.dataTransfer) return event.preventDefault();
		event.dataTransfer.setData(OBJECT_MIME, JSON.stringify({ kind: 'folder', id } satisfies ObjectRef));
		event.dataTransfer.effectAllowed = 'move';
	}

	async function addFolder() {
		const id = await newFolder(kind, null);
		renaming.set({ kind: 'folder', id });
	}

	function finishRename(id: string, name: string) {
		renaming.set(null);
		if (name.trim()) void renameFolder(id, name.trim());
	}

	const isRenaming = (id: string) => $renaming?.kind === 'folder' && $renaming.id === id;

	function focusOnMount(node: HTMLInputElement) {
		node.focus();
		node.select();
	}
</script>

{#snippet items(list: Row[])}
	{#if list.length > 0}
		<div class="items {layout}" style:--min="{min}px">
			{#each list as row (row.id)}{@render item(row)}{/each}
		</div>
	{/if}
{/snippet}

{#snippet folderNode(node: FolderNode<Row>)}
	{@const id = node.folder.id}
	{@const open = !$closedFolders.has(id)}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<section
		class="folder"
		class:open
		class:over={over === id}
		ondragover={(e) => {
			if (accepts(e)) {
				e.preventDefault();
				e.stopPropagation();
				over = id;
			}
		}}
		ondragleave={(e) => left(e) && over === id && (over = null)}
		ondrop={(e) => dropInto(e, id)}
	>
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="header"
			draggable={$canEdit && !isRenaming(id)}
			ondragstart={(e) => dragFolder(e, id)}
			oncontextmenu={(e) => {
				e.preventDefault();
				openMenu({ kind: 'folder', id }, e.clientX, e.clientY);
			}}
			use:longPress={(x, y) => openMenu({ kind: 'folder', id }, x, y)}
		>
			<button class="toggle" aria-expanded={open} onclick={() => toggleFolder(id)}>
				<span class="chevron"><ChevronRight size={13} /></span>
				<FolderIcon size={13} />
				{#if !isRenaming(id)}<span class="name">{node.folder.name}</span>{/if}
			</button>
			{#if isRenaming(id)}
				<input
					class="rename"
					value={node.folder.name}
					use:focusOnMount
					onkeydown={(e) => {
						if (e.key === 'Enter') finishRename(id, e.currentTarget.value);
						if (e.key === 'Escape') renaming.set(null);
						e.stopPropagation();
					}}
					onblur={(e) => finishRename(id, e.currentTarget.value)}
				/>
			{/if}
			<span class="count">{countIn(node)}</span>
		</div>
		{#if open}
			<div class="inside">
				{#each node.folders as child (child.folder.id)}{@render folderNode(child)}{/each}
				{@render items(node.rows)}
			</div>
		{/if}
	</section>
{/snippet}

<svelte:window ondragend={() => (over = null)} ondrop={() => (over = null)} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="pool"
	class:over={over === ROOT}
	ondragover={(e) => {
		if (accepts(e) && tree.folders.length > 0) {
			e.preventDefault();
			over = ROOT;
		}
	}}
	ondragleave={(e) => left(e) && over === ROOT && (over = null)}
	ondrop={(e) => dropInto(e, null)}
>
	{#each tree.folders as node (node.folder.id)}{@render folderNode(node)}{/each}
	{@render items(tree.rows)}
	{#if $canEdit}
		<button class="new" title="A folder in this pool" onclick={addFolder}><FolderPlus size={12} /> Folder</button>
	{/if}
</div>

<style>
	.pool {
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-width: 0;
	}
	.items.wrap {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.items.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(var(--min), 1fr));
		gap: 6px;
	}
	.items.column {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.folder {
		border-left: 2px solid var(--line-strong, #333);
		padding-left: 6px;
	}
	.header {
		display: flex;
		align-items: center;
		gap: 6px;
		min-height: 28px;
		border-radius: 4px;
		user-select: none;
		touch-action: manipulation;
	}
	/* Where a drop lands: the whole folder, its contents included, or the whole pool. */
	.folder.over,
	.pool.over {
		background: var(--bg-hover, #2a2a2a);
		outline: 1px dashed var(--text-dim, #888);
		outline-offset: 2px;
		border-radius: 4px;
	}
	.toggle {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		background: none;
		border: none;
		color: var(--text, #ddd);
		font: inherit;
		font-size: var(--font-sm, 12px);
		font-weight: 600;
		padding: 2px 4px;
		cursor: pointer;
		min-height: 28px;
	}
	.chevron {
		display: inline-flex;
		transition: transform 0.12s;
	}
	.folder.open > .header .chevron {
		transform: rotate(90deg);
	}
	.count {
		color: var(--text-faint, #666);
		font-size: var(--font-xs, 11px);
		font-variant-numeric: tabular-nums;
	}
	.rename {
		background: var(--bg-input, #111);
		border: 1px solid var(--line-strong, #444);
		color: var(--text, #ddd);
		font: inherit;
		font-size: var(--font-sm, 12px);
		padding: 2px 6px;
		border-radius: 3px;
		min-width: 0;
	}
	.inside {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 2px 0 4px;
	}
	.new {
		align-self: flex-start;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: none;
		border: 1px dashed var(--line-strong, #3a3a3a);
		border-radius: 4px;
		color: var(--text-faint, #666);
		font: inherit;
		font-size: var(--font-xs, 11px);
		padding: 2px 8px;
		cursor: pointer;
	}
	.new:hover {
		color: var(--text, #ddd);
	}
</style>
