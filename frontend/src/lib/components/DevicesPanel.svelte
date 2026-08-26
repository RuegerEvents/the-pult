<script lang="ts">
	import { onMount } from 'svelte';
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import type { DevicesState, DiscoveredDevice } from '$lib/generated/index.js';

	const client = getClientContext();

	let devices = $state<DevicesState>({ discovered: {}, broker_addr: null, active: false });
	let busy = $state<string | null>(null);

	const listed = $derived(Object.values(devices.discovered) as DiscoveredDevice[]);

	/// Every device call is keyed by serial and answers with an error string, so one
	/// wrapper covers Adopt, Identify, and Forget.
	async function act(method: string, device: DiscoveredDevice) {
		busy = device.serial;
		try {
			await client.call(`device.${method}`, { serial: device.serial });
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		} finally {
			busy = null;
		}
	}

	onMount(() => {
		// devices is LOCAL state, not a collection — subscribed by path like session.
		const apply = (v: unknown) => {
			if (v && typeof v === 'object') devices = v as DevicesState;
		};
		const unsub = client.subscribe('devices', apply);
		const doFetch = () => client.get(['devices']).then(apply);
		doFetch();
		const unsubConnect = client.addConnectListener(doFetch);
		return () => { unsub(); unsubConnect(); };
	});
</script>

<div class="panel">
	<div class="panel-header">
		<span class="panel-title">Devices</span>
		{#if !devices.active}
			<span class="badge badge--dim" title="Only the node leading the session drives devices">
				○ Watching
			</span>
		{:else}
			<span class="badge badge--green">● Driving</span>
		{/if}
	</div>

	{#if listed.length === 0}
		<p class="empty-hint">No OpenHaunt nodes on the network.</p>
	{:else}
		<div class="device-list">
			{#each listed as device (device.serial)}
				<div class="device-row" class:offline={!device.online}>
					<div class="device-info">
						<span class="device-name">
							<span class="dot" class:on={device.online}></span>
							{device.name}
						</span>
						<span class="device-meta dim">
							{device.module_name || 'Unknown module'}
							{#if device.caps.length}· {device.caps.join(', ')}{/if}
						</span>
						{#if device.is_mains}
							<span class="mains">⚡ Switches mains voltage</span>
						{/if}
					</div>
					<div class="device-actions">
						{#if device.adopted_fixture_id}
							<button
								class="chip-btn"
								disabled={busy === device.serial || !devices.active}
								onclick={() => act('forget', device)}
							>
								Forget
							</button>
						{:else}
							<button
								class="chip-btn adopt"
								disabled={busy === device.serial || !devices.active}
								onclick={() => act('adopt', device)}
							>
								Adopt
							</button>
						{/if}
						<button
							class="chip-btn"
							title="Blink the node so you can tell which box it is"
							disabled={busy === device.serial || !device.online}
							onclick={() => act('identify', device)}
						>
							Find
						</button>
					</div>
				</div>
			{/each}
		</div>
	{/if}

	{#if devices.broker_addr}
		<p class="hint">Broker <span class="mono">{devices.broker_addr}</span></p>
	{/if}
</div>

<style>
	.panel {
		background: #252525;
		border: 1px solid #333;
		border-radius: 6px;
		padding: 12px 14px;
	}

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 10px;
	}

	.panel-title {
		font-size: 0.68rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #777;
	}

	.badge {
		font-size: 0.68rem;
		font-weight: 500;
		padding: 2px 6px;
		border-radius: 10px;
	}
	.badge--green { background: #14532d44; color: #4ade80; border: 1px solid #14532d; }
	.badge--dim   { background: #2a2a2a;   color: #555;    border: 1px solid #333; }

	.empty-hint {
		font-size: 0.78rem;
		color: #555;
		font-style: italic;
	}

	.device-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.device-row {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 8px;
		padding: 6px 8px;
		background: #1e1e1e;
		border: 1px solid #2e2e2e;
		border-radius: 4px;
	}
	.device-row.offline { opacity: 0.6; }

	.device-info {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.device-name {
		display: flex;
		align-items: center;
		gap: 5px;
		font-size: 0.82rem;
		color: #e0e0e0;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: #555;
		flex-shrink: 0;
	}
	.dot.on { background: #4ade80; }

	.device-meta { font-size: 0.68rem; }
	.dim  { color: #555; }
	.mono { font-family: monospace; }

	.mains {
		font-size: 0.68rem;
		color: #e0a355;
	}

	.device-actions {
		display: flex;
		gap: 4px;
		flex-shrink: 0;
	}

	.chip-btn {
		font-size: 0.68rem;
		padding: 3px 8px;
		border-radius: 3px;
		border: 1px solid #444;
		background: transparent;
		color: #888;
		cursor: pointer;
		white-space: nowrap;
	}
	.chip-btn:hover:not(:disabled) { border-color: #888; color: #ccc; }
	.chip-btn:disabled { cursor: not-allowed; opacity: 0.4; }
	.chip-btn.adopt { border-color: #4a9eff44; color: #4a9eff; }
	.chip-btn.adopt:hover:not(:disabled) { background: #4a9eff22; border-color: #4a9eff; }

	.hint {
		font-size: 0.68rem;
		color: #555;
		margin-top: 8px;
	}
</style>
