<script lang="ts">
	/**
	 * The discrete things a connector said, newest last.
	 *
	 * The station drains its ring on every look and this list is what the browser
	 * made of the batches — see `$lib/wire.ts`, which is where that rule lives. What
	 * did not survive one of the two rings is counted rather than swallowed, for the
	 * reason the system log counts a gap in its `seq`: an invisible hole in a
	 * diagnostic is the one thing worse than a visible one.
	 */

	import type { MessageTraffic } from '$lib/generated/index.js';

	const { of }: { of: MessageTraffic } = $props();

	let follow = $state(true);
	let list = $state<HTMLElement | null>(null);

	const clock = (ms: number) =>
		new Date(ms).toLocaleTimeString(undefined, { hour12: false }) +
		'.' +
		String(ms % 1000).padStart(3, '0');

	$effect(() => {
		// Read, so this runs again on every arrival.
		of.messages.length;
		if (follow && list) list.scrollTop = list.scrollHeight;
	});
</script>

{#if of.dropped > 0}
	<p class="gap">{of.dropped.toLocaleString()} messages did not reach this list.</p>
{/if}

{#if of.messages.length === 0}
	<p class="empty">Nothing has been said since you started watching.</p>
{:else}
	<label class="follow"><input type="checkbox" bind:checked={follow} /> follow</label>
	<div class="log" bind:this={list}>
		{#each of.messages as message, at (at)}
			<div class="line">
				<span class="at">{clock(message.at_ms)}</span>
				<span class="to">{message.to}</span>
				<span class="what">{message.what}</span>
				<span class="detail">{message.detail}</span>
			</div>
		{/each}
	</div>
{/if}

<style>
	.log { max-height: 340px; overflow-y: auto; font-size: 12px; font-variant-numeric: tabular-nums; }
	.line { display: flex; gap: 10px; padding: 1px 0; white-space: nowrap; }
	.at { color: #666; flex: none; }
	.to { color: #9bb8e0; flex: none; min-width: 130px; }
	.what { color: #bbb; flex: none; min-width: 90px; }
	.detail { color: #808080; overflow: hidden; text-overflow: ellipsis; }
	.empty { color: #777; font-size: 13px; }
	.gap { color: #c9a227; font-size: 12px; margin: 0 0 6px; }
	.follow { color: #777; font-size: 12px; display: block; margin-bottom: 4px; }
</style>
