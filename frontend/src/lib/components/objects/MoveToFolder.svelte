<script lang="ts">
	/**
	 * Move to folder…, for any object a pool can file and for a folder itself.
	 *
	 * Mounted once at the root, beside the context menu that opens it, so it is there
	 * whichever panel the object was in. The folders offered are the ones of the
	 * object's own kind; a folder is not offered a place inside itself. Picking one is
	 * one write. "New folder" makes one at the top of the pool and files the object in
	 * it, as one gesture.
	 */

	import Dialog from '$lib/components/Dialog.svelte';
	import { canMoveFolder, flatten } from '$lib/folders.js';
	import { kindOf, moving, rowOf } from '$lib/objects.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { fileObject, folders, moveFolder, newFolder } from '$lib/stores/folders.js';
	import { addToast } from '$lib/toasts.js';

	const ref = $derived($moving);
	const row = $derived(ref ? rowOf(ref) : undefined);
	/** The pool's kind: the object's, or for a folder the kind it holds. */
	const pool = $derived(
		ref?.kind === 'folder' ? ($folders.find((f) => f.id === ref.id)?.kind ?? null) : (ref?.kind ?? null)
	);
	const choices = $derived(
		pool
			? flatten($folders, pool).filter((c) => ref?.kind !== 'folder' || canMoveFolder($folders, ref.id, c.folder.id))
			: []
	);
	const current = $derived(
		ref?.kind === 'folder' ? ($folders.find((f) => f.id === ref.id)?.parent ?? null) : (row?.folder ?? null)
	);

	let newName = $state('');

	async function put(folder: string | null) {
		const r = ref;
		moving.set(null);
		if (!r) return;
		try {
			if (r.kind === 'folder') await moveFolder(r.id, folder);
			else await fileObject(r, folder);
		} catch (e) {
			addToast(`Move: ${e instanceof Error ? e.message : e}`);
		}
	}

	async function putInNew() {
		const name = newName.trim();
		if (!name || !pool) return;
		newName = '';
		beginGesture();
		try {
			const id = await newFolder(pool, null, name);
			await put(id);
		} finally {
			endGesture();
		}
	}
</script>

{#if ref && row && pool}
	<Dialog title="Move to folder" onclose={() => moving.set(null)}>
		<div class="move">
			<p class="what">{kindOf(ref.kind)?.label ?? ''} · {row.name}</p>
			<ul>
				<li>
					<button class:here={current === null} onclick={() => put(null)}>Top level</button>
				</li>
				{#each choices as choice (choice.folder.id)}
					<li style:padding-left="{choice.depth * 14}px">
						<button class:here={current === choice.folder.id} onclick={() => put(choice.folder.id)}>
							{choice.folder.name}
						</button>
					</li>
				{/each}
			</ul>
			<form
				onsubmit={(e) => {
					e.preventDefault();
					void putInNew();
				}}
			>
				<input class="input" placeholder="New folder" bind:value={newName} />
				<button class="btn btn-ghost" type="submit" disabled={!newName.trim()}>Make and move</button>
			</form>
		</div>
	</Dialog>
{/if}

<style>
	.move {
		padding: 12px 14px;
		display: flex;
		flex-direction: column;
		gap: 10px;
		min-width: 260px;
	}
	.what {
		margin: 0;
		color: var(--text-dim, #888);
		font-size: 12px;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 50vh;
		overflow: auto;
	}
	li button {
		width: 100%;
		text-align: left;
		background: none;
		border: none;
		border-radius: 4px;
		color: var(--text, #ddd);
		font: inherit;
		font-size: 13px;
		padding: 7px 8px;
		cursor: pointer;
	}
	li button:hover {
		background: var(--bg-hover, #2a2a2a);
	}
	li button.here {
		color: var(--text-bright, #fff);
		font-weight: 600;
	}
	li button.here::after {
		content: ' · here';
		color: var(--text-faint, #666);
		font-weight: 400;
	}
	form {
		display: flex;
		gap: 6px;
	}
	form .input {
		flex: 1;
	}
</style>
