<script lang="ts">
	/**
	 * Which show is open, and what to do with it.
	 *
	 * In the top bar beside the layout menu, because a show is the thing this whole
	 * window is about: an operator should not have to find a panel to save one.
	 *
	 * Everything here that changes *which* show is open goes through `show.*` — a
	 * station RPC, not a command, because it is nobody's to undo and must not be told
	 * to a peer. Each answers and then the station stops; this page's socket closes,
	 * reconnects onto the new console, and reloads.
	 */

	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { show as openShow } from '$lib/stores/show.js';
	import { revealPanel } from '$lib/stores/layout.js';
	import { addToast } from '$lib/toasts.js';
	import { backendOrigin, type OpenShow } from '$lib/ws/endpoint.js';
	import { focusOnMount, selectOnMount } from '$lib/actions.js';

	let { show }: { show: OpenShow } = $props();

	const client = getClientContext();
	const data = getDataContext();

	let open = $state(false);
	let naming = $state<'version' | 'copy' | null>(null);
	let draft = $state('');
	let busy = $state(false);

	// The show's own name where it has one — the row is what an operator renamed —
	// falling back to what the folder is called, which is all `/api/config` knows.
	const name = $derived($openShow?.name ?? show.name);

	function start(what: 'version' | 'copy') {
		draft = what === 'copy' ? `${name} copy` : '';
		naming = what;
		open = false;
	}

	async function commit() {
		const typed = draft.trim();
		const what = naming;
		naming = null;
		if (what === 'version') return save(typed || undefined);
		if (what === 'copy' && typed) return ask('show.saveAs', { name: typed });
	}

	/** A version, named or not. Not a switch: the console goes on running. */
	async function save(withName?: string) {
		busy = true;
		try {
			await data.versions.checkpoint(withName ? { name: withName } : {});
			addToast(withName ? `Saved “${withName}”` : 'Version saved', 'success');
		} catch (e) {
			addToast(`Could not save a version: ${e}`);
		} finally {
			busy = false;
		}
	}

	/** A switch. Answers, and then the station stops — so nothing here waits after it. */
	async function ask(method: string, args: Record<string, unknown> = {}) {
		open = false;
		if (busy) return;
		busy = true;
		try {
			await client.call(method, args);
		} catch (e) {
			addToast(`${e}`);
			busy = false;
		}
	}

	/**
	 * Export. A plain link rather than a fetch: the browser's own download is what
	 * turns a response into a file on somebody's disk, and it carries the name the
	 * station puts in the Content-Disposition.
	 */
	function exportUrl(versions: boolean): string {
		const url = new URL('/api/shows/export', backendOrigin(window.location));
		if (versions) url.searchParams.set('versions', '1');
		return url.toString();
	}
</script>

<div class="bar">
	{#if naming}
		<form
			class="naming"
			onsubmit={(e) => {
				e.preventDefault();
				commit();
			}}
		>
			<input
				bind:value={draft}
				placeholder={naming === 'copy' ? 'Copy name…' : 'Version name…'}
				use:focusOnMount
				use:selectOnMount
				onkeydown={(e) => e.key === 'Escape' && (naming = null)}
			/>
			<button class="chip" type="submit">{naming === 'copy' ? 'Save as' : 'Save'}</button>
			<button class="chip" type="button" onclick={() => (naming = null)}>Cancel</button>
		</form>
	{:else}
		<button class="name" onclick={() => (open = !open)} title={show.path}>
			{name}<span class="caret">▾</span>
		</button>
		<button class="chip" disabled={busy} onclick={() => save()} title="⌘S">Save</button>
	{/if}

	{#if open}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="menu" onpointerleave={() => (open = false)}>
			<span class="heading">This show</span>
			<button onclick={() => start('version')}>Save version…</button>
			<button
				onclick={() => {
					open = false;
					revealPanel('show');
				}}>Versions…</button
			>
			<button onclick={() => start('copy')}>Save as…</button>
			<a class="row" href={exportUrl(false)} download onclick={() => (open = false)}>
				Export .pultz
			</a>
			<a class="row dim" href={exportUrl(true)} download onclick={() => (open = false)}>
				Export with versions
			</a>
			<span class="heading">Another show</span>
			<button onclick={() => ask('show.close')}>Close — back to the start</button>
			<span class="path">{show.path}</span>
		</div>
	{/if}
</div>

<style>
	.bar {
		position: relative;
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.name {
		background: none;
		border: none;
		color: var(--text, #e0e0e0);
		font-size: var(--font-sm, 12px);
		font-weight: 600;
		cursor: pointer;
		padding: 4px 2px;
		max-width: 220px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.caret {
		margin-left: 5px;
		color: var(--text-faint, #555);
	}

	.chip {
		background: var(--bg-raised, #252525);
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 4px);
		color: var(--text-dim, #888);
		font-size: var(--font-xs, 11px);
		padding: 3px 8px;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		border-color: var(--accent, #4a9eff);
		color: var(--text, #e0e0e0);
	}
	.chip:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.menu {
		position: absolute;
		top: 100%;
		left: 0;
		z-index: 20;
		min-width: 230px;
		display: flex;
		flex-direction: column;
		padding: 5px;
		background: var(--bg-panel, #222);
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 4px);
		box-shadow: 0 8px 24px rgb(0 0 0 / 45%);
	}
	.menu button,
	.menu .row {
		background: none;
		border: none;
		border-radius: 3px;
		color: var(--text, #e0e0e0);
		font-size: var(--font-sm, 12px);
		text-align: left;
		text-decoration: none;
		padding: 7px 8px;
		cursor: pointer;
	}
	.menu button:hover,
	.menu .row:hover {
		background: var(--bg-hover, #2a2a2a);
	}
	.menu .row.dim {
		color: var(--text-dim, #888);
	}

	.heading {
		font-size: var(--font-xs, 11px);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint, #555);
		padding: 8px 8px 4px;
	}

	.path {
		font-family: monospace;
		font-size: 10px;
		color: var(--text-faint, #555);
		padding: 6px 8px 3px;
		border-top: 1px solid var(--line, #2a2a2a);
		margin-top: 4px;
		overflow: hidden;
		text-overflow: ellipsis;
		direction: rtl;
	}

	.naming {
		display: flex;
		gap: 5px;
		align-items: center;
	}
	.naming input {
		background: var(--bg-sunken, #141414);
		border: 1px solid var(--line-input, #555);
		border-radius: var(--radius, 4px);
		color: var(--text, #e0e0e0);
		font-size: var(--font-sm, 12px);
		padding: 3px 7px;
		width: 170px;
	}
</style>
