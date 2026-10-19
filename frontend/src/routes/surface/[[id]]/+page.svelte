<script lang="ts">
	/**
	 * A surface, full-screen: what a phone at the maze entrance opens.
	 *
	 * `/surface/<id>` is one surface and `/surface` the list of them. The station serves
	 * the app for any path it has no file for, so a deep link survives a reload with
	 * nothing on the server knowing this route exists.
	 *
	 * The layout leaves this page with no top bar and no cover. When the console goes
	 * away the tiles stay, dimmed and unpressable, under a banner that says so — the
	 * socket's own reconnect brings them back without the page being touched.
	 */

	import { page } from '$app/state';
	import SurfaceTiles from '$lib/components/surface/SurfaceTiles.svelte';
	import { collection } from '$lib/stores/show.js';
	import { connection } from '$lib/stores/connection.js';
	import { surfaceShow } from '$lib/stores/surface.js';
	import { surfaceUrl } from '$lib/surface.js';
	import { getClientContext, getStationContext } from '$lib/ws/context.js';

	const client = getClientContext();
	const station = getStationContext();
	const surfaces = collection('surfaces');

	const id = $derived(page.params.id ?? null);
	const surface = $derived(id ? ($surfaces.find((s) => s.id === id) ?? null) : null);
	const live = $derived($connection.connected);

	const linkTo = (surfaceId: string) =>
		new URL(surfaceUrl({ origin: page.url.origin, search: page.url.search }, surfaceId)).pathname +
		page.url.search;

	$effect(() => {
		document.title = surface ? `${surface.name} — the-pult` : 'Surfaces — the-pult';
	});
</script>

<div class="phone">
	<header class="bar">
		<span class="title">{surface?.name ?? 'Surfaces'}</span>
		<span class="status" class:live>
			<span class="dot"></span>{live ? 'Connected' : 'Not connected'}
		</span>
	</header>

	{#if !live && $connection.everConnected}
		<div class="lost" role="alert">
			<strong>Not connected to the console.</strong>
			<span>What you see is what it last said. Buttons do nothing until it is back.</span>
			<button class="retry" onclick={() => client.retryNow()}>Try again now</button>
		</div>
	{/if}

	<div class="body">
		{#if $station && !$station.show}
			<p class="note">This console has no show open.</p>
		{:else if surface}
			<SurfaceTiles {surface} show={$surfaceShow} {live} />
		{:else}
			{#if id}
				<p class="note">This show has no surface with that link. Pick one:</p>
			{/if}
			{#if $surfaces.length === 0}
				<p class="note">This show has no surfaces yet. Make one in the Surfaces panel on a desk.</p>
			{/if}
			<nav class="list">
				{#each $surfaces as s (s.id)}
					<a class="pick" href={linkTo(s.id)}>{s.name}</a>
				{/each}
			</nav>
		{/if}
	</div>
</div>

<style>
	.phone {
		height: 100%;
		display: flex;
		flex-direction: column;
		background: #141414;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px 14px;
		padding-top: max(10px, env(safe-area-inset-top));
		background: #0d0d0d;
		border-bottom: 1px solid #2a2a2a;
		flex-shrink: 0;
	}
	.title {
		flex: 1;
		font-size: 17px;
		font-weight: 700;
		color: #fff;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.status {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		color: #f87171;
	}
	.status.live {
		color: #86efac;
	}
	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
		background: currentColor;
	}
	.lost {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 12px 14px;
		background: #7f1d1d;
		color: #fee2e2;
		font-size: 14px;
		line-height: 1.35;
		flex-shrink: 0;
	}
	.retry {
		align-self: flex-start;
		min-height: 44px;
		padding: 0 16px;
		border-radius: 8px;
		border: 1px solid #fca5a5;
		background: transparent;
		color: #fff;
		font: inherit;
		font-weight: 600;
	}
	.body {
		flex: 1;
		overflow-y: auto;
		padding: 12px;
		padding-bottom: max(12px, env(safe-area-inset-bottom));
	}
	.note {
		color: #999;
		font-size: 15px;
		line-height: 1.4;
		margin-bottom: 12px;
	}
	.list {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.pick {
		display: flex;
		align-items: center;
		min-height: 64px;
		padding: 0 16px;
		border-radius: 10px;
		background: #2d3a4f;
		color: #fff;
		font-size: 17px;
		font-weight: 600;
		text-decoration: none;
	}
</style>
