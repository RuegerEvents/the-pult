<script lang="ts">
	/**
	 * Whether this page has its station. A dot while it does, with the word in the
	 * tooltip; the word itself only when it does not, which is the one time somebody
	 * needs to read it.
	 */
	let { connected }: { connected: boolean } = $props();
</script>

<div class="connection-status" class:connected title={connected ? 'Connected to the station' : undefined}>
	<span class="dot"></span>
	{#if !connected}<span class="label">Disconnected</span>{/if}
</div>

<style>
	.connection-status {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 0 6px;
		font-size: 0.75rem;
		color: var(--bad, #ef4444);
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--bad, #ef4444);
		transition: background 0.3s;
	}

	.connected .dot {
		background: #4caf50;
	}
</style>
