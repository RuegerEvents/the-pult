<script lang="ts">
	import { onMount } from 'svelte';
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { select, selected, toggle } from '$lib/stores/selection.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import OutputGaps from './OutputGaps.svelte';
	import StripPatch from './StripPatch.svelte';
	import Dialog from './Dialog.svelte';
	import { bytesText } from '$lib/destination.js';
	import { collection, showData } from '$lib/stores/show.js';
	import type { DevicesState, DiscoveredDevice, Fixture } from '$lib/generated/index.js';

	const client = getClientContext();

	// Find and Select stay live: neither changes the show. Adopt patches a fixture
	// and Forget unpatches one, which is what the lock is for.

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
	const fixtures = collection('fixtures');

	/// The node's own fixture, where its address says whether its stream goes down
	/// every path it is on.
	function ownFixture(device: DiscoveredDevice): Fixture | undefined {
		return ($fixtures as Fixture[]).find((f) => f.id === device.adopted_fixture_id);
	}

	/** Rename a node: its own fixture, which the station passes on to the node. One Ctrl-Z. */
	function rename(device: DiscoveredDevice, value: string) {
		const fixture = ownFixture(device);
		const name = value.trim();
		if (!fixture || !name || name === fixture.name) return;
		if (new TextEncoder().encode(name).length > 31) addToast('A node keeps 31 bytes of its name; the rest is cut off on the node');
		void showData().fixtures.byId(fixture.id).name.set(name);
	}

	function redundant(fixture: Fixture | undefined): boolean {
		return !!fixture && 'OpenHaunt' in fixture.address && fixture.address.OpenHaunt.redundant_stream === true;
	}

	/// One value, and Ctrl-Z takes it back, so it is edited where it is shown.
	async function setRedundant(device: DiscoveredDevice, fixture: Fixture, on: boolean) {
		if (!('OpenHaunt' in fixture.address)) return;
		try {
			await showData()
				.fixtures.byId(fixture.id)
				.address.set({ OpenHaunt: { ...fixture.address.OpenHaunt, redundant_stream: on } });
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		}
	}

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

	/**
	 * Forget asks first, and says what goes with it: every fixture patched to the node
	 * — its heads, a strip with all its cells — is deleted, and cues that set them have
	 * nothing left to set. Ctrl-Z puts the fixtures back; the node is adopted again by
	 * Adopt.
	 */
	let forgetting = $state<DiscoveredDevice | null>(null);
	/**
	 * Taken as an argument and held before the dialog closes: `device` in the dialog is
	 * read off `forgetting`, and clearing that first handed `act` nothing — which is why
	 * Forget did nothing at all.
	 */
	function forget(device: DiscoveredDevice) {
		const held = { ...device };
		forgetting = null;
		void act('forget', held);
	}
	const goingWith = (device: DiscoveredDevice): Fixture[] =>
		($fixtures as Fixture[]).filter((f) => 'OpenHaunt' in f.address && f.address.OpenHaunt.serial === device.serial);

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
			<span
				class="badge badge--dim"
				title="The leading station drives the nodes; Adopt, Forget and strip changes made here are done by it"
			>
				○ The leader drives these
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
							{#if $canEdit && ownFixture(device)}
								<!-- The node is called what its own fixture is called; the station
								     tells the node, now or when it is next up. -->
								<input
									class="rename"
									value={ownFixture(device)?.name ?? device.name}
									aria-label="Name of {device.name}"
									title={device.online ? 'Renames the node too' : 'Renames the node too, when it is next online'}
									onchange={(e) => rename(device, e.currentTarget.value)}
									onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
								/>
							{:else}
								{device.name}
							{/if}
						</span>
						<span class="device-meta dim">
							{device.module_name || 'Unknown module'}
							{#if device.caps.length}· {device.caps.join(', ')}{/if}
						</span>
						<!-- Where it is, in the row and not only behind the disclosure: the first
						     thing anybody standing at a node with a laptop wants. -->
						<span class="device-meta dim mono">
							{#if device.ip}
								{device.ip}{device.net.length > 1 ? ` · ${device.net.map((a) => `${a.ip} ${a.transport}`).join(', ')}` : ''}
							{:else}
								{device.online ? 'address not known yet' : 'offline · not seen since this station started'}
							{/if}
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
							{#if $canEdit}
								<button
									class="chip-btn"
									disabled={busy === device.serial}
									onclick={() => (forgetting = device)}
								>
									Forget
								</button>
							{/if}
						{:else if $canEdit}
							<button
								class="chip-btn adopt"
								disabled={busy === device.serial || !ports(device)}
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

					{#if device.adopted_fixture_id}
						<StripPatch {device} unlocked={$canEdit} />
					{/if}

					{#if device.audio?.stream && ownFixture(device)}
						{@const own = ownFixture(device)!}
						<!-- Every packet down every path the node is on: pulling its cable then
						     costs no sound, at the price of the stream's airtime on Wi-Fi all
						     the time (OpenHaunt docs/13). -->
						{#if $canEdit}
							<label
								class="redundant"
								title="Send the audio stream down every path the node is on, not only the one it prefers"
							>
								<input
									type="checkbox"
									checked={redundant(own)}
									onchange={(e) => setRedundant(device, own, e.currentTarget.checked)}
								/>
								Stream down every path{#if device.net.length > 1}<span class="dim"> · {device.net.length} paths</span>{/if}
							</label>
						{:else if redundant(own)}
							<span class="redundant dim">Stream goes down every path</span>
						{/if}
					{/if}

					{#if open.has(device.serial)}
						<!-- Everything the node has told this console about itself. Behind a
						     disclosure because it is what you look at when something is
						     wrong, and clutter the rest of the time. -->
						<div class="detail">
							<dl>
								<dt>Address</dt><dd class="mono">{device.ip}:{device.port}</dd>
								{#if device.net.length > 0}
									<!-- What the node says on `net`, the path it prefers first. -->
									<dt>Paths</dt>
									<dd class="mono">{device.net.map((a) => `${a.ip} (${a.transport})`).join(', ')}</dd>
								{/if}
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
								{#if device.audio}
									{@const a = device.audio}
									<!-- What the node said its loudspeaker is. An amp output is a
									     bridged speaker drive, never a line out, and says so. -->
									<dt>Sound</dt>
									<dd>
										{a.outputs
											.map((o) => `${o.name} (${o.kind === 'amp' ? 'speaker drive, not line level' : `${o.channels} ch line`})`)
											.join(', ')}
										· {a.voices} voices at {a.rate / 1000} kHz
										· {bytesText(a.clips.bytes_free)} of {bytesText(a.clips.bytes_total)} free
									</dd>
									{#if device.audio_status}
										{@const st = device.audio_status}
										<dt>Playing</dt>
										<dd class:bad={!st.locked || st.underruns > 0}>
											{st.voices.length
												? st.voices.map((v) => v.voice).join(', ')
												: 'nothing'}
											{#if st.stream?.active}· stream {st.stream.depth_ms} ms deep, {st.stream.missing} missing, {st.stream.late} late{/if}
											{#if st.underruns > 0}· {st.underruns} underruns{/if}
											{#if !st.locked}· not locked to the console's clock{/if}
										</dd>
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

{#if forgetting}
	{@const device = forgetting}
	{@const gone = goingWith(device)}
	<Dialog title="Forget {device.name}?" onclose={() => (forgetting = null)} onaccept={() => forget(device)}>
		<div class="prompt">
		{#if gone.length > 0}
			<p>
				{gone.length === 1 ? 'Its fixture is' : `Its ${gone.length} fixtures are`} deleted with it, and every
				cue, group and binding that names {gone.length === 1 ? 'it' : 'them'} has nothing left to drive:
			</p>
			<ul class="gone">
				{#each gone as fixture (fixture.id)}<li>{fixture.name}</li>{/each}
			</ul>
		{:else}
			<p>Nothing is patched to it.</p>
		{/if}
		<p class="note">Ctrl-Z brings the fixtures back. {device.online ? '' : 'It is offline, so it is only forgotten here; it is not told.'}</p>
		</div>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (forgetting = null)}>Cancel</button>
			<button class="btn btn-danger" onclick={() => forget(device)}>Forget</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.prompt {
		padding: 14px 16px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.prompt p {
		margin: 0;
		line-height: 1.5;
	}
	.gone {
		margin: 4px 0 8px;
		padding-left: 18px;
		max-height: 200px;
		overflow-y: auto;
	}
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

	.rename {
		font: inherit;
		color: inherit;
		background: transparent;
		border: 1px solid transparent;
		border-radius: 3px;
		padding: 0 4px;
		min-width: 0;
		flex: 1;
	}
	.rename:hover,
	.rename:focus {
		border-color: var(--line-strong, #3a3a3a);
		background: var(--bg-sunken, #111);
		outline: none;
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

	/* Under the row, across it, like the strips. */
	.redundant {
		grid-column: 1 / -1;
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.68rem;
		color: #888;
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
