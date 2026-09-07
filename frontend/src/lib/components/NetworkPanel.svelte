<script lang="ts">
	/**
	 * Which cable each of this console's services goes out on.
	 *
	 * A panel of its own rather than a section of Settings, because it is two things
	 * at once and the second is why it exists: it is where the six services are set,
	 * and it is the screen that answers *why can the previz not see us* — which is
	 * almost never asked at the console that is broken. So it shows every station's
	 * interfaces and every station's faults, not only this one's.
	 *
	 * Everything here is read from what a station published. Nothing on this page
	 * resolves a name or predicts whether a cable would work: that rule lives in
	 * `pult_schema::types::network` and a second copy of it would disagree with the
	 * station on exactly the cases nobody tests.
	 */

	import { onMount } from 'svelte';

	import type { Station, StationNetwork } from '$lib/generated/index.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { readPreferences, writePreferences } from '$lib/preferences.js';
	import {
		choicesFor,
		faultText,
		faults,
		NETWORK_SERVICES,
		serviceName,
		slotText,
		type NetworkPrefs
	} from '$lib/network.js';
	import { editing } from '$lib/stores/editing.js';

	const data = getDataContext();
	const client = getClientContext();
	const unlocked = editing('network');

	let stations = $state<Station[]>([]);
	/// The cabling, keyed by station id. Its own collection rather than fields on the
	/// station row, because an inventory changes when a cable is plugged in and that
	/// row is a reading taken every two seconds — see `StationNetwork`.
	let networks = $state<Record<string, StationNetwork>>({});
	let thisStation = $state<string | null>(null);
	let prefs = $state<NetworkPrefs | null>(null);
	let trouble = $state<string | null>(null);
	/// Which station's cabling is being looked at. This one to begin with, because
	/// that is whose settings the form writes.
	let looking = $state<string | null>(null);

	const self = $derived(stations.find((s) => s.id === thisStation) ?? null);
	const shown = $derived(stations.find((s) => s.id === looking) ?? self);
	const cabling = $derived(shown ? (networks[shown.id] ?? null) : null);
	const groups = $derived(choicesFor(cabling));
	const isSelf = $derived(shown?.id === thisStation);
	/// An output's binding is the one keyed by a row id rather than by a service.
	const outputs = $derived(
		(cabling?.bindings ?? []).filter((b) => typeof b.service !== 'string')
	);

	onMount(() => {
		const stop = data.stations.subscribeDeep((rows: Station[]) => {
			stations = [...rows].sort((a, b) => a.hostname.localeCompare(b.hostname));
		});
		const stopNetworks = data.station_networks.subscribeDeep((rows: StationNetwork[]) => {
			networks = Object.fromEntries(rows.map((row) => [row.id, row]));
		});
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') {
				thisStation = (v as { node_id: string | null }).node_id;
				looking ??= thisStation;
			}
		};
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => void client.get(['session']).then(applySession);
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);
		void reload();
		return () => {
			stop();
			stopNetworks();
			stopSession();
			stopConnect();
		};
	});

	async function reload() {
		const read = await readPreferences();
		prefs = read?.network ?? {};
	}

	/**
	 * Change one service's cable.
	 *
	 * The whole section goes at once, because *absent* is a value here: a service the
	 * operator has cleared must become "said nothing" rather than keep what it had,
	 * and a field-by-field merge cannot express that.
	 */
	async function set(key: keyof NetworkPrefs, value: string) {
		trouble = null;
		const next: NetworkPrefs = { ...(prefs ?? {}), [key]: value === '' ? null : value };
		const stored = await writePreferences({ network: next });
		if (!stored) {
			trouble = 'This console could not write its settings down.';
			return;
		}
		prefs = stored.network ?? {};
	}
</script>

<div class="panel">
	{#if stations.length > 1}
		<div class="who">
			{#each stations as station (station.id)}
				<button
					class="tab"
					class:on={station.id === looking}
					onclick={() => (looking = station.id)}
				>
					{station.hostname}{station.id === thisStation ? ' · this one' : ''}
					{#if faults(networks[station.id] ?? null) > 0}<span class="dot" title="a service is not on the cable it was told to use"></span>{/if}
				</button>
			{/each}
		</div>
	{/if}

	{#if !shown}
		<p class="empty">No station has reported yet.</p>
	{:else}
		<section>
			<h3>Services</h3>
			{#if !isSelf}
				<!--
					A preference belongs to the machine it is about, and this page talks
					to one station. Rather than pretend otherwise, the form is read-only
					here and says where to go — which is honest, and is also the answer
					to why the interfaces and the faults are on the replicated row: you
					can *see* the other console's problem from here even though you
					cannot set it.
				-->
				<p class="note">
					These are set on {shown.hostname} itself. What it managed is shown below.
				</p>
			{/if}

			<table>
				<thead>
					<tr><th>Service</th><th>Cable</th><th>State</th></tr>
				</thead>
				<tbody>
					{#each NETWORK_SERVICES as service (service.key)}
						<!--
							Art-Net and sACN have no binding of their own, and correctly:
							they are the fallback an output row takes, so what happened to
							them is read on the output rows below rather than here.
						-->
						{@const binding = (cabling?.bindings ?? []).find(
							(b) =>
								typeof b.service === 'string' &&
								b.service.toLowerCase() === service.key.toLowerCase()
						)}
						<tr>
							<td>
								<div class="name">{service.name}</div>
								<div class="what">{service.what}</div>
							</td>
							<td>
								{#if isSelf && $unlocked && prefs}
									<select
										value={prefs[service.key] ?? ''}
										onchange={(e) => set(service.key, e.currentTarget.value)}
									>
										<option value="">Every interface</option>
										<!--
											Grouped and never filtered. The interfaces with no
											address are the adapter ports — exactly where a show
											LAN gets plugged in — and the whole re-resolve rule
											exists so one can be named before it is ready.
										-->
										{#each groups as group (group.label)}
											<optgroup label={group.label}>
												{#each group.choices as choice (choice.value)}
													<option value={choice.value}>
														{choice.label} — {choice.detail}
													</option>
												{/each}
											</optgroup>
										{/each}
									</select>
								{:else}
									<span class="wanted">{prefs?.[service.key] ?? 'Every interface'}</span>
								{/if}
							</td>
							<td>
								{#if binding?.fault}
									<span class="bad">{faultText(binding.fault)}</span>
									{#if service.listener}
										<!--
											The one deliberate exception in the whole
											mechanism, said out loud where it applies: a
											listener falls back rather than refusing,
											because the page is how this gets fixed.
										-->
										<div class="what">listening on every interface instead</div>
									{:else}
										<div class="what">not running until that cable is there</div>
									{/if}
								{:else if binding?.bound}
									<span class="ok">{binding.bound}</span>
								{:else}
									<span class="quiet">every interface</span>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
			{#if trouble}<p class="bad">{trouble}</p>{/if}
		</section>

		<section>
			<h3>Outputs</h3>
			{#if outputs.length === 0}
				<p class="empty">This station is sending nothing.</p>
			{:else}
				<!--
					An output's cable is on its own row rather than here, because the row
					replicates and `en5` means a different cable on every machine. This
					is where what happened is read; the Outputs panel is where it is set.
				-->
				<table>
					<tbody>
						{#each outputs as binding (JSON.stringify(binding.service))}
							<tr>
								<td>{binding.label || serviceName(binding.service)}</td>
								<td><span class="wanted">{binding.wanted ?? 'every interface'}</span></td>
								<td>
									{#if binding.fault}
										<span class="bad">{faultText(binding.fault)}</span>
									{:else if binding.bound}
										<span class="ok">{binding.bound}</span>
									{:else}
										<span class="quiet">every interface</span>
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section>
			<h3>Interfaces on {shown.hostname}</h3>
			{#if !cabling || cabling.interfaces.length === 0}
				<p class="empty">That station has not reported any.</p>
			{:else}
				<table>
					<thead>
						<tr><th>Name</th><th>Addresses</th><th></th></tr>
					</thead>
					<tbody>
						{#each cabling.interfaces as each (each.name)}
							<tr>
								<td>{each.name}</td>
								<td>
									{#if each.addresses.length}
										{each.addresses.join(', ')}
									{:else}
										<span class="quiet">no IPv4 address</span>
									{/if}
								</td>
								<td class="quiet">
									{#if !each.up}down{/if}
									{#if each.loopback}loopback{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section>
			<h3>sACN priority</h3>
			<!--
				Here rather than on the Stations panel because it is a fact about the
				wire: several consoles may send one sACN universe, and this is the
				number their receivers arbitrate on. Art-Net has no such field, which
				is why an unowned Art-Net output goes from the leader alone.
			-->
			<table>
				<tbody>
					{#each stations as station (station.id)}
						<tr>
							<td>{station.hostname}</td>
							<td class="quiet">{slotText(station)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
			<p class="what">
				A station keeps its slot for as long as it is in the session, and remembers it
				across a restart — an sACN receiver changing which source it follows is a
				visible jump on stage.
			</p>
		</section>
	{/if}
</div>

<style>
	.panel {
		padding: 0.75rem;
		overflow: auto;
		height: 100%;
		font-size: 0.85rem;
	}
	section {
		margin-bottom: 1.25rem;
	}
	h3 {
		margin: 0 0 0.4rem;
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		opacity: 0.7;
	}
	table {
		width: 100%;
		border-collapse: collapse;
	}
	th {
		text-align: left;
		font-weight: 500;
		opacity: 0.6;
		padding: 0.25rem 0.5rem 0.25rem 0;
	}
	td {
		padding: 0.35rem 0.5rem 0.35rem 0;
		vertical-align: top;
		border-top: 1px solid rgba(128, 128, 128, 0.2);
	}
	.name {
		font-weight: 500;
	}
	.what,
	.quiet {
		opacity: 0.6;
		font-size: 0.78rem;
	}
	.wanted {
		font-family: ui-monospace, monospace;
	}
	.ok {
		font-family: ui-monospace, monospace;
		color: var(--ok, #3fa66a);
	}
	.bad {
		color: var(--bad, #c8553d);
	}
	.note {
		opacity: 0.7;
		margin: 0 0 0.5rem;
	}
	.empty {
		opacity: 0.6;
	}
	.who {
		display: flex;
		gap: 0.25rem;
		margin-bottom: 0.75rem;
		flex-wrap: wrap;
	}
	.tab {
		background: none;
		border: 1px solid rgba(128, 128, 128, 0.35);
		border-radius: 3px;
		padding: 0.2rem 0.5rem;
		font: inherit;
		color: inherit;
		cursor: pointer;
	}
	.tab.on {
		border-color: currentColor;
	}
	.dot {
		display: inline-block;
		width: 0.45rem;
		height: 0.45rem;
		border-radius: 50%;
		background: var(--bad, #c8553d);
		margin-left: 0.3rem;
	}
	select {
		font: inherit;
		max-width: 22rem;
	}
</style>
