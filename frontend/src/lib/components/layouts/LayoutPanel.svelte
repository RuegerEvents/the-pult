<script lang="ts">
	/**
	 * Layouts: drawings of somewhere, with what is happening there painted on them.
	 *
	 * The maze as a few rectangles with every PIR and contact on it, lit while somebody
	 * is on one and fading for half a minute after — so a glance says where the group is
	 * and which way it went. One layout per tile, picked from the cards along the top;
	 * which one is remembered per browser (`lastShownLayout`), the way the timeline panel
	 * remembers its timeline, and each tile then keeps its own pick.
	 *
	 * Built on the drawing itself, with the Show lock open: drag a fixture from the list
	 * beside it (or from the Plan's tray) to place one element per thing it reports, or
	 * add the selection's sensors in one go; anything else comes from + Add. What an
	 * element is, is edited in the Inspector. `/layout/<id>` is the same drawing with
	 * none of the desk round it, for a phone, and presses nothing.
	 */

	import { get } from 'svelte/store';

	import type { Fixture, Layout } from '$lib/generated/index.js';
	import {
		boxOf,
		FIXTURE_MIME,
		layoutUrl,
		newElement,
		NEW_LAYOUT,
		sensingFixtures,
		sensorElements,
		smallest,
		snap,
		TARGET_KINDS,
		type TargetKind
	} from '$lib/layoutView.js';
	import { phoneHosts, phoneLocation } from '$lib/surface.js';
	import { selectSubject } from '$lib/objects.js';
	import { connection } from '$lib/stores/connection.js';
	import { lastShownLayout, layouts, writeElements } from '$lib/stores/layouts.js';
	import { selection } from '$lib/stores/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { surfaceShow } from '$lib/stores/surface.js';
	import { getDataContext, getStationContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import { copyText, uuid } from '$lib/uuid.js';
	import Card from '$lib/components/objects/Card.svelte';
	import Foldered from '$lib/components/objects/Foldered.svelte';
	import Dialog from '$lib/components/Dialog.svelte';
	import NameDialog from '$lib/components/NameDialog.svelte';
	import QrCode from '$lib/components/surface/QrCode.svelte';
	import LayoutDesk from './LayoutDesk.svelte';

	const data = getDataContext();
	const station = getStationContext();
	const networks = collection('station_networks');

	let chosen = $state<string | null>(get(lastShownLayout));
	let creating = $state(false);
	let showLink = $state(false);
	let adding = $state(false);
	let listing = $state(false);

	const layout = $derived<Layout | null>($layouts.find((l) => l.id === chosen) ?? $layouts[0] ?? null);

	function choose(id: string) {
		chosen = id;
		lastShownLayout.set(id);
	}

	// ── The phone link: the surface's, at /layout ────────────────────────────

	const hosts = $derived(phoneHosts($networks.find((n) => n.id === $station?.nodeId)?.interfaces ?? []));
	let host = $state<string | null>(null);
	const chosenHost = $derived(hosts.find((h) => h.address === host)?.address ?? hosts[0]?.address ?? null);
	const onLoopback = $derived(
		typeof window !== 'undefined' && /^(localhost|127\.|\[::1\])/.test(window.location.hostname)
	);
	const link = $derived.by(() => {
		if (!layout || typeof window === 'undefined') return '';
		return layoutUrl(phoneLocation(window.location, chosenHost, $station?.port ?? null), layout.id);
	});

	// ── Making and adding ─────────────────────────────────────────────────────

	async function createLayout(name: string) {
		const id = uuid();
		await data.layouts.create({
			id,
			name,
			...NEW_LAYOUT,
			backdrop: null,
			decay: {},
			elements: [],
			color: null,
			folder: null
		});
		choose(id);
		selectSubject({ kind: 'layout', id });
	}

	const center = (l: Layout) => ({ x: l.width / 2, y: l.height / 2 });

	/** The selection's first fixture, which a new parameter element starts on. */
	const firstSelected = $derived($selection[0] ?? null);

	const available = (kind: TargetKind) =>
		!!layout && newElement(kind, $surfaceShow, center(layout), layout, layout.grid, firstSelected, 'probe') !== null;

	async function add(kind: TargetKind) {
		adding = false;
		if (!layout) return;
		const element = newElement(kind, $surfaceShow, center(layout), layout, layout.grid, firstSelected);
		if (!element) return;
		await writeElements(layout.id, [...layout.elements, element]);
		selectSubject({ kind: 'layout-element', id: element.id });
	}

	/** Where new sensors go: under everything already drawn, or the top when there is no room. */
	function freeRow(l: Layout) {
		const bottom = Math.max(0, ...l.elements.map((e) => {
			const b = boxOf(e);
			return b.y + b.h;
		}));
		const gap = smallest(l.grid);
		const y = l.elements.length ? snap(bottom + gap, l.grid) : gap;
		return { x: gap, y: y + gap < l.height ? y : gap };
	}

	async function placeSensors(fixtures: Fixture[], at?: { x: number; y: number }) {
		if (!layout) return;
		const added = sensorElements(fixtures, $surfaceShow.types, at ?? freeRow(layout), layout, layout.grid);
		if (added.length === 0) {
			addToast(
				fixtures.length === 1
					? `${fixtures[0].name} reports nothing: it has no input parameters to put on a layout.`
					: 'None of those fixtures report anything: they have no input parameters.',
				'info'
			);
			return;
		}
		await writeElements(layout.id, [...layout.elements, ...added]);
	}

	/** Every reported parameter of the selected fixtures, in the selection's order. */
	function addSelection() {
		const fixtures = $selection
			.map((id) => $surfaceShow.fixtures.find((f) => f.id === id))
			.filter((f): f is Fixture => !!f);
		if (fixtures.length === 0) {
			addToast('Nothing is selected. Select fixtures that report something, or drag one from the list.', 'info');
			return;
		}
		void placeSensors(fixtures);
	}

	const sensing = $derived(sensingFixtures($surfaceShow));
</script>

<div class="layout-panel">
	<header class="head">
		<h2>Layouts</h2>
		<span class="spacer"></span>
		{#if layout}
			<button class="btn btn-ghost" onclick={() => (showLink = true)}>Open on a phone</button>
		{/if}
		{#if $canEdit}
			<button class="btn btn-ghost" onclick={() => (creating = true)}>+ Layout</button>
		{/if}
	</header>

	{#if $layouts.length > 0}
		<!-- The layouts as cards: a tap shows one here; the shared menu opens it
		     full-screen, copies its phone link, renames, paints or deletes it. -->
		<div class="cards">
			<Foldered kind="layout" rows={$layouts}>
				{#snippet item(l)}
					<Card
						ref={{ kind: 'layout', id: l.id }}
						name={l.name}
						color={l.color}
						state="{l.elements.length} element{l.elements.length === 1 ? '' : 's'}"
						on={layout?.id === l.id}
						onact={() => choose(l.id)}
					/>
				{/snippet}
			</Foldered>
		</div>
	{/if}

	{#if !layout}
		<p class="empty">
			No layouts. A layout is a drawing of somewhere — a maze, a stage, a building — with what
			is happening there painted on it: every contact and PIR lit while somebody is on it and
			fading after they have gone, a temperature, a count, a timeline running.
			{#if $canEdit}Make one with + Layout.{/if}
		</p>
	{:else}
		{#if $canEdit}
			<div class="tools">
				<span class="menu-anchor">
					<button class="btn btn-primary btn-small" aria-haspopup="menu" aria-expanded={adding} onclick={() => (adding = !adding)}>+ Add</button>
					{#if adding}
						<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
						<div class="catch" onclick={() => (adding = false)}></div>
						<div class="menu" role="menu">
							{#each TARGET_KINDS as k (k.kind)}
								{@const ok = available(k.kind)}
								<button
									role="menuitem"
									disabled={!ok}
									title={ok ? undefined : 'The show has nothing this kind of element could point at yet'}
									onclick={() => add(k.kind)}>{k.label}</button
								>
							{/each}
						</div>
					{/if}
				</span>
				<button class="btn btn-ghost btn-small" onclick={addSelection} title="One element per thing each selected fixture reports">
					Add sensors of the selection{$selection.length ? ` (${$selection.length})` : ''}
				</button>
				<button class="btn btn-ghost btn-small" class:on={listing} aria-pressed={listing} onclick={() => (listing = !listing)}>Sensor list</button>
				<span class="spacer"></span>
				<span class="hint">Drag to move · grips resize · tap the empty canvas for its settings</span>
			</div>
		{/if}

		<div class="work">
			{#if $canEdit && listing}
				<!-- Fixtures that report something, to drag onto the drawing: one element per
				     reported parameter lands where it is dropped. A click adds them below. -->
				<aside class="list" aria-label="Fixtures that report something">
					{#if sensing.length === 0}
						<p class="hint">No fixture reports anything. A sensor node's contacts and readings appear here once it is adopted.</p>
					{/if}
					{#each sensing as fixture (fixture.id)}
						<button
							class="fixture"
							draggable="true"
							title="Drag onto the layout, or click to add below what is there"
							ondragstart={(e) => {
								e.dataTransfer?.setData(FIXTURE_MIME, fixture.id);
								e.dataTransfer?.setData('text/plain', fixture.name);
								if (e.dataTransfer) e.dataTransfer.effectAllowed = 'copy';
							}}
							onclick={() => placeSensors([fixture])}
						>
							{fixture.name}
						</button>
					{/each}
				</aside>
			{/if}
			<LayoutDesk {layout} show={$surfaceShow} live={$connection.connected} />
		</div>
	{/if}
</div>

{#if creating}
	<NameDialog
		title="New layout"
		placeholder="Name, e.g. Maze"
		onsubmit={createLayout}
		onclose={() => (creating = false)}
	/>
{/if}

{#if showLink && layout}
	<Dialog title="Open “{layout.name}” on a phone" onclose={() => (showLink = false)}>
		<section class="link">
			<QrCode text={link} size={168} />
			<div class="link-text">
				<p>Scan it on a phone on the same network as this station, or type:</p>
				<a class="url" href={link} target="_blank" rel="noopener">{link}</a>
				{#if onLoopback && hosts.length > 1}
					<label class="check">
						over
						<select class="input" value={chosenHost} onchange={(e) => (host = e.currentTarget.value)}>
							{#each hosts as h (h.address)}<option value={h.address}>{h.name} — {h.address}</option>{/each}
						</select>
					</label>
				{/if}
				<p class="hint">It opens this layout full-screen, with nothing else of the console around it. It shows; nothing on it can be pressed.</p>
			</div>
		</section>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => copyText(link).then(() => addToast('Link copied', 'success'), (e) => addToast(String(e)))}>Copy link</button>
			<button class="btn btn-primary" onclick={() => (showLink = false)}>Done</button>
		{/snippet}
	</Dialog>
{/if}

<style>
	.layout-panel {
		height: 100%;
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 10px 12px 12px;
		min-height: 0;
	}
	.head,
	.tools {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	.spacer {
		flex: 1;
	}
	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: #999;
	}
	.cards {
		max-height: 30%;
		overflow-y: auto;
		flex-shrink: 0;
	}
	.work {
		flex: 1;
		min-height: 0;
		display: flex;
		gap: 8px;
		border: 1px solid #2a2a2a;
		border-radius: 6px;
		overflow: hidden;
	}
	.list {
		flex: 0 0 170px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 8px;
		border-right: 1px solid #2a2a2a;
		background: #17191d;
	}
	.fixture {
		text-align: left;
		padding: 6px 8px;
		border-radius: 4px;
		border: 1px solid #333;
		background: #1f2227;
		color: #ddd;
		font-size: 12px;
		cursor: grab;
	}
	.fixture:hover {
		border-color: var(--accent, #4a90d9);
	}
	.btn-small {
		min-height: 26px;
		padding: 0 10px;
		font-size: 12px;
	}
	.on {
		border-color: var(--accent, #4a90d9);
		color: #fff;
	}
	.menu-anchor {
		position: relative;
	}
	.catch {
		position: fixed;
		inset: 0;
		z-index: 19;
	}
	.menu {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		z-index: 20;
		min-width: 190px;
		padding: 4px;
		background: #222;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		box-shadow: 0 6px 20px rgb(0 0 0 / 0.4);
		display: flex;
		flex-direction: column;
	}
	.menu button {
		text-align: left;
		padding: 7px 10px;
		border: none;
		border-radius: 4px;
		background: none;
		color: #ddd;
		font-size: 13px;
		cursor: pointer;
	}
	.menu button:hover:not(:disabled) {
		background: #2e2e2e;
	}
	.menu button:disabled {
		color: #555;
		cursor: default;
	}
	.hint {
		color: #777;
		font-size: 12px;
	}
	.empty {
		color: #777;
		font-size: 12px;
		line-height: 1.4;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 5px;
		color: #999;
		font-size: 12px;
	}
	.link {
		display: flex;
		gap: 16px;
		align-items: center;
		flex-wrap: wrap;
	}
	.link-text {
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-width: 200px;
		flex: 1;
		font-size: 13px;
		color: #bbb;
	}
	.url {
		font-size: 18px;
		font-weight: 600;
		color: #8ab4ff;
		overflow-wrap: anywhere;
	}
</style>
