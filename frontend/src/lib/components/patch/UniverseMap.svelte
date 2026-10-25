<script lang="ts">
	/**
	 * One DMX universe as 512 tiles, sixteen rows of thirty-two: what is where, what is
	 * free, and what clashes, at a glance.
	 *
	 * A fixture is drawn in its type's color, a channel two fixtures share in red, and
	 * the selection outlined, so the list beside it and this are the same choice. A tile
	 * dragged moves its fixture — or every selected one in this universe, by the same
	 * number of channels — and the move is written once, on release, as one gesture.
	 * Nothing about a drag reaches the wire until then: a fixture sliding through the
	 * addresses of three others would be three other lights flickering.
	 */

	import type { Fixture } from '$lib/generated/index.js';
	import { dmxAddress } from '$lib/patch.js';
	import { CHANNELS, occupied, shifted, universeCells, universesInUse } from '$lib/patching.js';

	type Props = {
		fixtures: Fixture[];
		spanOf: (fixture: Fixture) => number;
		selected: Set<string>;
		canEdit: boolean;
		/** A tile clicked: its fixture, and whether to add to what is picked. */
		onpick: (id: string, event: MouseEvent | PointerEvent) => void;
		/** A drag let go: where each moved fixture's first break now starts. */
		onmove: (moves: { id: string; universe: number; address: number }[]) => void;
	};
	let { fixtures, spanOf, selected, canEdit, onpick, onmove }: Props = $props();

	const inUse = $derived(universesInUse(fixtures));
	let chosen = $state<number | null>(null);
	/** The universe on show: the one picked, else the selection's, else the first in use. */
	const universe = $derived.by(() => {
		if (chosen !== null) return chosen;
		for (const fixture of fixtures) {
			if (!selected.has(fixture.id)) continue;
			const dmx = dmxAddress(fixture.address);
			if (dmx) return dmx.universe;
		}
		return inUse[0] ?? 1;
	});
	const tabs = $derived([...new Set([...inUse, universe])].sort((a, b) => a - b));

	let drag = $state<{ ids: string[]; from: number; delta: number; moved: boolean } | null>(null);

	/** What is drawn: where things are, or where they would land mid-drag. */
	const shown = $derived.by(() => {
		if (!drag || drag.delta === 0) return fixtures;
		const moving = new Set(drag.ids);
		const delta = drag.delta;
		return fixtures.map((fixture) => {
			if (!moving.has(fixture.id)) return fixture;
			const dmx = dmxAddress(fixture.address);
			if (!dmx || !('Dmx' in fixture.address)) return fixture;
			const [first, ...rest] = fixture.address.Dmx.breaks;
			const address = shifted(dmx.address, spanOf(fixture), delta);
			return { ...fixture, address: { Dmx: { ...fixture.address.Dmx, breaks: [{ ...first, address }, ...rest] } } };
		});
	});
	const cells = $derived(universeCells(shown, universe, spanOf));
	const used = $derived(cells.filter((c) => c.length > 0).length);
	const byId = $derived(new Map(fixtures.map((f) => [f.id, f])));
	const starts = $derived(new Set(occupied(shown, universe, spanOf).map((b) => `${b.id}:${b.start}`)));

	/** A type's color, the same every time: a hue from its id. */
	function hue(fixture: Fixture | undefined): number {
		const id = fixture?.fixture_type_id ?? '';
		let h = 0;
		for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 360;
		return h;
	}

	function title(channel: number, ids: string[]): string {
		if (ids.length === 0) return `${universe}.${String(channel).padStart(3, '0')} — free`;
		const names = ids.map((id) => byId.get(id)?.name ?? '?').join(' and ');
		return `${universe}.${String(channel).padStart(3, '0')} — ${names}${ids.length > 1 ? ' (clash)' : ''}`;
	}

	function channelAt(event: PointerEvent): number | null {
		const el = document.elementFromPoint(event.clientX, event.clientY) as HTMLElement | null;
		const ch = el?.closest<HTMLElement>('[data-ch]')?.dataset.ch;
		return ch ? Number(ch) : null;
	}

	function down(event: PointerEvent, channel: number, ids: string[]) {
		if (event.button !== 0 || ids.length === 0) return;
		const id = ids[ids.length - 1];
		if (!canEdit) {
			onpick(id, event);
			return;
		}
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		// Grabbing a picked fixture takes everything picked in this universe with it;
		// grabbing anything else takes that one alone.
		const together = selected.has(id)
			? fixtures.filter((f) => selected.has(f.id) && dmxAddress(f.address)?.universe === universe).map((f) => f.id)
			: [id];
		drag = { ids: together, from: channel, delta: 0, moved: false };
		if (!selected.has(id)) onpick(id, event);
	}

	function move(event: PointerEvent) {
		if (!drag) return;
		const at = channelAt(event);
		if (at === null) return;
		const delta = at - drag.from;
		if (delta !== drag.delta) drag = { ...drag, delta, moved: true };
	}

	function up(event: PointerEvent, ids: string[]) {
		const was = drag;
		drag = null;
		if (!was) return;
		if (!was.moved || was.delta === 0) {
			if (ids.length && !was.moved) onpick(ids[ids.length - 1], event);
			return;
		}
		const moving = new Set(was.ids);
		onmove(
			fixtures.flatMap((fixture) => {
				if (!moving.has(fixture.id)) return [];
				const dmx = dmxAddress(fixture.address);
				if (!dmx) return [];
				return [{ id: fixture.id, universe: dmx.universe, address: shifted(dmx.address, spanOf(fixture), was.delta) }];
			})
		);
	}
</script>

<div class="map">
	<div class="head">
		<div class="tabs" role="tablist" aria-label="Universe">
			{#each tabs as u (u)}
				<button role="tab" aria-selected={u === universe} class:on={u === universe} onclick={() => (chosen = u)}>{u}</button>
			{/each}
			<button
				class="add"
				title="Show the next universe nothing is in yet"
				aria-label="Next empty universe"
				onclick={() => (chosen = Math.max(0, ...tabs) + 1)}>+</button
			>
		</div>
		<span class="usage">Universe {universe} · {used} / {CHANNELS}</span>
	</div>
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="grid" class:dragging={!!drag} onpointermove={move}>
		{#each cells as ids, i (i)}
			{@const fixture = ids.length ? byId.get(ids[ids.length - 1]) : undefined}
			<div
				class="cell"
				class:taken={ids.length > 0}
				class:clash={ids.length > 1}
				class:picked={ids.some((id) => selected.has(id))}
				class:start={ids.some((id) => starts.has(`${id}:${i + 1}`))}
				class:moving={!!drag && ids.some((id) => drag?.ids.includes(id))}
				style:--h={ids.length ? hue(fixture) : null}
				data-ch={i + 1}
				title={title(i + 1, ids)}
				onpointerdown={(e) => down(e, i + 1, ids)}
				onpointerup={(e) => up(e, ids)}
				onpointercancel={() => (drag = null)}
			></div>
		{/each}
	</div>
</div>

<style>
	.map {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.head {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}
	.tabs {
		display: flex;
		gap: 2px;
		flex-wrap: wrap;
	}
	.tabs button {
		min-width: 28px;
		height: 24px;
		padding: 0 7px;
		border: 1px solid var(--line-strong);
		border-radius: 4px;
		background: var(--bg-raised);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		font-variant-numeric: tabular-nums;
		cursor: pointer;
	}
	.tabs button.on {
		background: var(--accent-solid);
		border-color: var(--accent-solid);
		color: #fff;
	}
	.tabs .add {
		border-style: dashed;
	}
	.usage {
		color: var(--text-faint);
		font-size: var(--font-xs);
		font-variant-numeric: tabular-nums;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(32, minmax(0, 1fr));
		gap: 1px;
		touch-action: none;
		user-select: none;
	}
	/* Square, so the map is as tall as its column is wide allows: sixteen rows. */
	.cell {
		aspect-ratio: 1;
		border-radius: 1px;
		background: #1d1d1d;
	}
	.cell.taken {
		background: hsl(var(--h) 55% 42%);
		cursor: grab;
	}
	/* The first channel of a fixture is brighter, so two of one type side by side
	   still read as two. */
	.cell.taken.start {
		background: hsl(var(--h) 65% 58%);
	}
	.cell.picked {
		box-shadow: inset 0 0 0 1px #fff;
	}
	.cell.clash {
		background: var(--bad);
	}
	.dragging .cell.taken {
		cursor: grabbing;
	}
	.cell.moving {
		opacity: 0.85;
		box-shadow: inset 0 0 0 1px var(--accent);
	}
</style>
