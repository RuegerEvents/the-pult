<script lang="ts">
	/**
	 * Asking before an older showfile is upgraded.
	 *
	 * An upgrade is a one-way door — an older build cannot open the result — so the
	 * station never walks a file up unasked; it answers `{needsUpgrade}` and this is
	 * the question. What it says is the whole of what changes: a version of the show
	 * as it was is kept, history starts at the upgrade, and the other stations in the
	 * session have to be on this build too.
	 */

	import Dialog from './Dialog.svelte';
	import type { Upgrade } from '$lib/shows.js';

	let {
		name,
		upgrade,
		restoring = false,
		onaccept,
		onclose
	}: {
		/** The show, or the version, being asked about. */
		name: string;
		upgrade: Upgrade;
		/** A version being restored rather than a show being opened. */
		restoring?: boolean;
		onaccept: () => void;
		onclose: () => void;
	} = $props();
</script>

<Dialog title={restoring ? `Restore “${name}”?` : `Upgrade “${name}”?`} {onclose} {onaccept}>
	{#if restoring}
		<p>
			This version was saved by an older build of the console (generation {upgrade.from}; this
			one reads {upgrade.to}). Restoring reads a copy of it upgraded to this build. The version
			itself stays as it was.
		</p>
	{:else}
		<p>
			This show was saved by an older build of the console (generation {upgrade.from}; this
			one reads {upgrade.to}). To open it here it has to be upgraded.
		</p>
		<ul>
			<li>The show as it is now is kept as a version, <em>Before upgrading from generation {upgrade.from}</em>.</li>
			<li>History starts at the upgrade: Ctrl-Z cannot reach anything done before it.</li>
			<li>An older build cannot open it afterwards. Upgrade every station in the session together.</li>
		</ul>
	{/if}
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		<button class="btn btn-primary" onclick={onaccept}>{restoring ? 'Upgrade and restore' : 'Upgrade and open'}</button>
	{/snippet}
</Dialog>

<style>
	p,
	ul {
		margin: 0 0 10px;
		line-height: 1.45;
	}
	ul {
		padding-left: 18px;
	}
	li {
		margin-bottom: 4px;
	}
</style>
