<script lang="ts">
	/**
	 * Patch some lights: one search over every place a fixture type can come from.
	 *
	 * The types already in the show, the generic ones this console ships with, and the
	 * GDTF Share — so nobody has to make a fixture type first and come back. A generic
	 * or Share type is brought into the show by patching it: a generic one as a copy
	 * with its fixed id (`types/generic.rs`, so the second RGBW par finds the first one's
	 * type), a Share one through the same import the Fixture Types panel uses.
	 *
	 * Then how many, what they are called, and where: the next run of free channels in
	 * the universe asked for, until somebody types an address of their own. The fixtures
	 * are written as one gesture, so a wrong patch is one Ctrl-Z. A Share import is its
	 * own, before them, being an import.
	 */

	import { onMount } from 'svelte';
	import Search from '@lucide/svelte/icons/search';

	import type { Fixture, FixtureType } from '$lib/generated/index.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { selectIds } from '$lib/stores/selection.js';
	import { openSetup } from '$lib/stores/setup.js';
	import { showClient } from '$lib/stores/show.js';
	import { userId } from '$lib/stores/user.js';
	import { addToast } from '$lib/toasts.js';
	import { DEFAULT_MODE } from '$lib/patch.js';
	import { CHANNELS, addressRun, freeRun, namesFor, nextNumber, universesInUse } from '$lib/patching.js';
	import { uuid } from '$lib/uuid.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { backendOrigin } from '$lib/ws/endpoint.js';
	import Dialog from '../Dialog.svelte';
	import Help from '../Help.svelte';

	type Props = {
		fixtures: Fixture[];
		types: FixtureType[];
		spanOf: (fixture: Fixture) => number;
		onclose: () => void;
	};
	let { fixtures, types, spanOf, onclose }: Props = $props();

	const data = getDataContext();
	const backend = () => backendOrigin(window.location);

	type Mode = { name: string; width: number };
	type ShareFixture = { rid: number; fixture: string; manufacturer: string; revision: string; modes: { name: string; dmxfootprint: number }[]; uploader: string };
	type Source = 'show' | 'generic' | 'share';
	type Hit = { key: string; source: Source; name: string; detail: string; modes: Mode[]; type?: FixtureType; share?: ShareFixture };

	let query = $state('');
	let only = $state<Source | 'all'>('all');
	let library = $state<FixtureType[]>([]);
	let shareStatus = $state<{ configured: boolean; signedIn: boolean } | null>(null);
	let shareHits = $state<ShareFixture[]>([]);
	let shareBusy = $state(false);
	let shareProblem = $state<string | null>(null);

	let picked = $state<Hit | null>(null);
	let modeName = $state('');
	let count = $state(1);
	let pattern = $state('');
	// Where the rig already is, once: the field is the operator's from here on.
	// svelte-ignore state_referenced_locally
	let universe = $state(universesInUse(fixtures)[0] ?? 1);
	/**
	 * The address field as typed. Empty follows the next free run as the rest changes,
	 * and shows it as the placeholder — so clearing the field to type another number
	 * does not have the next free one put straight back under the cursor.
	 */
	// A number input binds a number, and `null` when it is empty.
	let addressText = $state<number | null>(null);
	const typedAddress = $derived(
		addressText !== null && Number.isInteger(addressText) && addressText >= 1 && addressText <= CHANNELS
			? addressText
			: null
	);
	let busy = $state(false);

	function modesOf(type: FixtureType): Mode[] {
		if (type.dmx_modes.length === 0) return [{ name: DEFAULT_MODE, width: Math.max(1, type.channel_count) }];
		return type.dmx_modes.map((m) => ({ name: m.name, width: Math.max(1, m.breaks[0] ?? 1) }));
	}
	const matches = (text: string) => {
		const q = query.trim().toLowerCase();
		return !q || text.toLowerCase().includes(q);
	};

	const hits = $derived.by((): Hit[] => {
		const inShow = new Set(types.map((t) => t.id));
		const out: Hit[] = [];
		if (only === 'all' || only === 'show') {
			for (const type of types) {
				if (!matches(`${type.manufacturer} ${type.name}`)) continue;
				out.push({ key: `show:${type.id}`, source: 'show', name: type.name, detail: type.manufacturer, modes: modesOf(type), type });
			}
		}
		if (only === 'all' || only === 'generic') {
			for (const type of library) {
				if (inShow.has(type.id) || !matches(type.name)) continue;
				out.push({ key: `generic:${type.id}`, source: 'generic', name: type.name, detail: `${type.channel_count} ch`, modes: modesOf(type), type });
			}
		}
		if (only === 'all' || only === 'share') {
			for (const row of shareHits) {
				out.push({
					key: `share:${row.rid}`,
					source: 'share',
					name: row.fixture,
					detail: `${row.manufacturer} · rev ${row.revision}`,
					modes: row.modes.map((m) => ({ name: m.name, width: Math.max(1, m.dmxfootprint) })),
					share: row
				});
			}
		}
		return out;
	});

	const mode = $derived(picked?.modes.find((m) => m.name === modeName) ?? picked?.modes[0] ?? null);
	const width = $derived(mode?.width ?? 1);
	const need = $derived(width * Math.max(1, count));
	const address = $derived(typedAddress ?? freeRun(fixtures, universe, Math.min(need, CHANNELS), spanOf) ?? 1);
	const run = $derived(addressRun(universe, address, width, Math.max(1, count)));
	const last = $derived(run[run.length - 1]);
	const names = $derived(namesFor(pattern || 'Fixture', Math.max(1, count), nextNumber(fixtures, pattern || 'Fixture')));

	function pick(hit: Hit) {
		picked = hit;
		modeName = hit.modes[0]?.name ?? '';
		pattern = hit.name.replace(/^Generic /, '');
	}

	let shareTimer: ReturnType<typeof setTimeout> | null = null;
	/** The Share is searched on the station, which keeps the list; a pause in typing asks. */
	function searchShare() {
		if (shareTimer) clearTimeout(shareTimer);
		shareProblem = null;
		if (only !== 'all' && only !== 'share') return;
		if (!shareStatus?.configured || query.trim().length < 2) {
			shareHits = [];
			return;
		}
		shareTimer = setTimeout(async () => {
			shareBusy = true;
			try {
				const url = new URL(`${backend()}/api/gdtf-share/search`);
				url.searchParams.set('q', query.trim());
				url.searchParams.set('limit', '30');
				const answer = await fetch(url);
				if (!answer.ok) {
					shareHits = [];
					shareProblem = await answer.text();
				} else shareHits = (await answer.json()).fixtures ?? [];
			} catch (e) {
				shareProblem = String(e);
			} finally {
				shareBusy = false;
			}
		}, 300);
	}

	onMount(() => {
		void showClient()
			.call('fixtureTypes.library', {})
			.then((answer) => (library = (answer as { types: FixtureType[] }).types));
		void fetch(`${backend()}/api/gdtf-share/status`)
			.then((r) => r.json())
			.then((s) => (shareStatus = s))
			.catch(() => (shareStatus = null));
		return () => {
			if (shareTimer) clearTimeout(shareTimer);
		};
	});

	/** The type to patch, in the show: as it is, copied in, or imported from the Share. */
	async function typeInShow(hit: Hit): Promise<FixtureType | null> {
		if (hit.source === 'show') return hit.type!;
		if (hit.source === 'generic') {
			const existing = types.find((t) => t.id === hit.type!.id);
			if (existing) return existing;
			// A snapshot, not the reactive proxy the library is held in: the socket
			// serializes what it is given, and a proxy is not a plain object.
			await data.fixture_types.create($state.snapshot(hit.type!) as FixtureType);
			return hit.type!;
		}
		const answer = await fetch(`${backend()}/api/gdtf-share/import?rid=${hit.share!.rid}`, {
			method: 'POST',
			headers: { 'x-pult-user': $userId }
		});
		if (!answer.ok) throw new Error(await answer.text());
		const id: string = (await answer.json()).fixture_type_id;
		// The row arrives over the socket; wait for it rather than guess at its modes.
		for (let i = 0; i < 50; i++) {
			const arrived = types.find((t) => t.id === id);
			if (arrived) return arrived;
			await new Promise((r) => setTimeout(r, 100));
		}
		throw new Error('The import finished but the fixture type did not arrive.');
	}

	async function patch() {
		if (!picked || busy) return;
		busy = true;
		try {
			const type = await typeInShow(picked);
			if (!type) return;
			// A Share mode is named by the file; if the imported type calls it something
			// else, its first mode is the honest fallback.
			const modeInType = type.dmx_modes.find((m) => m.name === modeName)?.name ?? type.dmx_modes[0]?.name ?? DEFAULT_MODE;
			const made: string[] = [];
			// Taken *before* the first write, and the reason is not obvious: `names` is
			// worked out from the fixtures already patched ("the first number after the
			// highest"), so every fixture created in this loop moves it. Read live, five
			// dimmers came out as Dimmer 1, 2, 4, 8 and 9 — each name was worked out
			// after the previous row had landed. Likewise the places, which depend on what
			// is free.
			const planned = [...names];
			const places = [...run];
			beginGesture();
			try {
				for (const [i, place] of places.entries()) {
					const id = uuid();
					await data.fixtures.create({
						id,
						name: planned[i],
						fixture_type_id: type.id,
						address: { Dmx: { mode: modeInType, breaks: [{ universe: place.universe, address: place.address }] } },
						// In nobody's drawing until somebody places it, nothing reported and
						// nothing driving it: the station's to fill in.
						position: null,
						parent: null,
						mount: null,
						layer: null,
						class: null,
						focus: null,
						fixture_number: null,
						unit_number: null,
						sensed_values: {},
						live_effects: {},
						live_fades: {},
						live_htp: {},
						home_values: {}
					});
					made.push(id);
				}
			} finally {
				endGesture();
			}
			selectIds(made);
			addToast(`Patched ${made.length} × ${type.name}.`, 'success');
			onclose();
		} catch (e) {
			console.error('[patch] adding fixtures failed', e);
			addToast(`Could not patch: ${e instanceof Error ? e.message : String(e)}`);
		} finally {
			busy = false;
		}
	}

	const SOURCES: [Source | 'all', string][] = [
		['all', 'All'],
		['show', 'In this show'],
		['generic', 'Generic'],
		['share', 'GDTF Share']
	];
	const BADGE: Record<Source, string> = { show: 'In show', generic: 'Generic', share: 'Share' };
</script>

<Dialog title="Add fixtures" size="wide" {onclose} onaccept={patch}>
	<div class="body">
		<div class="find">
			<label class="search">
				<Search size={14} />
				<!-- svelte-ignore a11y_autofocus -->
				<input placeholder="Search types, generic and the GDTF Share…" bind:value={query} oninput={searchShare} autofocus />
			</label>
			<div class="segmented" role="group" aria-label="Where from">
				{#each SOURCES as [key, label] (key)}
					<button class:on={only === key} onclick={() => ((only = key), searchShare())}>{label}</button>
				{/each}
			</div>
			<div class="hits" role="listbox" aria-label="Fixture types">
				{#each hits as hit (hit.key)}
					<button role="option" aria-selected={picked?.key === hit.key} class:on={picked?.key === hit.key} onclick={() => pick(hit)} ondblclick={() => (pick(hit), patch())}>
						<span class="name">{hit.name}<small>{hit.detail}</small></span>
						<span class="badge {hit.source}">{hit.source === 'share' && hit.share?.uploader === 'Manuf.' ? 'Manuf.' : BADGE[hit.source]}</span>
					</button>
				{:else}
					<p class="none">{query.trim() ? 'Nothing matched.' : 'Nothing here yet.'}</p>
				{/each}
				{#if only === 'all' || only === 'share'}
					{#if shareStatus && !shareStatus.configured}
						<p class="share-note">
							The GDTF Share needs a login on this station.
							<button class="link" onclick={() => openSetup('fixturetypes')}>Sign in…</button>
						</p>
					{:else if shareBusy}
						<p class="share-note">Searching the GDTF Share…</p>
					{:else if shareProblem}
						<p class="share-note bad">{shareProblem}</p>
					{:else if shareStatus?.configured && query.trim().length < 2}
						<p class="share-note">Type two letters to search the GDTF Share too.</p>
					{/if}
				{/if}
			</div>
		</div>

		<!-- Enter anywhere in the fields patches, the way a dialog's default button does;
		     a select keeps its own Enter, which opens it. -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fields how"
			onkeydown={(e) => {
				if (e.key !== 'Enter' || e.isComposing || !picked || busy || count < 1) return;
				if (e.target instanceof HTMLSelectElement || e.target instanceof HTMLButtonElement) return;
				e.preventDefault();
				void patch();
			}}
		>
			{#if picked}
				<span class="label">Type</span>
				<span class="chosen">{picked.name}</span>
				{#if picked.modes.length > 1}
					<span class="label">Mode</span>
					<select class="select" bind:value={modeName}>
						{#each picked.modes as m (m.name)}
							<option value={m.name}>{m.name} · {m.width} ch</option>
						{/each}
					</select>
				{/if}
				<span class="label">How many</span>
				<input class="input narrow" type="number" min="1" max="512" bind:value={count} />
				<span class="label">Names <Help label="Naming them">Numbered from after the highest one already patched with this name. <b>{'{n}'}</b> puts the number somewhere other than the end.</Help></span>
				<input class="input" bind:value={pattern} />
				<span class="label">Universe</span>
				<input class="input narrow" type="number" min="1" bind:value={universe} oninput={() => (addressText = null)} />
				<span class="label">Address</span>
				<div class="inline">
					<input
						class="input narrow"
						type="number"
						min="1"
						max="512"
						placeholder={String(address)}
						bind:value={addressText}
					/>
					{#if typedAddress !== null}
						<button class="link" onclick={() => (addressText = null)}>next free</button>
					{:else}
						<span class="note">next free</span>
					{/if}
				</div>
				<span></span>
				<p class="summary">
					{count} × {width} = {need} channels · {universe}.{String(address).padStart(3, '0')} – {last.universe}.{String(last.address + width - 1).padStart(3, '0')}
					{#if last.universe !== universe}<br /><span class="note">runs on into universe {last.universe}</span>{/if}
					<br /><span class="note">{names[0]}{names.length > 1 ? ` … ${names[names.length - 1]}` : ''}</span>
				</p>
			{:else}
				<p class="pick-one">Pick a type on the left.</p>
			{/if}
		</div>
	</div>
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Cancel</button>
		<button class="btn btn-primary" disabled={!picked || busy || count < 1} onclick={patch}
			>{busy ? 'Patching…' : `Patch ${Math.max(1, count)}`}</button
		>
	{/snippet}
</Dialog>

<style>
	.body {
		display: grid;
		grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
		gap: 16px;
		min-height: 360px;
	}
	@media (max-width: 640px) {
		.body {
			grid-template-columns: 1fr;
		}
	}
	.find {
		display: flex;
		flex-direction: column;
		gap: 8px;
		min-width: 0;
	}
	.search {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 0 10px;
		min-height: 34px;
		border: 1px solid var(--line-input);
		border-radius: var(--radius);
		background: var(--bg-input, #0e0e0e);
		color: var(--text-faint);
	}
	.search input {
		flex: 1;
		min-width: 0;
		border: none;
		background: none;
		color: var(--text);
		font: inherit;
		outline: none;
	}
	.hits {
		display: flex;
		flex-direction: column;
		gap: 2px;
		max-height: 340px;
		overflow-y: auto;
	}
	.hits button {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		padding: 7px 9px;
		border: 1px solid transparent;
		border-radius: 5px;
		background: none;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		cursor: pointer;
	}
	.hits button:hover {
		background: var(--bg-hover);
	}
	.hits button.on {
		background: color-mix(in srgb, var(--accent) 16%, transparent);
		border-color: var(--accent-solid);
	}
	.name {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.name small {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.badge {
		flex: none;
		padding: 1px 7px;
		border-radius: 9px;
		font-size: 10px;
		background: var(--bg-raised);
		color: var(--text-dim);
	}
	.badge.share {
		background: color-mix(in srgb, var(--good) 22%, transparent);
		color: var(--good);
	}
	.badge.show {
		background: color-mix(in srgb, var(--accent) 22%, transparent);
		color: var(--accent);
	}
	.none,
	.share-note,
	.pick-one {
		margin: 6px 4px;
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.share-note.bad {
		color: var(--bad);
	}
	.how {
		align-content: start;
	}
	.chosen {
		font-weight: 600;
	}
	.inline {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.input.narrow {
		width: 5rem;
	}
	.note {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.summary {
		margin: 4px 0 0;
		font-size: var(--font-xs);
		color: var(--text-dim);
		font-variant-numeric: tabular-nums;
		line-height: 1.5;
	}
	.link {
		background: none;
		border: none;
		padding: 0;
		color: var(--accent);
		font: inherit;
		font-size: var(--font-xs);
		text-decoration: underline;
		cursor: pointer;
	}
</style>
