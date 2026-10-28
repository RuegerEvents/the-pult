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
	 *
	 * Laid out as two columns when there is room — the show and its title block on
	 * one side, the versions on the other — and one when there is not, by a container
	 * query rather than the window, because a Setup section and a tile are different
	 * widths on the same screen.
	 */

	import { onMount } from 'svelte';

	import type { Production } from '$lib/generated/index.js';

	import { focusOnMount, selectOnMount } from '$lib/actions.js';
	import { getClientContext, getDataContext, getStationContext } from '$lib/ws/context.js';
	import { collection, show as openShow } from '$lib/stores/show.js';
	import { users } from '$lib/stores/user.js';
	import { addToast } from '$lib/toasts.js';
	import { asSize, parentPath } from '$lib/shows.js';
	import { readPreferences } from '$lib/preferences.js';
	import Dialog from '$lib/components/Dialog.svelte';
	import Help from '$lib/components/Help.svelte';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
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

	/** Take the version the naming dialog describes. A name is optional: one left
	 * empty is shown by when it was taken, which is the station's own rule. */
	function saveNamed() {
		const typed = versionName.trim();
		naming = false;
		versionName = '';
		void takeAVersion(typed || undefined);
	}
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

	/**
	 * The title block's fields, in the order they read on paper.
	 *
	 * A list rather than markup per field, because they are the same control seven
	 * times and the only thing that differs is what it is called.
	 */
	const PRODUCTION_FIELDS = [
		{ key: 'title', label: 'Production', hint: 'Defaults to the show’s own name' },
		{ key: 'venue', label: 'Venue', hint: '' },
		{ key: 'address', label: 'Address', hint: '' },
		{ key: 'dates', label: 'Dates', hint: '02.05.2025 - 05.05.2025' },
		{ key: 'designer', label: 'Drawn by', hint: '' },
		{ key: 'contact', label: 'Contact', hint: '' },
		{ key: 'revision', label: 'Revision', hint: 'Draft, Rev C…' }
	] as const;

	const EMPTY_PRODUCTION: Production = {
		title: '',
		venue: '',
		address: '',
		dates: '',
		designer: '',
		contact: '',
		revision: ''
	};

	const production = $derived($openShow?.production ?? EMPTY_PRODUCTION);

	/**
	 * One field of it, written as the whole block.
	 *
	 * `production` is one column, so there is no path to `venue` on its own — the same
	 * shape `Sheet::blocks` has, for the same reason: it is a record somebody edits as
	 * a unit and nothing else points into it.
	 */
	async function setProduction(key: keyof Production, value: string) {
		await data.show.production.set({ ...production, [key]: value });
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

	/**
	 * Put a version back. As ordinary writes on the station, in one gesture: every
	 * station in the session follows, nobody restarts, and Ctrl-Z takes it back. What is
	 * running — cues up, timelines rolling — stays running.
	 */
	async function restore(version: Version) {
		confirming = null;
		restoreRefusal = '';
		try {
			const answer = (await client.call('show.restore', { versionId: version.id })) as { changed?: number } | null;
			const n = answer?.changed ?? 0;
			addToast(n === 0 ? 'The show already was that version' : `Restored “${label(version)}”: ${n} ${n === 1 ? 'change' : 'changes'}`, 'success');
		} catch (e) {
			// Shown in place rather than as a toast, because it is an answer to the button
			// that was just pressed: most often a version this station never held.
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

<div class="show-panel">
	{#if !$openShow}
		<p class="empty-hint">No show open.</p>
	{:else}
		<div class="columns">
			<div class="column">
				<section>
					<h3 class="section-title">Show</h3>
					<div class="fields">
						<span class="label">Name</span>
						{#if editingName}
							<form
								class="inline"
								onsubmit={(e) => {
									e.preventDefault();
									saveName();
								}}
							>
								<input
									class="input"
									bind:value={draftName}
									disabled={saving}
									use:focusOnMount
									use:selectOnMount
									onkeydown={(e) => {
										if (e.key === 'Escape') editingName = false;
									}}
								/>
								<button class="btn btn-primary" type="submit" disabled={saving}>Save</button>
								<button class="btn btn-ghost" type="button" onclick={() => (editingName = false)}>Cancel</button>
							</form>
						{:else}
							<button
								class="name-value"
								onclick={() => {
									draftName = $openShow?.name ?? '';
									editingName = true;
								}}
								title="Rename the show"
							>
								{$openShow.name}
								<Pencil size={12} />
							</button>
						{/if}

						{#if bundle}
							<span class="label">Folder</span>
							<span class="mono" title={bundle.path}>{bundle.name}</span>
							<span class="label">In</span>
							<span class="mono path" title={bundle.path}>{parentPath(bundle.path)}</span>
						{/if}
						<span class="label">Show ID</span>
						<span class="mono" title={$openShow.id}>{$openShow.id.slice(0, 8)}…</span>
					</div>
				</section>

				<!--
					The title block, and the reason it is here rather than in `preferences.toml`:
					a showfile travels, and paperwork exported from a file you sent somebody
					should still carry your name. Every line may be empty, and an empty one is
					omitted from the sheet rather than printed as a blank row.
				-->
				<section>
					<h3 class="section-title">
						Title block
						<Help label="What the title block is">
							What the Paperwork panel prints in the corner of every sheet. It belongs to the show,
							so a file sent to somebody else still carries your name. An empty line is left off
							the sheet.
						</Help>
					</h3>
					<div class="fields">
						{#each PRODUCTION_FIELDS as field (field.key)}
							<label class="label" for="production-{field.key}">{field.label}</label>
							<input
								id="production-{field.key}"
								class="input"
								value={production[field.key]}
								placeholder={field.hint}
								oninput={(e) => setProduction(field.key, e.currentTarget.value)}
							/>
						{/each}
					</div>
				</section>
			</div>

			<section class="column">
				<h3 class="section-title">
					Versions
					<Help label="What a version is">
						A version is a point to come back to, not a flush: every change is already on the disk.
						⌘S takes one. Each station keeps its own copy of the show as it was, so a station that
						joined later lists a version it cannot restore.
					</Help>
					<span class="spacer"></span>
					{#if autosave}
						<span class="note">
							{#if autosave.minutes > 0}
								auto every {autosave.minutes} min, keeping {autosave.keep}
							{:else}
								autosave off
							{/if}
						</span>
					{/if}
					<button class="btn btn-primary" onclick={() => (naming = true)} disabled={!bundle}>Save version…</button>
				</h3>

				{#if restoreRefusal}
					<p class="refusal">{restoreRefusal}</p>
				{/if}

				{#if !ordered.length}
					<p class="empty-hint">No versions yet.</p>
				{:else}
					<div class="table-wrap">
						<table class="rack">
							<thead>
								<tr><th>Version</th><th>Saved</th><th>By</th><th></th></tr>
							</thead>
							<tbody>
								{#each ordered as version (version.id)}
									{@const holds = here.includes(version.id)}
									<tr class:auto={version.automatic}>
										<td class="version-name">
											{label(version)}
											{#if version.automatic}<span class="tag">auto</span>{/if}
											{#if !holds}<span class="tag warn" title="This station never held this version; restore it on the one that took it">not here</span>{/if}
										</td>
										<td class="dim">{when(version.created_at)}</td>
										<td class="dim">{whoTook(version)}</td>
										<td class="acts">
											<button
												class="btn btn-ghost"
												disabled={!holds}
												title={holds
													? 'Put the show back as it was'
													: 'This station never held this version; open it on the one that took it'}
												onclick={() => {
													restoreRefusal = '';
													confirming = version;
												}}><RotateCcw size={13} />Restore</button
											>
											<button class="btn btn-icon" onclick={() => forget(version)} title="Delete this version" aria-label="Delete this version"
												><Trash2 size={13} /></button
											>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</section>
		</div>
	{/if}
</div>

{#if naming}
	<Dialog title="Save a version" onclose={() => (naming = false)} onaccept={saveNamed}>
		<form
			class="naming"
			onsubmit={(e) => {
				e.preventDefault();
				saveNamed();
			}}
		>
			<input class="input" bind:value={versionName} placeholder="Name (optional)…" use:focusOnMount />
			<p class="fine">Left empty, it is listed by when it was saved.</p>
		</form>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (naming = false)}>Cancel</button>
			<button class="btn btn-primary" onclick={saveNamed}>Save</button>
		{/snippet}
	</Dialog>
{/if}

{#if confirming}
	{@const version = confirming}
	<Dialog title="Restore" onclose={() => (confirming = null)}>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (confirming = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={() => restore(version)}>Restore</button>
		{/snippet}
		<div class="ask">
			<h3>Restore “{label(version)}”?</h3>
			<p>
				The show goes back to how it was at {when(version.created_at)}, on every station in the
				session. Ctrl-Z takes it back, and a version of what it is now is taken first as well.
			</p>
			<p class="fine">Nothing restarts. What is running keeps running: cues that are up, timelines rolling.</p>
		</div>
	</Dialog>
{/if}

<style>
	.show-panel {
		container-type: inline-size;
		min-height: 0;
	}
	.columns {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 20px;
	}
	@container (min-width: 760px) {
		.columns {
			grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
		}
	}
	.column {
		display: flex;
		flex-direction: column;
		gap: 20px;
		min-width: 0;
	}
	section {
		display: flex;
		flex-direction: column;
		gap: 10px;
		min-width: 0;
	}
	.section-title {
		display: flex;
		align-items: center;
		gap: 6px;
		margin: 0;
		font-size: 11px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-faint);
	}
	.section-title .spacer {
		flex: 1;
	}
	.section-title .note,
	.section-title :global(.btn) {
		text-transform: none;
		letter-spacing: 0;
		font-weight: 400;
	}
	.fields .input {
		width: 100%;
	}

	.name-value {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		justify-self: start;
		padding: 0;
		border: none;
		background: none;
		color: var(--text);
		font: inherit;
		font-size: var(--font-md, 14px);
		cursor: pointer;
	}
	.name-value :global(svg) {
		color: var(--text-faint);
		opacity: 0;
		transition: opacity 0.15s;
	}
	.name-value:hover :global(svg),
	.name-value:focus-visible :global(svg) {
		opacity: 1;
	}

	.mono {
		font-family: var(--font-mono, monospace);
		font-size: var(--font-xs);
		color: var(--text-dim);
		min-width: 0;
	}
	.path {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		direction: rtl;
		text-align: left;
	}
	.dim {
		color: var(--text-dim);
	}
	.note {
		font-size: var(--font-xs);
		color: var(--text-faint);
	}

	.empty-hint {
		margin: 0;
		font-size: var(--font-sm);
		color: var(--text-faint);
	}

	.refusal {
		margin: 0;
		font-size: var(--font-sm);
		color: var(--warn, #f59e0b);
		line-height: 1.45;
		padding: 6px 8px;
		border: 1px solid #4a3a10;
		border-radius: 4px;
	}

	.table-wrap {
		min-height: 0;
		overflow: auto;
	}
	.rack {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	.rack th {
		text-align: left;
		font-weight: 500;
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint);
		padding: 0 10px 6px 0;
	}
	.rack td {
		padding: 6px 10px 6px 0;
		border-top: 1px solid var(--line);
		vertical-align: middle;
	}
	.rack tr.auto .version-name {
		color: var(--text-dim);
	}
	.version-name {
		color: var(--text);
	}
	.tag {
		margin-left: 6px;
		padding: 1px 6px;
		border: 1px solid var(--line-strong);
		border-radius: 8px;
		font-size: 10px;
		color: var(--text-faint);
		white-space: nowrap;
	}
	.tag.warn {
		border-color: color-mix(in srgb, var(--warn, #f59e0b) 50%, transparent);
		color: var(--warn, #f59e0b);
	}
	.acts {
		white-space: nowrap;
		text-align: right;
		padding-right: 0 !important;
	}
	.acts :global(.btn) {
		gap: 4px;
	}

	.naming {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.naming .input {
		width: 100%;
	}
	.naming .fine {
		margin: 0;
		font-size: var(--font-xs);
		color: var(--text-faint);
	}

	.ask {
		display: flex;
		flex-direction: column;
		gap: 10px;
		max-width: 44ch;
	}
	.ask h3 {
		margin: 0;
		font-size: 0.95rem;
		color: var(--text);
		font-weight: 600;
	}
	.ask p {
		margin: 0;
		font-size: var(--font-sm);
		color: var(--text-dim);
		line-height: 1.5;
	}
	.ask .fine {
		font-size: var(--font-xs);
		color: var(--text-faint);
	}
</style>
