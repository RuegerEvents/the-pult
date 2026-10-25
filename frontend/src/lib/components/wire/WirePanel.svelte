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
	 * **And it offers inputs beside them**, through the same `output.watch` with the
	 * input's row id. That is deliberate on the station's side too: an input's viewers
	 * land in the same table an output's do, so a peer's input is watched down the link
	 * with nothing added to either protocol. The two are told apart here by which
	 * collection the id came out of, and nowhere else.
	 *
	 * **What is drawn comes from the connector.** This file knows nothing about
	 * Art-Net, sACN or OpenHaunt. A connector answers with sections in named shapes
	 * and `views.ts` turns a shape into a component, so an output with a viewer of
	 * its own touches that table and nothing here.
	 */

	import { onMount } from 'svelte';

	import type {
		ChannelTest,
		InputConfig,
		InputStatus,
		OutputConfig,
		OutputStatus,
		OutputView
	} from '$lib/generated/index.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { Wire, wireKey } from '$lib/wire.js';
	import { emptyTest, forcedCount, isEmpty } from '$lib/dmxtest.js';
	import { viewFor, type Tester } from './views.js';
	import Help from '../Help.svelte';
	import { collection } from '$lib/stores/show.js';
	import { openSetup } from '$lib/stores/setup.js';

	const fixtures = collection('fixtures');
	/**
	 * DMX universes the rig is patched on that no enabled Art-Net or sACN output
	 * carries. Nothing goes out for them, so there is no wire to show — and saying so
	 * by name beats a picker that simply does not offer them.
	 */
	const uncarried = $derived.by(() => {
		const patched = new Set<number>();
		for (const f of $fixtures) {
			if ('Dmx' in f.address) for (const b of f.address.Dmx.breaks) patched.add(b.universe);
		}
		const dmx = outputs.filter((o) => o.enabled && (o.kind === 'Artnet' || o.kind === 'Sacn'));
		// An output naming no universes carries every universe in the patch.
		if (dmx.some((o) => o.universes.length === 0)) return [];
		const carried = new Set(dmx.flatMap((o) => o.universes));
		return [...patched].filter((u) => !carried.has(u)).sort((a, b) => a - b);
	});

	const client = getClientContext();
	const data = getDataContext();

	let outputs = $state<OutputConfig[]>([]);
	let statuses = $state<Record<string, OutputStatus>>({});
	let inputs = $state<InputConfig[]>([]);
	let inputStatuses = $state<Record<string, InputStatus>>({});
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

	/** One line in the picker, whichever direction it goes. */
	type Watchable = { key: string; id: string; name: string; node: string; incoming: boolean };

	/**
	 * What can be looked at: every enabled output, since a peer's can be asked for, and
	 * every enabled input that some station is listening on.
	 *
	 * An input that names no station is left out rather than offered and silent: nobody
	 * holds a socket for it, so there is nothing for anybody to answer with.
	 */
	const offered = $derived<Watchable[]>([
		...outputs
			.filter((output) => output.enabled)
			.map((output) => ({
				key: keyOf(output),
				id: output.id,
				name: output.name,
				node: station(output),
				incoming: false
			})),
		...inputs
			.filter((input) => input.enabled && input.node_id)
			.map((input) => ({
				key: wireKey(input.node_id as string, input.id),
				id: input.id,
				name: input.name,
				node: input.node_id as string,
				incoming: true
			}))
	]);

	const selected = $derived(offered.find((each) => each.key === watching) ?? null);

	/**
	 * What this browser is forcing on the wire it is looking at.
	 *
	 * **Held only while the page is here and on this wire.** The station keeps it per
	 * session and lets go when the socket closes, and this lets go of everything the
	 * moment the panel moves to another wire or unmounts — a tester that left a light
	 * at full because somebody picked a different output in a dropdown would be a
	 * diagnostic that outlived the person making it. Held in this component, not in a
	 * store, for the same reason.
	 */
	let test = $state<ChannelTest>(emptyTest());
	/** One send per animation frame however many times a slider fires. */
	let sendQueued = false;

	const forcing = $derived(!isEmpty(test));

	function sendTest() {
		sendQueued = false;
		if (!watching) return;
		const [nodeId, outputId] = watching.split(':');
		client.call('output.test', { nodeId, outputId, test }).catch(() => {});
	}

	function setTest(next: ChannelTest) {
		test = next;
		// An empty test is a release and goes at once: it is the message that must not
		// wait for the next frame, which a hidden tab never serves.
		if (isEmpty(next)) return sendTest();
		if (!sendQueued) {
			sendQueued = true;
			requestAnimationFrame(sendTest);
			// And a timer behind it, because a backgrounded tab is served no frames.
			setTimeout(() => sendQueued && sendTest(), 100);
		}
	}

	/** Let go of whatever is forced on the wire being left, before leaving it. */
	function releaseOn(key: string | null) {
		if (!key || isEmpty(test)) return;
		const [nodeId, outputId] = key.split(':');
		client.call('output.test', { nodeId, outputId }).catch(() => {});
		test = emptyTest();
	}

	/** Offered on an output's own wire; an input has nothing to put a level on. */
	const tester = $derived<Tester | undefined>(
		selected && !selected.incoming ? { test, set: setTest } : undefined
	);
	const view = $derived.by((): OutputView | undefined => {
		version;
		return watching ? wire.view(watching) : undefined;
	});

	/** Why there is nothing to look at, when there is nothing to look at. */
	const silence = $derived.by(() => {
		if (!selected) return null;
		if (view) return null;
		const elsewhere = selected.node !== thisStation;
		if (Date.now() - asked_at < 2000) return 'asking…';
		if (elsewhere) return 'That station has not answered. It may be running an older build, or gone.';
		if (selected.incoming) {
			return inputStatuses[selected.id]
				? 'Nothing has arrived on this input yet.'
				: 'This input is not listening on this station.';
		}
		if (!statuses[selected.id]) return 'This output is not running on this station.';
		return 'This connector does not describe what it sends.';
	});

	async function watch(key: string | null, at: string | null) {
		if (watching === key && focus === at) return;
		if (watching && watching !== key) {
			releaseOn(watching);
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
		// While something is forced, say so again every few seconds. The station holds
		// the test and keeps it until this page goes, so on a wire this station runs
		// that is a no-op; on a peer's it is what carries the test across a link that
		// dropped and came back, which forgets everything on the far side.
		const holding = setInterval(() => {
			if (!isEmpty(test)) sendTest();
		}, 3000);
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
		const stopInputs = data.inputs.subscribeDeep((v) => { inputs = v; });
		const applyInputStatus = (v: unknown) => {
			if (v && typeof v === 'object') inputStatuses = v as Record<string, InputStatus>;
		};
		const stopInputStatus = client.subscribe('input_status', applyInputStatus);

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
			client.get(['input_status']).then(applyInputStatus);
			client.get(['session']).then(applySession);
			// A reconnect is a new socket and so a new session: the station forgot
			// what this browser was watching when the old one closed, and nothing
			// re-asks on its behalf.
			if (watching) {
				const [nodeId, outputId] = watching.split(':');
				asked_at = Date.now();
				client.call('output.watch', { nodeId, outputId, focus }).catch(() => {});
				// The new session knows nothing of what the old one was forcing, and
				// the station let go when the old socket closed. Say it again only if
				// there is something to say; it is the operator's, not the station's.
				if (!isEmpty(test)) sendTest();
			}
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);

		return () => {
			clearInterval(holding);
			releaseOn(watching);
			letGo();
			stopOutputs();
			stopInputs();
			stopInputStatus();
			stopStatus();
			stopSession();
			stopTraffic();
			stopConnect();
		};
	});
</script>

<div class="wire">
	<header>
		<h2>
			On the wire
			<Help label="About this panel">
				<p>
					What each output put on its cable, and what each input received. Drawn ten times a
					second while you are looking, and not sent again when nothing has changed — so a
					settled rig holds still rather than going blank.
				</p>
				<p>
					An input shows the <em>merged</em> image: highest priority present, and the highest of
					each slot among equals.
				</p>
			</Help>
		</h2>
		<select
			class="picker"
			value={watching ?? ''}
			onchange={(e) => watch(e.currentTarget.value || null, null)}
		>
			<option value="" disabled hidden>Choose a wire…</option>
			{#each offered as each (each.key)}
				<option value={each.key}>
					{each.incoming ? '← ' : ''}{each.name}
					{#if each.node && each.node !== thisStation}
						· {each.node.slice(0, 8)}…
					{/if}
				</option>
			{/each}
		</select>
	</header>

	{#if forcing}
		<p class="forcing">
			Forcing {forcedCount(test)} {forcedCount(test) === 1 ? 'channel' : 'channels'} on this wire.
			The show is not in control of {forcedCount(test) === 1 ? 'it' : 'them'} until you let go.
			<button class="btn btn-ghost btn-small" onclick={() => setTest(emptyTest())}>
				Release all
			</button>
		</p>
	{/if}

	{#if uncarried.length > 0}
		<p class="uncarried">
			{uncarried.length === 1 ? 'Universe' : 'Universes'} {uncarried.join(', ')}
			{uncarried.length === 1 ? 'is' : 'are'} patched, but no Art-Net or sACN output carries
			{uncarried.length === 1 ? 'it' : 'them'}, so nothing goes out.
			<button class="btn btn-ghost btn-small" onclick={() => openSetup('outputs')}>Add an output</button>
		</p>
	{/if}

	{#if offered.length === 0}
		<p class="empty">
			Nothing is being sent or listened to. Add an output in the I/O panel to put the show on
			a wire, or an input to read somebody else's.
		</p>
	{:else if !selected}
		<p class="empty">Pick a wire to see what is on it.</p>
	{:else if silence}
		<p class="empty">{silence}</p>
	{:else if view}
		{#each view.sections as section, at (at)}
			{@const Section = viewFor(section.body.shape)}
			<section class="block">
				<h3>
					{section.title}
					{#if section.note}<Help label="About {section.title}">{section.note}</Help>{/if}
				</h3>
				<Section
					shape={section.body.shape}
					of={section.body.of}
					focus={view.focus}
					ask={(next: string | null) => watch(watching, next)}
					{tester}
				/>
			</section>
		{/each}
	{/if}
</div>

<style>
	.wire { padding: 16px 20px; }
	header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	h2, h3 { display: flex; align-items: center; gap: 6px; }
	h3 { font-size: 12px; font-weight: 600; color: #ccc; margin: 0 0 6px; }
	.uncarried { color: #e0a060; font-size: 12px; margin: 0 0 12px; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
	.forcing { color: #ffd58a; background: #2a2010; border: 1px solid #5a4416; border-radius: 4px; font-size: 12px; margin: 0 0 12px; padding: 6px 10px; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
	.btn-small { min-height: 24px; padding: 0 10px; font-size: 12px; }
	.picker { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.block { margin-bottom: 20px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
</style>
