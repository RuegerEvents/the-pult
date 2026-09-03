<script lang="ts">
	/**
	 * What actually leaves this console.
	 *
	 * Not the Outputs panel, which is where an output is configured, and not the
	 * System panel, which says how many bytes went. This is *which* bytes: the sheet
	 * a DMX universe went out as, the messages a node was sent.
	 *
	 * **Opening this panel is `output.watch` and closing it is `output.unwatch`**, so
	 * a console with it shut costs the station nothing. The rules behind that live in
	 * `infra/connectors/viewers.rs`; what matters here is the unwind, which is why
	 * every path out of this component lets go — the teardown below, and a reconnect,
	 * which is a new session the station knows nothing about.
	 *
	 * **The selector offers other stations' outputs**, because only the station
	 * holding a socket can say what went through it: the ask crosses the sync link
	 * and that station's connector answers.
	 *
	 * **What is drawn comes from the connector.** This file knows nothing about
	 * Art-Net, sACN or OpenHaunt. A connector answers with sections in named shapes
	 * and `views.ts` turns a shape into a component, so an output with a viewer of
	 * its own touches that table and nothing here.
	 */

	import { onMount } from 'svelte';

	import type { OutputConfig, OutputStatus, OutputView } from '$lib/generated/index.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { Wire, wireKey } from '$lib/wire.js';
	import { viewFor } from './views.js';

	const client = getClientContext();
	const data = getDataContext();

	let outputs = $state<OutputConfig[]>([]);
	let statuses = $state<Record<string, OutputStatus>>({});
	let thisStation = $state<string | null>(null);

	let wire = $state(new Wire());
	/** Bumped whenever the wire is written to, since it is not itself reactive. */
	let version = $state(0);

	/** Which output this panel is watching, as `${nodeId}:${outputId}`. */
	let watching = $state<string | null>(null);
	/** What it asked to see of it, in the connector's own terms. */
	let focus = $state<string | null>(null);
	/** When the ask went out, so a connector that says nothing can be told from one
	 *  that has not been asked yet. */
	let asked_at = $state(0);

	const station = (output: OutputConfig) => output.node_id ?? thisStation ?? '';
	const keyOf = (output: OutputConfig) => wireKey(station(output), output.id);

	/** Outputs worth offering: everything the show has, since a peer's can be asked for. */
	const offered = $derived(outputs.filter((output) => output.enabled));

	const selected = $derived(offered.find((output) => keyOf(output) === watching) ?? null);
	const view = $derived.by((): OutputView | undefined => {
		version;
		return watching ? wire.view(watching) : undefined;
	});

	/** Why there is nothing to look at, when there is nothing to look at. */
	const silence = $derived.by(() => {
		if (!selected) return null;
		if (view) return null;
		const elsewhere = selected.node_id && selected.node_id !== thisStation;
		if (Date.now() - asked_at < 2000) return 'asking…';
		if (elsewhere) return 'That station has not answered. It may be running an older build, or gone.';
		if (!statuses[selected.id]) return 'This output is not running on this station.';
		return 'This connector does not describe what it sends.';
	});

	async function watch(key: string | null, at: string | null) {
		if (watching === key && focus === at) return;
		if (watching && watching !== key) {
			const [nodeId, outputId] = watching.split(':');
			client.call('output.unwatch', { nodeId, outputId }).catch(() => {});
			wire.forget(watching);
		}
		watching = key;
		focus = at;
		if (!key) return;
		const [nodeId, outputId] = key.split(':');
		asked_at = Date.now();
		client.call('output.watch', { nodeId, outputId, focus: at }).catch(() => {});
	}

	function letGo() {
		if (!watching) return;
		const [nodeId, outputId] = watching.split(':');
		client.call('output.unwatch', { nodeId, outputId }).catch(() => {});
	}

	onMount(() => {
		const stopOutputs = data.outputs.subscribeDeep((v) => {
			outputs = v;
			// The first enabled output, so the panel opens on something rather than on
			// a chooser — a rig usually has one, and looking at it is why anybody
			// opened this.
			if (!watching && v.length > 0) {
				const first = v.find((output) => output.enabled);
				if (first) watch(keyOf(first), null);
			}
		});

		const applyStatus = (v: unknown) => {
			if (v && typeof v === 'object') statuses = v as Record<string, OutputStatus>;
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const stopStatus = client.subscribe('output_status', applyStatus);
		const stopSession = client.subscribe('session', applySession);
		const stopTraffic = client.subscribe('output_traffic', (v: unknown) => {
			if (!v || typeof v !== 'object') return;
			wire.take(v as OutputView);
			version++;
		});

		const fetchLocal = () => {
			client.get(['output_status']).then(applyStatus);
			client.get(['session']).then(applySession);
			// A reconnect is a new socket and so a new session: the station forgot
			// what this browser was watching when the old one closed, and nothing
			// re-asks on its behalf.
			if (watching) {
				const [nodeId, outputId] = watching.split(':');
				asked_at = Date.now();
				client.call('output.watch', { nodeId, outputId, focus }).catch(() => {});
			}
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);

		return () => {
			letGo();
			stopOutputs();
			stopStatus();
			stopSession();
			stopTraffic();
			stopConnect();
		};
	});
</script>

<div class="wire">
	<header>
		<h2>On the wire</h2>
		<select
			class="picker"
			value={watching ?? ''}
			onchange={(e) => watch(e.currentTarget.value || null, null)}
		>
			<option value="">nothing</option>
			{#each offered as output (output.id)}
				<option value={keyOf(output)}>
					{output.name}
					{#if output.node_id && output.node_id !== thisStation}
						· {output.node_id.slice(0, 8)}…
					{/if}
				</option>
			{/each}
		</select>
	</header>

	{#if offered.length === 0}
		<p class="empty">
			Nothing is being sent anywhere. Add an output in the Outputs panel to put the show on a
			wire.
		</p>
	{:else if !selected}
		<p class="empty">Pick an output to see what it is putting on the wire.</p>
	{:else if silence}
		<p class="empty">{silence}</p>
	{:else if view}
		{#each view.sections as section, at (at)}
			{@const Section = viewFor(section.body.shape)}
			<section class="block">
				<h3>{section.title}</h3>
				{#if section.note}<p class="note">{section.note}</p>{/if}
				<Section
					shape={section.body.shape}
					of={section.body.of}
					focus={view.focus}
					ask={(next: string | null) => watch(watching, next)}
				/>
			</section>
		{/each}
		<p class="foot">
			Drawn ten times a second while you are looking, and not sent again when nothing has
			changed — so a settled rig holds still rather than going blank.
		</p>
	{/if}
</div>

<style>
	.wire { padding: 16px 20px; }
	header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	h3 { font-size: 12px; font-weight: 600; color: #ccc; margin: 0 0 6px; }
	.picker { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.block { margin-bottom: 20px; }
	.note { color: #777; font-size: 12px; margin: 0 0 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.foot { color: #666; font-size: 12px; margin-top: 12px; font-style: italic; }
</style>
