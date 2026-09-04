<script lang="ts">
	/**
	 * The show: what it is called, where it lives, and every point somebody saved.
	 *
	 * The two halves are not the same kind of thing, and the panel says so. A
	 * version's *row* replicates — every station in the session knows it exists, who
	 * took it and when. Its *snapshot* is each station's own copy of its own
	 * showfile, so a station that joined after a version was taken has the row and no
	 * file, and can only say "not on this station". That is what the LOCAL
	 * `versions_here` is for, and why a row that cannot be restored still appears.
	 */

	import { onMount } from 'svelte';

	import { focusOnMount, selectOnMount } from '$lib/actions.js';
	import { getClientContext, getDataContext, getStationContext } from '$lib/ws/context.js';
	import { collection, show as openShow } from '$lib/stores/show.js';
	import { users } from '$lib/stores/user.js';
	import { addToast } from '$lib/toasts.js';
	import { asSize, parentPath } from '$lib/shows.js';
	import { beginSwitch, endSwitch } from '$lib/stores/switching.js';
	import { readPreferences } from '$lib/preferences.js';
	import type { Version } from '$lib/generated/index.js';

	const client = getClientContext();
	const data = getDataContext();
	const station = getStationContext();
	const versions = collection('versions');

	let editingName = $state(false);
	let draftName = $state('');
	let saving = $state(false);
	let naming = $state(false);
	let versionName = $state('');
	/** The version an operator has asked to restore and not yet confirmed. */
	let confirming = $state<Version | null>(null);
	let restoreRefusal = $state('');
	/** Which snapshots this station actually holds. */
	let here = $state<string[]>([]);
	let autosave = $state<{ minutes: number; keep: number } | null>(null);

	const ordered = $derived(
		[...($versions as Version[])].sort((a, b) => b.created_at.localeCompare(a.created_at))
	);
	const bundle = $derived($station?.show ?? null);

	function whoTook(version: Version): string {
		if (!version.user_id) return version.automatic ? 'the console' : 'unattributed';
		return $users.find((user) => user.id === version.user_id)?.name ?? 'someone else';
	}

	function when(at: string): string {
		const stamp = new Date(at);
		return Number.isNaN(stamp.getTime()) ? at : stamp.toLocaleString();
	}

	/** What a version nobody named is shown as — the same rule the station uses. */
	function label(version: Version): string {
		return version.name?.trim() ? version.name : when(version.created_at);
	}

	async function saveName() {
		const show = $openShow;
		if (!show || !draftName.trim()) return;
		saving = true;
		await data.show.set({ ...show, name: draftName.trim() });
		editingName = false;
		saving = false;
	}

	async function takeAVersion(withName?: string) {
		try {
			await data.versions.checkpoint(withName ? { name: withName } : {});
		} catch (e) {
			addToast(`Could not save a version: ${e}`);
		}
	}

	async function restore(version: Version) {
		confirming = null;
		restoreRefusal = '';
		beginSwitch(version.name ? `restoring “${version.name}”` : 'restoring a version');
		try {
			await client.call('show.restore', { versionId: version.id });
		} catch (e) {
			endSwitch();
			// The refusal that matters is "leave the session first": a peer holds the
			// show as it is now and would replay it straight back over whatever this
			// station put there. Shown in place rather than as a toast, because it is
			// an answer to the button that was just pressed.
			restoreRefusal = `${e}`;
		}
	}

	async function forget(version: Version) {
		try {
			await data.versions.byId(version.id).delete();
		} catch (e) {
			addToast(`Could not remove that version: ${e}`);
		}
	}

	onMount(() => {
		void readPreferences().then((prefs) => {
			if (prefs) autosave = { minutes: prefs.autosaveMinutes ?? 0, keep: prefs.autosaveKeep ?? 0 };
		});
		const take = (value: unknown) => {
			here = Array.isArray(value) ? (value as string[]) : [];
		};
		const stop = client.subscribe('versions_here', take);
		const ask = () => void client.get(['versions_here']).then(take);
		ask();
		const stopConnect = client.addConnectListener(ask);
		return () => {
			stop();
			stopConnect();
		};
	});
</script>

<div class="panel">
	<div class="panel-header">
		<span class="panel-title">Show</span>
		<button class="chip" onclick={() => (naming = true)} disabled={!bundle}>Save version…</button>
	</div>

	{#if !$openShow}
		<p class="empty-hint">No show open.</p>
	{:else}
		<div class="field">
			<span class="label">Name</span>
			{#if editingName}
				<form
					class="inline-form"
					onsubmit={(e) => {
						e.preventDefault();
						saveName();
					}}
				>
					<input
						class="inline-input"
						bind:value={draftName}
						disabled={saving}
						use:focusOnMount
						use:selectOnMount
						onkeydown={(e) => {
							if (e.key === 'Escape') editingName = false;
						}}
					/>
					<button class="save-btn" type="submit" disabled={saving}>✓</button>
					<button class="cancel-btn" type="button" onclick={() => (editingName = false)}>✕</button>
				</form>
			{:else}
				<button
					class="name-value"
					onclick={() => {
						draftName = $openShow?.name ?? '';
						editingName = true;
					}}
					title="Click to edit"
				>
					{$openShow.name}
					<span class="edit-hint">✎</span>
				</button>
			{/if}
		</div>

		{#if bundle}
			<div class="field">
				<span class="label">Folder</span>
				<span class="mono dim" title={bundle.path}>{bundle.name}</span>
			</div>
			<div class="field">
				<span class="label">In</span>
				<span class="mono dim path" title={bundle.path}>{parentPath(bundle.path)}</span>
			</div>
		{/if}
		<div class="field">
			<span class="label">Show ID</span>
			<span class="mono dim">{$openShow.id.slice(0, 8)}…</span>
		</div>
	{/if}

	{#if naming}
		<form
			class="naming"
			onsubmit={(e) => {
				e.preventDefault();
				const typed = versionName.trim();
				naming = false;
				versionName = '';
				takeAVersion(typed || undefined);
			}}
		>
			<input
				bind:value={versionName}
				placeholder="Version name…"
				use:focusOnMount
				onkeydown={(e) => e.key === 'Escape' && (naming = false)}
			/>
			<button class="chip" type="submit">Save</button>
			<button class="chip" type="button" onclick={() => (naming = false)}>Cancel</button>
		</form>
	{/if}

	<div class="versions">
		<div class="versions-head">
			<span class="panel-title">Versions</span>
			{#if autosave}
				<span class="note">
					{#if autosave.minutes > 0}
						auto every {autosave.minutes} min, keeping {autosave.keep}
					{:else}
						autosave off
					{/if}
				</span>
			{/if}
		</div>

		{#if restoreRefusal}
			<p class="refusal">{restoreRefusal}</p>
		{/if}

		{#if !ordered.length}
			<p class="empty-hint">
				No versions yet. ⌘S takes one — it is a point to come back to, not a flush:
				everything is already on the disk.
			</p>
		{:else}
			<ul>
				{#each ordered as version (version.id)}
					{@const holds = here.includes(version.id)}
					<li class:auto={version.automatic}>
						<div class="what">
							<span class="version-name">{label(version)}</span>
							<span class="meta">
								{when(version.created_at)} · {whoTook(version)}
								{#if version.automatic}· automatic{/if}
							</span>
							{#if !holds}
								<span class="meta warn">not on this station</span>
							{/if}
						</div>
						<div class="acts">
							<button
								class="chip"
								disabled={!holds}
								title={holds
									? 'Put the show back as it was'
									: 'This station never held this version; open it on the one that took it'}
								onclick={() => {
									restoreRefusal = '';
									confirming = version;
								}}>Restore</button
							>
							<button class="chip" onclick={() => forget(version)} title="Delete this version"
								>✕</button
							>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

{#if confirming}
	<div class="scrim">
		<div class="dialog">
			<h3>Restore “{label(confirming)}”?</h3>
			<p>
				The show goes back to how it was at {when(confirming.created_at)}. A version of
				what it is now is taken first, so this can be undone by restoring that one.
			</p>
			<p class="fine">The console restarts, and this page reloads onto the restored show.</p>
			<div class="acts">
				<button class="chip" onclick={() => (confirming = null)}>Cancel</button>
				<button class="chip danger" onclick={() => confirming && restore(confirming)}
					>Restore</button
				>
			</div>
		</div>
	</div>
{/if}

<style>
	.panel {
		background: var(--bg-raised, #252525);
		border: 1px solid var(--line, #333);
		border-radius: 6px;
		padding: 12px 14px;
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-height: 0;
	}

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 6px;
	}

	.panel-title {
		font-size: 0.68rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #777;
	}

	.field {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 10px;
		padding: 5px 0;
		border-bottom: 1px solid #2e2e2e;
	}

	.label {
		font-size: 0.78rem;
		color: #888;
		flex-shrink: 0;
	}

	.name-value {
		background: none;
		border: none;
		color: #e0e0e0;
		font-size: 0.85rem;
		cursor: pointer;
		padding: 0;
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.name-value:hover .edit-hint {
		opacity: 1;
	}
	.edit-hint {
		opacity: 0;
		font-size: 0.7rem;
		color: #666;
		transition: opacity 0.15s;
	}

	.inline-form,
	.naming {
		display: flex;
		gap: 4px;
		align-items: center;
	}
	.inline-input,
	.naming input {
		background: #1a1a1a;
		border: 1px solid #555;
		border-radius: 3px;
		color: #e0e0e0;
		font-size: 0.85rem;
		padding: 2px 6px;
		width: 140px;
	}
	.naming {
		margin: 8px 0 2px;
	}
	.naming input {
		flex: 1;
		width: auto;
	}

	.save-btn,
	.cancel-btn,
	.chip {
		background: none;
		border: 1px solid #444;
		border-radius: 3px;
		color: #aaa;
		cursor: pointer;
		font-size: 0.72rem;
		padding: 2px 7px;
	}
	.chip:hover:not(:disabled) {
		border-color: #4a9eff;
		color: #e0e0e0;
	}
	.chip:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.chip.danger:hover {
		border-color: #ef4444;
		color: #ef4444;
	}
	.save-btn:hover {
		border-color: #22c55e;
		color: #22c55e;
	}
	.cancel-btn:hover {
		border-color: #ef4444;
		color: #ef4444;
	}

	.mono {
		font-family: monospace;
		font-size: 0.78rem;
		color: #888;
	}
	.dim {
		color: #666;
	}
	.path {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		direction: rtl;
	}

	.empty-hint {
		font-size: 0.78rem;
		color: #666;
		line-height: 1.5;
		padding: 6px 0;
	}

	.versions {
		margin-top: 12px;
		border-top: 1px solid #2e2e2e;
		padding-top: 10px;
		min-height: 0;
		overflow-y: auto;
	}
	.versions-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		margin-bottom: 6px;
	}
	.note {
		font-size: 0.68rem;
		color: #666;
	}

	.refusal {
		font-size: 0.75rem;
		color: #f59e0b;
		line-height: 1.45;
		padding: 6px 8px;
		border: 1px solid #4a3a10;
		border-radius: 3px;
		margin-bottom: 8px;
	}

	ul {
		list-style: none;
		display: flex;
		flex-direction: column;
	}
	li {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		padding: 6px 0;
		border-bottom: 1px solid #2a2a2a;
	}
	li.auto .version-name {
		color: #9a9a9a;
	}
	.what {
		display: flex;
		flex-direction: column;
		gap: 1px;
		min-width: 0;
	}
	.version-name {
		font-size: 0.83rem;
		color: #e0e0e0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.meta {
		font-size: 0.68rem;
		color: #666;
	}
	.meta.warn {
		color: #f59e0b;
	}
	.acts {
		display: flex;
		gap: 5px;
		flex-shrink: 0;
	}

	.scrim {
		position: fixed;
		inset: 0;
		z-index: 90;
		display: grid;
		place-items: center;
		background: rgb(0 0 0 / 55%);
	}
	.dialog {
		width: min(420px, 90vw);
		background: #222;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		padding: 18px;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.dialog h3 {
		font-size: 0.95rem;
		color: #fff;
		font-weight: 600;
	}
	.dialog p {
		font-size: 0.8rem;
		color: #aaa;
		line-height: 1.5;
	}
	.dialog .fine {
		font-size: 0.72rem;
		color: #666;
	}
	.dialog .acts {
		justify-content: flex-end;
		margin-top: 4px;
	}
</style>
