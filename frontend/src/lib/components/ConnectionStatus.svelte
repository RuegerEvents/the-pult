<script lang="ts">
	let { connected }: { connected: boolean } = $props();
</script>

<div class="connection-status" class:connected>
	<span class="dot"></span>
	<span class="label">{connected ? 'Connected' : 'Disconnected'}</span>
</div>

<style>
	.connection-status {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.75rem;
		color: #888;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: #555;
		transition: background 0.3s;
	}

	.connected .dot {
		background: #4caf50;
	}

	.connected .label {
		color: #4caf50;
	}
</style>
