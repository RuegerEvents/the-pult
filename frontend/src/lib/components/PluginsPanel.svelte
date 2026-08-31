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
	import { editing } from '$lib/stores/editing.js';
	import { pluginsState } from '$lib/stores/plugins.js';
	import { getDataContext } from '$lib/ws/context.js';
	import type { PluginInfo, PluginPackage, PluginStage } from '$lib/generated/index.js';

	const data = getDataContext();
	const unlocked = editing('plugins');

	let packages = $state<PluginPackage[]>([]);
	let installing = $state(false);
	let installError = $state<string | null>(null);
	let fileInput = $state<HTMLInputElement | null>(null);

	const runtime = $derived($pluginsState.plugins);

	/** This station's view of one plugin, if it has one. */
	const localOf = (id: string): PluginInfo | undefined => runtime.find((p) => p.id === id);

	/** A plugin running here that the show knows nothing about. */
	const strays = $derived(
		runtime.filter((p) => !packages.some((pkg) => pkg.plugin_id === p.id))
	);

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

	function onPick(event: Event) {
		const file = (event.target as HTMLInputElement).files?.[0];
		if (file) void install(file);
	}

	onMount(() => data.plugin_packages.subscribeDeep((v) => { packages = v; }));
</script>

<div class="plugins">
	<section class="block">
		<header class="block-head">
			<h2>Plugins</h2>
			{#if $unlocked}
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
										{#if $unlocked}
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
										{#if $unlocked}
											<button
												class="ghost danger"
												onclick={() => data.plugin_packages.byId(pkg.id).delete()}
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

<style>
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
