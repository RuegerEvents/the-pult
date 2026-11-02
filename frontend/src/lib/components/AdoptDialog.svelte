<script lang="ts">
	/**
	 * Adopting a node: what it is called, and — where a node the show is patched to is
	 * gone — whether this one takes its place.
	 *
	 * Taking a node's place keeps that node's fixtures, ids and all, so every cue, flow,
	 * group and layout that named them drives this one; only the serial moves. The names
	 * start from the old node's in the show, and whatever is here when Adopt is pressed
	 * is written to the show and to the node (`device.adopt` with `names`).
	 */
	import Dialog from './Dialog.svelte';
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { collection } from '$lib/stores/show.js';
	import type { DiscoveredDevice, Fixture, FixtureType } from '$lib/generated/index.js';
	import { NAME_MAX, namesInShow, namesOnNode, tooLong, type AdoptionNames } from '$lib/nodeNames.js';

	let {
		device,
		gone,
		onclose
	}: {
		/** The node being adopted. */
		device: DiscoveredDevice;
		/** Nodes the show is patched to that are not on the network: what it may replace. */
		gone: DiscoveredDevice[];
		onclose: () => void;
	} = $props();

	/** What `device.compare` answers. */
	interface Comparison {
		moduleDiffers: boolean;
		was: string;
		is: string;
		missing: { fixture: string; head: number; key: string; label: string }[];
	}

	const client = getClientContext();
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	/** The serial of the node this one replaces, or empty for a node of its own. */
	let replaces = $state('');
	let names = $state<AdoptionNames>({ name: '', heads: {}, ports: {} });
	let comparison = $state<Comparison | null>(null);
	let busy = $state(false);

	// What the node says, to start from: read once, not followed, so typing is not undone
	// by the next thing the node announces.
	let started = false;
	$effect(() => {
		if (started) return;
		started = true;
		names = namesOnNode(device);
	});

	const ports = $derived(device.description?.ports ?? []);
	const heads = $derived(device.description?.heads ?? []);
	const headName = (head: number) => heads.find((h) => h.head === head)?.name ?? `Head ${head}`;

	async function pick(serial: string) {
		replaces = serial;
		comparison = null;
		const old = gone.find((d) => d.serial === serial);
		names = old
			? namesInShow(old, $fixtures as Fixture[], $types as FixtureType[])
			: namesOnNode(device);
		if (!old) return;
		try {
			comparison = (await client.call('device.compare', { serial: device.serial, replaces: serial })) as Comparison;
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		}
	}

	const long = $derived(
		[names.name, ...Object.values(names.heads), ...Object.values(names.ports)].some((n) => n && tooLong(n))
	);

	async function adopt() {
		if (!names.name.trim()) {
			addToast('A node needs a name');
			return;
		}
		busy = true;
		try {
			// Every port and head, an empty one too: empty clears a label on the node.
			const portNames: Record<number, string> = {};
			for (const p of ports) portNames[p.port] = (names.ports[p.port] ?? '').trim();
			const headNames: Record<number, string> = {};
			for (const h of heads) headNames[h.head] = (names.heads[h.head] ?? '').trim();
			await client.call('device.adopt', {
				serial: device.serial,
				replaces: replaces || undefined,
				names: { name: names.name.trim(), heads: headNames, ports: portNames }
			});
			onclose();
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		} finally {
			busy = false;
		}
	}
</script>

<Dialog title="Adopt {device.name}" size="wide" {onclose} onaccept={adopt}>
	<div class="adopt">
		{#if gone.length > 0}
			<fieldset>
				<legend>Is it a new node, or does it replace one that is gone?</legend>
				<label class="choice">
					<input type="radio" name="replaces" checked={replaces === ''} onchange={() => pick('')} />
					A node of its own
				</label>
				{#each gone as old (old.serial)}
					<label class="choice">
						<input type="radio" name="replaces" checked={replaces === old.serial} onchange={() => pick(old.serial)} />
						Replaces <strong>{old.name}</strong>
						<span class="dim">{old.module_name || 'offline'} · {old.serial}</span>
					</label>
				{/each}
				{#if replaces}
					<p class="note">
						Its fixtures stay, and every cue, flow and layout that uses them drives this node instead.
					</p>
				{/if}
				{#if comparison && (comparison.moduleDiffers || comparison.missing.length > 0)}
					<div class="warn">
						{#if comparison.moduleDiffers}
							<p>It was a {comparison.was}; this is a {comparison.is}.</p>
						{/if}
						{#if comparison.missing.length > 0}
							<p>
								This node does not have {comparison.missing.length === 1 ? 'this port' : 'these ports'}, so whatever
								uses {comparison.missing.length === 1 ? 'it' : 'them'} has nothing left to drive:
							</p>
							<ul>
								{#each comparison.missing as m (`${m.fixture}/${m.key}`)}
									<li>{m.fixture} · {m.label}</li>
								{/each}
							</ul>
						{/if}
					</div>
				{/if}
			</fieldset>
		{/if}

		<div class="names">
			<label class="row">
				<span class="what">Node</span>
				<input bind:value={names.name} maxlength="64" aria-label="Name of the node" />
			</label>
			{#each heads as head (head.head)}
				<label class="row">
					<span class="what">{head.name}</span>
					<input
						value={names.heads[head.head] ?? ''}
						placeholder="{names.name} {head.name}"
						aria-label="Name of {head.name}"
						oninput={(e) => (names.heads = { ...names.heads, [head.head]: e.currentTarget.value })}
					/>
				</label>
			{/each}
			{#each ports as port (port.port)}
				<label class="row">
					<span class="what">
						<span class="mono dim">{port.port}</span>
						{port.name}
						{#if (port.head ?? 0) !== 0}<span class="dim"> · {headName(port.head ?? 0)}</span>{/if}
						<span class="dim"> · {port.access === 'readonly' ? 'input' : 'output'}</span>
					</span>
					<input
						value={names.ports[port.port] ?? ''}
						placeholder={port.name}
						aria-label="Name of {port.name}"
						oninput={(e) => (names.ports = { ...names.ports, [port.port]: e.currentTarget.value })}
					/>
				</label>
			{/each}
		</div>
		<p class="note">
			These are written to the show and to the node{device.online ? '' : ', when it is next online'}. A node keeps
			{NAME_MAX} bytes of each{#if long}<span class="bad"> — some are longer and are cut off on the node</span>{/if}.
		</p>
	</div>
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		<button class="btn btn-primary" disabled={busy} onclick={adopt}>
			{replaces ? 'Replace' : 'Adopt'}
		</button>
	{/snippet}
</Dialog>

<style>
	.adopt {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}
	fieldset {
		border: 1px solid #333;
		border-radius: 4px;
		padding: 8px 10px;
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	legend {
		font-size: 0.75rem;
		color: #aaa;
		padding: 0 4px;
	}
	.choice {
		display: flex;
		align-items: baseline;
		gap: 6px;
		font-size: 0.8rem;
	}
	.names {
		display: flex;
		flex-direction: column;
		gap: 4px;
		max-height: 50vh;
		overflow-y: auto;
	}
	.row {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: 8px;
		align-items: center;
		font-size: 0.78rem;
	}
	.row input {
		background: #1a1a1a;
		border: 1px solid #333;
		border-radius: 3px;
		color: #ddd;
		padding: 3px 6px;
		font: inherit;
	}
	.row input:focus {
		border-color: #4a9eff;
		outline: none;
	}
	.what {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.note {
		margin: 0;
		font-size: 0.72rem;
		color: #888;
	}
	.warn {
		border: 1px solid #a1620744;
		background: #a1620722;
		color: #fbbf24;
		border-radius: 4px;
		padding: 6px 8px;
		font-size: 0.75rem;
	}
	.warn p {
		margin: 0 0 4px;
	}
	.warn ul {
		margin: 0;
		padding-left: 18px;
		max-height: 140px;
		overflow-y: auto;
	}
	.bad {
		color: #f87171;
	}
	.dim {
		color: #666;
	}
	.mono {
		font-family: monospace;
	}
</style>
