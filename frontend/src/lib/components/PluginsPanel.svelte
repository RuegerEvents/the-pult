<script lang="ts">
	/**
	 * What plugins the show carries, and what this station is doing about them.
	 *
	 * Two sources, deliberately: `plugin_packages` is the show's roster and is
	 * the same on every console, while the LOCAL `plugins` state is this
	 * station's own answer — what it has fetched, what failed here, and what it
	 * is running from a directory instead. A row is one plugin seen through
	 * both, which is why a station with a missing bundle and a station running
	 * it can show the same list and disagree honestly about one column.
	 */
	import { onMount } from 'svelte';
	import { canEdit } from '$lib/stores/showLock.js';
	import Dialog from './Dialog.svelte';
	import { pluginsState } from '$lib/stores/plugins.js';
	import { getDataContext } from '$lib/ws/context.js';
	import type {
		PluginDatum,
		PluginInfo,
		PluginPackage,
		PluginStage
	} from '$lib/generated/index.js';

	const data = getDataContext();

	let packages = $state<PluginPackage[]>([]);
	let stored = $state<PluginDatum[]>([]);
	let installing = $state(false);
	let installError = $state<string | null>(null);
	let fileInput = $state<HTMLInputElement | null>(null);
	/**
	 * What the station carries inside itself, installable with one press rather than a
	 * file picker on whichever tablet pressed Install — where the bundle never is.
	 */
	let builtin = $state<{ id: string; name: string; version: string }[]>([]);
	const offered = $derived(builtin.filter((b) => !packages.some((pkg) => pkg.plugin_id === b.id)));

	const runtime = $derived($pluginsState.plugins);

	/** This station's view of one plugin, if it has one. */
	const localOf = (id: string): PluginInfo | undefined => runtime.find((p) => p.id === id);

	/** A plugin running here that the show knows nothing about. */
	const strays = $derived(
		runtime.filter((p) => !packages.some((pkg) => pkg.plugin_id === p.id))
	);

	/**
	 * Data in the show belonging to no plugin anybody here has.
	 *
	 * Removing a plugin deliberately does not delete what it remembered, so that
	 * a plugin removed by mistake and put back finds its macros where it left
	 * them. The cost of that is data nobody owns, which would accumulate unseen
	 * — so it is shown, grouped by the plugin id that wrote it, and can be
	 * cleared out on purpose.
	 */
	const orphaned = $derived.by(() => {
		const known = new Set([
			...packages.map((pkg) => pkg.plugin_id),
			...runtime.map((p) => p.id)
		]);
		const groups = new Map<string, PluginDatum[]>();
		for (const row of stored) {
			if (known.has(row.plugin_id)) continue;
			const rows = groups.get(row.plugin_id) ?? [];
			rows.push(row);
			groups.set(row.plugin_id, rows);
		}
		return [...groups.entries()]
			.map(([pluginId, rows]) => ({
				pluginId,
				rows: rows.toSorted((a, b) => `${a.store}${a.key}`.localeCompare(`${b.store}${b.key}`))
			}))
			.toSorted((a, b) => a.pluginId.localeCompare(b.pluginId));
	});

	/**
	 * Removing a plugin and forgetting what one left behind both ask first. The row
	 * comes back with Ctrl-Z, but what the plugin was holding in memory on every
	 * station does not, and forgetting is every key it ever stored, at once.
	 */
	let asking = $state<{ what: 'remove' | 'forget'; id: string; name: string } | null>(null);

	async function answer() {
		const ask = asking;
		asking = null;
		if (!ask) return;
		if (ask.what === 'remove') await data.plugin_packages.byId(ask.id).delete();
		else await forget(ask.id);
	}

	/** Forget everything one absent plugin left behind. */
	async function forget(pluginId: string) {
		const rows = orphaned.find((group) => group.pluginId === pluginId)?.rows ?? [];
		for (const row of rows) {
			await data.plugin_data.byId(row.id).delete();
		}
	}

	const STAGES: { value: PluginStage; label: string; hint: string }[] = [
		{ value: 'Both', label: 'Always', hint: 'relevant while building and while running' },
		{ value: 'Setup', label: 'Setup', hint: 'used while the show is being built' },
		{ value: 'Runtime', label: 'Runtime', hint: 'used while the show is being run' }
	];

	function statusText(info: PluginInfo | undefined): string {
		if (!info) return 'not started here';
		switch (info.status.state) {
			case 'Fetching':
				return 'fetching the bundle…';
			case 'Loading':
				return 'loading';
			case 'Running':
				return 'running';
			case 'Failed':
				return info.status.reason;
		}
	}

	const statusKind = (info: PluginInfo | undefined) =>
		!info ? 'idle' : info.status.state.toLowerCase();

	/** What the bundle says this plugin may do, in words rather than flags. */
	function permissionWords(info: PluginInfo | undefined): string[] {
		if (!info) return [];
		const p = info.permissions;
		const words: string[] = [];
		if (p.data === 'read-write') words.push('changes the show');
		else if (p.data === 'read') words.push('reads the show');
		if (p.commands) words.push('runs commands');
		if (p.http.length > 0) words.push(`talks to ${p.http.join(', ')}`);
		if (p.env.length > 0) words.push(`is given ${p.env.join(', ')}`);
		return words;
	}

	const shortDigest = (sha: string) => sha.slice(0, 12);

	async function install(file: File) {
		installError = null;
		installing = true;
		try {
			const response = await fetch('/api/plugins', {
				method: 'POST',
				headers: { 'content-type': 'application/vnd.pult.plugin+zip' },
				body: await file.arrayBuffer()
			});
			if (!response.ok) {
				installError = (await response.text()) || `the station refused it (${response.status})`;
			}
		} catch (e) {
			installError = e instanceof Error ? e.message : String(e);
		} finally {
			installing = false;
			if (fileInput) fileInput.value = '';
		}
	}

	async function installBuiltin(id: string) {
		installError = null;
		installing = true;
		try {
			const response = await fetch(`/api/plugins/builtin/${encodeURIComponent(id)}`, { method: 'POST' });
			if (!response.ok) {
				installError = (await response.text()) || `the station refused it (${response.status})`;
			}
		} catch (e) {
			installError = e instanceof Error ? e.message : String(e);
		} finally {
			installing = false;
		}
	}

	function onPick(event: Event) {
		const file = (event.target as HTMLInputElement).files?.[0];
		if (file) void install(file);
	}

	onMount(() => data.plugin_packages.subscribeDeep((v) => { packages = v; }));
	onMount(() => {
		fetch('/api/plugins/builtin')
			.then((r) => (r.ok ? r.json() : []))
			.then((list) => (builtin = list))
			.catch(() => {});
	});
	onMount(() => data.plugin_data.subscribeDeep((v) => { stored = v; }));
</script>

<div class="plugins">
	<section class="block">
		<header class="block-head">
			<h2>Plugins</h2>
			{#if $canEdit}
				<button class="ghost" onclick={() => fileInput?.click()} disabled={installing}>
					{installing ? 'Installing…' : '+ Install'}
				</button>
				<input
					class="hidden-file"
					type="file"
					accept=".zip,application/vnd.pult.plugin+zip,application/zip"
					bind:this={fileInput}
					onchange={onPick}
				/>
			{/if}
		</header>

		{#if installError}
			<p class="error">{installError}</p>
		{/if}

		{#if $canEdit && offered.length > 0}
			<div class="builtin">
				<span class="hint">Built into this station:</span>
				{#each offered as plugin (plugin.id)}
					<button class="ghost" disabled={installing} onclick={() => installBuiltin(plugin.id)}>
						+ {plugin.name} <span class="hint">{plugin.version}</span>
					</button>
				{/each}
			</div>
		{/if}

		{#if packages.length === 0}
			<p class="empty">
				This show carries no plugins. Installing one here puts it on every station in
				the session.
			</p>
		{:else}
			{#each STAGES as stage (stage.value)}
				{@const rows = packages.filter((p) => p.stage === stage.value)}
				{#if rows.length > 0}
					<h3 class="stage">{stage.label}<span class="hint"> — {stage.hint}</span></h3>
					<table class="carried">
						<thead>
							<tr>
								<th>Plugin</th><th>Version</th><th>Bundle</th>
								<th>May</th><th>On</th><th>Here</th><th></th>
							</tr>
						</thead>
						<tbody>
							{#each rows as pkg (pkg.id)}
								{@const here = localOf(pkg.plugin_id)}
								<tr class:off={!pkg.enabled}>
									<td>
										<span class="name">{pkg.name}</span>
										<span class="id">{pkg.plugin_id}</span>
									</td>
									<td>{pkg.version}</td>
									<td><code title={pkg.sha256}>{shortDigest(pkg.sha256)}</code></td>
									<td class="may">
										{#if permissionWords(here).length === 0}
											<span class="quiet">nothing outside itself</span>
										{:else}
											{permissionWords(here).join(' · ')}
										{/if}
									</td>
									<td>
										{#if $canEdit}
											<input
												type="checkbox"
												checked={pkg.enabled}
												onchange={(e) =>
													data.plugin_packages
														.byId(pkg.id)
														.enabled.set(e.currentTarget.checked)}
											/>
										{:else}
											{pkg.enabled ? 'yes' : 'no'}
										{/if}
									</td>
									<td class="status {statusKind(here)}">
										{statusText(here)}
										{#if here?.overridden_by_disk}
											<span class="overridden">
												running the copy on this machine's disk, not this bundle
											</span>
										{/if}
									</td>
									<td>
										{#if $canEdit}
											<button
												class="ghost danger"
												onclick={() => (asking = { what: 'remove', id: pkg.id, name: pkg.plugin_id })}
											>
												Remove
											</button>
										{/if}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				{/if}
			{/each}
		{/if}
	</section>

	{#if orphaned.length > 0}
		<section class="block">
			<header class="block-head"><h2>Left behind</h2></header>
			<p class="hint">
				Data in this show belonging to plugins nothing here has. Removing a plugin does
				not delete what it remembered, so putting it back finds its work where it left
				it — but nothing else will ever read this.
			</p>
			<ul class="strays">
				{#each orphaned as group (group.pluginId)}
					<li>
						<span class="name">{group.pluginId}</span>
						<span class="id">
							{group.rows.length}
							{group.rows.length === 1 ? 'value' : 'values'} in
							{new Set(group.rows.map((r) => r.store)).size}
							{new Set(group.rows.map((r) => r.store)).size === 1 ? 'store' : 'stores'}
						</span>
						{#if $canEdit}
							<button
								class="ghost danger"
								onclick={() => (asking = { what: 'forget', id: group.pluginId, name: group.pluginId })}
							>
								Forget
							</button>
						{/if}
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if strays.length > 0}
		<section class="block">
			<header class="block-head"><h2>On this machine only</h2></header>
			<p class="hint">
				Loaded from a plugin directory rather than from the show, so no other station
				has them.
			</p>
			<ul class="strays">
				{#each strays as info (info.id)}
					<li>
						<span class="name">{info.name}</span>
						<span class="id">{info.id}</span>
						<span class="version">{info.version}</span>
						<span class="status {statusKind(info)}">{statusText(info)}</span>
					</li>
				{/each}
			</ul>
		</section>
	{/if}
</div>

{#if asking}
	<Dialog
		title={asking.what === 'remove' ? 'Remove a plugin' : 'Forget what a plugin left'}
		onclose={() => (asking = null)}
	>
		<p class="ask">
			{#if asking.what === 'remove'}
				<b>{asking.name}</b> stops on every station of this show. Ctrl-Z puts it back, but not
				anything it was holding in memory. What it stored stays, under <i>Left behind</i>.
			{:else}
				Everything <b>{asking.name}</b> stored in this show is deleted. A plugin put back later
				starts from nothing.
			{/if}
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (asking = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={answer}>{asking?.what === 'remove' ? 'Remove' : 'Forget'}</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.ask {
		padding: 14px;
		font-size: var(--font-sm);
		line-height: 1.5;
	}
	.plugins {
		padding: 0.5rem;
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
	}
	.block-head {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	h2 {
		flex: 1;
		margin: 0;
		font-size: 0.95rem;
	}
	h3.stage {
		margin: 0.6rem 0 0.2rem;
		font-size: 0.8rem;
		font-weight: 600;
	}
	.hint,
	.quiet {
		color: var(--text-dim);
		font-weight: 400;
		font-size: 0.8rem;
	}
	.empty {
		color: var(--text-dim);
		margin: 0.5rem 0;
	}
	.error {
		color: var(--danger, #d66);
		margin: 0.4rem 0;
	}
	.builtin { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 0 0 12px; }
	.hidden-file {
		display: none;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
	}
	th {
		text-align: left;
		font-weight: 500;
		color: var(--text-dim);
		padding: 0.2rem 0.4rem;
	}
	td {
		padding: 0.3rem 0.4rem;
		vertical-align: top;
		border-top: 1px solid var(--line, #2a2a2a);
	}
	tr.off {
		opacity: 0.55;
	}
	.name {
		display: block;
	}
	.id,
	.version {
		color: var(--text-dim);
		font-size: 0.78rem;
	}
	code {
		font-size: 0.78rem;
		color: var(--text-dim);
	}
	.may {
		max-width: 22rem;
	}
	.status.running {
		color: var(--ok, #6c6);
	}
	.status.failed {
		color: var(--danger, #d66);
	}
	.status.fetching,
	.status.loading {
		color: var(--text-dim);
	}
	.overridden {
		display: block;
		color: var(--warn, #db4);
		font-size: 0.78rem;
	}
	.strays {
		list-style: none;
		margin: 0.3rem 0 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		font-size: 0.85rem;
	}
	.strays li {
		display: flex;
		gap: 0.5rem;
		align-items: baseline;
	}
	.strays .name {
		display: inline;
	}
</style>
