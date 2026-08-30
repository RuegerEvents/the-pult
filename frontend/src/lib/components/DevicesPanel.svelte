<script lang="ts">
	import { onMount } from 'svelte';
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { select, selected, toggle } from '$lib/stores/selection.js';
	import { editing } from '$lib/stores/editing.js';
	import OutputGaps from './OutputGaps.svelte';
	import type { DevicesState, DiscoveredDevice } from '$lib/generated/index.js';

	const client = getClientContext();

	// Find and Select stay live: neither changes the show. Adopt patches a fixture
	// and Forget unpatches one, which is what the lock is for.
	const unlocked = editing('devices');

	let devices = $state<DevicesState>({ discovered: {}, broker_addr: null, active: false });
	let busy = $state<string | null>(null);
	/** Which rows have their detail open. Per browser, not show data. */
	let open = $state<Set<string>>(new Set());

	function toggleOpen(serial: string) {
		const next = new Set(open);
		if (!next.delete(serial)) next.add(serial);
		open = next;
	}

	/** Seconds of uptime as something a person reads. */
	function uptime(seconds: number): string {
		if (seconds < 60) return `${seconds}s`;
		if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
		const hours = Math.floor(seconds / 3600);
		return hours < 48 ? `${hours}h ${Math.floor((seconds % 3600) / 60)}m` : `${Math.floor(hours / 24)}d`;
	}

	/**
	 * How long ago a health message arrived.
	 *
	 * Worth showing because a node that has gone quiet still reports its last known
	 * temperature, and "38 °C" from four minutes ago is a different fact from
	 * "38 °C" from four seconds ago.
	 */
	function ago(iso: string): string {
		const seconds = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 1000));
		if (seconds < 90) return `${seconds}s ago`;
		return `${Math.round(seconds / 60)}m ago`;
	}

	const listed = $derived(Object.values(devices.discovered) as DiscoveredDevice[]);

	/// What a node said its terminals are — "8 inputs · 1 output" — or nothing,
	/// for a node that has not described itself.
	///
	/// A console carries no table of module types, so this is the only account of
	/// what a device can do, and it is also what decides whether Adopt is offered.
	function ports(device: DiscoveredDevice): string | null {
		const description = device.description;
		if (!description) return null;
		const inputs = description.ports.filter((p) => p.access === 'readonly').length;
		const outputs = description.ports.length - inputs;
		const parts: string[] = [];
		if (inputs > 0) parts.push(`${inputs} input${inputs === 1 ? '' : 's'}`);
		if (outputs > 0) parts.push(`${outputs} output${outputs === 1 ? '' : 's'}`);
		if (description.dmx) parts.push('forwards a universe');
		return parts.length > 0 ? parts.join(' · ') : null;
	}

	const UNDESCRIBED = 'This node does not describe its ports, so there is nothing to patch.';

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

	/// Select the fixture a node was adopted as: click alone, shift-click to add.
	function pick(event: MouseEvent, device: DiscoveredDevice) {
		const id = device.adopted_fixture_id;
		if (!id) return;
		if (event.shiftKey) toggle(id);
		else select(id);
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

	{#if devices.active}
		<OutputGaps only="OpenHaunt" />
	{/if}

	{#if listed.length === 0}
		<p class="empty-hint">No OpenHaunt nodes on the network.</p>
	{:else}
		<div class="device-list">
			{#each listed as device (device.serial)}
				<div
					class="device-row"
					class:offline={!device.online}
					class:selected={!!device.adopted_fixture_id && $selected.has(device.adopted_fixture_id)}
				>
					<button
						class="disclose"
						aria-expanded={open.has(device.serial)}
						aria-label="Details for {device.name}"
						onclick={() => toggleOpen(device.serial)}
					>{open.has(device.serial) ? '▾' : '▸'}</button>
					<div class="device-info">
						<span class="device-name">
							<span class="dot" class:on={device.online}></span>
							{device.name}
						</span>
						<span class="device-meta dim">
							{device.module_name || 'Unknown module'}
							{#if device.caps.length}· {device.caps.join(', ')}{/if}
						</span>
						{#if ports(device)}
							<span class="device-meta dim">{ports(device)}</span>
						{:else}
							<span class="device-meta undescribed">does not describe its ports</span>
						{/if}
						{#if device.is_mains}
							<span class="mains">⚡ Switches mains voltage</span>
						{/if}
					</div>
					<div class="device-actions">
						{#if device.adopted_fixture_id}
							<button
								class="chip-btn"
								class:on={$selected.has(device.adopted_fixture_id)}
								title="Select its fixture — shift-click to add to the selection"
								onclick={(e) => pick(e, device)}
							>
								Select
							</button>
							{#if $unlocked}
								<button
									class="chip-btn"
									disabled={busy === device.serial || !devices.active}
									onclick={() => act('forget', device)}
								>
									Forget
								</button>
							{/if}
						{:else if $unlocked}
							<button
								class="chip-btn adopt"
								disabled={busy === device.serial || !devices.active || !ports(device)}
								title={ports(device) ? undefined : UNDESCRIBED}
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

					{#if open.has(device.serial)}
						<!-- Everything the node has told this console about itself. Behind a
						     disclosure because it is what you look at when something is
						     wrong, and clutter the rest of the time. -->
						<div class="detail">
							<dl>
								<dt>Address</dt><dd class="mono">{device.ip}:{device.port}</dd>
								<dt>Host</dt><dd class="mono">{device.host}</dd>
								<dt>Firmware</dt><dd>{device.fw || '—'} · protocol {device.protocol_version || '—'}</dd>
								<dt>Module</dt>
								<dd class="mono">
									{device.module_type.toString(16).padStart(4, '0')}
									{#if device.module_serial}· {device.module_serial}{/if}
									{#if device.module_rev}· rev {device.module_rev}{/if}
								</dd>
								{#if device.health}
									{@const h = device.health}
									<dt>Health</dt>
									<dd>
										up {uptime(h.uptime_s)}
										{#if h.temperature_c !== null}· {h.temperature_c.toFixed(1)} °C{/if}
										{#if h.poe_class !== null}· PoE class {h.poe_class}{/if}
										{#if h.reported_at}· {ago(h.reported_at)}{/if}
									</dd>
									{#if h.errors.length > 0}
										<dt>Errors</dt><dd class="bad">{h.errors.join(', ')}</dd>
									{/if}
								{/if}
							</dl>

							{#if device.description?.ports?.length}
								<table class="ports">
									<thead>
										<tr><th>#</th><th>Name</th><th>Flow</th><th>Type</th><th>Can trace</th></tr>
									</thead>
									<tbody>
										{#each device.description.ports as port (port.port)}
											{@const traces = device.effects?.ports?.find((p) => p.port === port.port)}
											<tr>
												<td class="mono">{port.port}</td>
												<td>{port.name}</td>
												<td>{port.access === 'readonly' ? 'read' : 'driven'}</td>
												<td>
													{port.dataType}{#if port.unit}<span class="dim"> {port.unit}</span>{/if}
												</td>
												<td>
													<!-- What the node said it can do for itself. Absent is the
													     default and means the console renders every value. -->
													{#if traces}
														{#each traces.shapes as shape (shape)}
															<span class="cap">{shape}</span>
														{/each}
														{#if traces.steps}<span class="cap">steps</span>{/if}
														{#if traces.transitions}<span class="cap">fades</span>{/if}
													{:else}
														<span class="dim">—</span>
													{/if}
												</td>
											</tr>
										{/each}
									</tbody>
								</table>
							{/if}
						</div>
					{/if}
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

	/* A grid rather than a flex row now that a detail panel has to span the whole
	   width beneath the other three. As a flex row it laid the detail out beside the
	   name and everything overlapped. */
	.device-row {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) auto;
		align-items: start;
		gap: 8px;
		padding: 6px 8px;
		background: #1e1e1e;
		border: 1px solid #2e2e2e;
		border-radius: 4px;
	}
	.device-row.offline { opacity: 0.6; }
	.device-row.selected { background: #1a2a40; }
	.chip-btn.on { border-color: #4a9eff; color: #4a9eff; }

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

	.undescribed {
		color: #6b5a3a;
		font-style: italic;
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
	.disclose {
		background: none;
		border: none;
		color: var(--text-dim);
		font: inherit;
		cursor: pointer;
		padding: 0 6px 0 0;
		align-self: start;
	}

	.detail {
		/* Under all three columns, not beside them. */
		grid-column: 1 / -1;
		border-top: 1px solid var(--line);
		margin-top: 8px;
		padding-top: 8px;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.detail dl {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 3px 12px;
		font-size: 12px;
		margin: 0;
	}
	.detail dt {
		color: var(--text-dim);
	}
	.detail dd {
		margin: 0;
		color: var(--text);
	}
	.detail dd.bad {
		color: var(--bad);
	}

	.ports {
		width: 100%;
		border-collapse: collapse;
		font-size: 12px;
	}
	.ports th {
		text-align: left;
		color: var(--text-dim);
		font-weight: 500;
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding-bottom: 3px;
	}
	.ports td {
		padding: 3px 8px 3px 0;
	}

	/* What a port said it can trace for itself. Absent means the console renders
	   every value and streams it, which is what every node did before. */
	.cap {
		display: inline-block;
		font-size: 10px;
		padding: 0 6px;
		margin: 0 2px 2px 0;
		border-radius: 999px;
		border: 1px solid var(--accent);
		color: var(--accent);
	}
</style>
