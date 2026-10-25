<script lang="ts">
	/**
	 * What the selected fixtures are, editable: the column beside the patch list.
	 *
	 * One fixture shows everything about it — name, type, mode, every break, where it
	 * is and where it rests. Several show what can be said of all of them at once: a
	 * name to number them by, a type and mode to put them all in, and a first address
	 * to lay them out from one after another. What only makes sense of one light — a
	 * position, a home value per parameter — waits until one is picked.
	 *
	 * A type or a mode is not changed here directly: `onask` hands it to the panel's
	 * Save/Cancel dialog, which says which parameters the show's cues would lose.
	 */

	import { untrack } from 'svelte';
	import Trash2 from '@lucide/svelte/icons/trash-2';

	import type { Fixture, FixtureType, ParameterValue } from '$lib/generated/index.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { DEFAULT_MODE, channelRange, dmxAddress, dmxBreaks, fixtureMode, kindLabel, parameterKey } from '$lib/patch.js';
	import { addressRun, namesFor, patchOrder } from '$lib/patching.js';
	import { getDataContext } from '$lib/ws/context.js';
	import Help from '../Help.svelte';
	import HomeValue from '../HomeValue.svelte';
	import PositionEditor from '../PositionEditor.svelte';

	type Props = {
		chosen: Fixture[];
		types: FixtureType[];
		canEdit: boolean;
		spanOf: (fixture: Fixture) => number;
		onask: (fixtures: Fixture[], what: 'mode' | 'type', value: string) => void;
	};
	let { chosen, types, canEdit, spanOf, onask }: Props = $props();

	const data = getDataContext();

	const ordered = $derived(patchOrder(chosen));
	const one = $derived(chosen.length === 1 ? chosen[0] : null);
	const typeIds = $derived([...new Set(chosen.map((f) => f.fixture_type_id))]);
	const sharedType = $derived(typeIds.length === 1 ? types.find((t) => t.id === typeIds[0]) : undefined);
	const modes = $derived([...new Set(chosen.map((f) => fixtureMode(f.address) ?? DEFAULT_MODE))]);
	const allDmx = $derived(chosen.every((f) => dmxAddress(f.address) !== null));
	const first = $derived(ordered.map((f) => dmxAddress(f.address)).find((d) => d !== null) ?? { universe: 1, address: 1 });

	let pattern = $state('');
	let fromUniverse = $state(1);
	let fromAddress = $state(1);
	// What the fields start from follows the *selection* — not every update to the
	// fixtures in it, which would wipe what somebody was half way through typing.
	const selectionKey = $derived(chosen.map((f) => f.id).join());
	$effect(() => {
		void selectionKey;
		untrack(() => {
			fromUniverse = first.universe;
			fromAddress = first.address;
			pattern = '';
		});
	});

	async function rename() {
		const names = namesFor(pattern || 'Fixture {n}', ordered.length);
		beginGesture();
		try {
			for (const [i, fixture] of ordered.entries()) await data.fixtures.byId(fixture.id).name.set(names[i]);
		} finally {
			endGesture();
		}
	}

	/** Lay the selection out one after another from an address, in list order. */
	async function readdress() {
		beginGesture();
		try {
			let universe = fromUniverse;
			let address = fromAddress;
			for (const fixture of ordered) {
				const [placed] = addressRun(universe, address, spanOf(fixture), 1);
				await setFirstBreak(fixture, placed);
				universe = placed.universe;
				address = placed.address + Math.max(1, spanOf(fixture));
			}
		} finally {
			endGesture();
		}
	}

	async function setFirstBreak(fixture: Fixture, next: { universe?: number; address?: number }) {
		const breaks = dmxBreaks(fixture.address);
		if (breaks.length === 0) return;
		const mode = fixtureMode(fixture.address) ?? DEFAULT_MODE;
		await data.fixtures.byId(fixture.id).address.set({ Dmx: { mode, breaks: [{ ...breaks[0], ...next }, ...breaks.slice(1)] } });
	}

	async function setBreak(fixture: Fixture, index: number, next: { universe?: number; address?: number }) {
		const breaks = dmxBreaks(fixture.address);
		if (!breaks[index]) return;
		const mode = fixtureMode(fixture.address) ?? DEFAULT_MODE;
		await data.fixtures.byId(fixture.id).address.set({ Dmx: { mode, breaks: breaks.map((b, i) => (i === index ? { ...b, ...next } : b)) } });
	}

	/**
	 * Set or clear one parameter's home value. The whole map is written, because that
	 * is the shape of the field; `null` clears the override, which is "whatever the
	 * type says" rather than zero.
	 */
	async function setHome(fixture: Fixture, key: string, next: ParameterValue | null) {
		const home = { ...fixture.home_values };
		if (next === null) delete home[key];
		else home[key] = next;
		await data.fixtures.byId(fixture.id).home_values.set(home);
	}

	async function takeHome() {
		beginGesture();
		try {
			for (const fixture of chosen) await data.fixtures.takeHome({ fixtureId: fixture.id });
		} finally {
			endGesture();
		}
	}

	async function unpatch() {
		beginGesture();
		try {
			for (const fixture of chosen) await data.fixtures.byId(fixture.id).delete();
		} finally {
			endGesture();
		}
	}
</script>

<div class="detail">
	{#if chosen.length === 0}
		<p class="empty">Pick a fixture in the list or on the map.</p>
	{:else}
		<header>
			<span class="what">{one ? one.name : `${chosen.length} fixtures`}</span>
			<span class="sub">{sharedType?.name ?? `${typeIds.length} types`}</span>
		</header>

		<!-- Enter commits the field it is pressed in: a field that writes on change is
		     blurred (which is its change), and the two that act through a button — names
		     for several, and laying several out — press that button. -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fields"
			onkeydown={(e) => {
				if (e.key !== 'Enter' || e.isComposing || !(e.target instanceof HTMLInputElement)) return;
				e.preventDefault();
				const input = e.target;
				const button = input.closest('.inline')?.querySelector<HTMLButtonElement>('button.btn');
				if (button && !button.disabled) button.click();
				else input.blur();
			}}
		>
			{#if one}
				<span class="label">Name</span>
				<input
					class="input"
					value={one.name}
					disabled={!canEdit}
					onchange={(e) => data.fixtures.byId(one.id).name.set(e.currentTarget.value)}
				/>
			{:else}
				<span class="label">Names <Help label="Numbering names">Every selected fixture, in list order. <b>{'{n}'}</b> is the number; without it the number goes on the end.</Help></span>
				<div class="inline">
					<input class="input" placeholder="Front wash {'{n}'}" bind:value={pattern} disabled={!canEdit} />
					<button class="btn" disabled={!canEdit || !pattern.trim()} onclick={rename}>Rename</button>
				</div>
			{/if}

			<span class="label">Type</span>
			<select
				class="select"
				disabled={!canEdit}
				value={sharedType?.id ?? ''}
				onchange={(e) => {
					const next = e.currentTarget.value;
					e.currentTarget.value = sharedType?.id ?? '';
					if (next) onask(chosen, 'type', next);
				}}
			>
				{#if !sharedType}<option value="">several</option>{/if}
				{#each types as t (t.id)}
					<option value={t.id}>{t.name}</option>
				{/each}
			</select>

			{#if sharedType && sharedType.dmx_modes.length > 0}
				<span class="label">Mode</span>
				<select
					class="select"
					disabled={!canEdit}
					value={modes.length === 1 ? modes[0] : ''}
					onchange={(e) => {
						const next = e.currentTarget.value;
						e.currentTarget.value = modes.length === 1 ? modes[0] : '';
						if (next) onask(chosen, 'mode', next);
					}}
				>
					{#if modes.length > 1}<option value="">several</option>{/if}
					{#each sharedType.dmx_modes as mode (mode.name)}
						<option value={mode.name}>{mode.name} · {mode.breaks.join('+')} ch</option>
					{/each}
				</select>
			{/if}

			{#if one && dmxAddress(one.address)}
				{@const dmx = dmxAddress(one.address)!}
				<span class="label">Address</span>
				<div class="inline">
					<input
						class="input narrow"
						type="number"
						min="1"
						title="Universe"
						disabled={!canEdit}
						value={dmx.universe}
						onchange={(e) => setFirstBreak(one, { universe: Number(e.currentTarget.value) })}
					/>
					<span class="dot">.</span>
					<input
						class="input narrow"
						type="number"
						min="1"
						max="512"
						title="Address"
						disabled={!canEdit}
						value={dmx.address}
						onchange={(e) => setFirstBreak(one, { address: Number(e.currentTarget.value) })}
					/>
					<span class="note">{channelRange(one, spanOf(one))}</span>
				</div>
				{#each dmxBreaks(one.address).slice(1) as entry, i (i)}
					<span class="label">Break {i + 2}</span>
					<div class="inline">
						<input
							class="input narrow"
							type="number"
							min="1"
							disabled={!canEdit}
							value={entry.universe}
							onchange={(e) => setBreak(one, i + 1, { universe: Number(e.currentTarget.value) })}
						/>
						<span class="dot">.</span>
						<input
							class="input narrow"
							type="number"
							min="1"
							max="512"
							disabled={!canEdit}
							value={entry.address}
							onchange={(e) => setBreak(one, i + 1, { address: Number(e.currentTarget.value) })}
						/>
					</div>
				{/each}
			{:else if !one && allDmx}
				<span class="label">Address <Help label="Laying out several">From this address, one after another in list order, each taking as many channels as its mode. A fixture that would run past 512 starts the next universe.</Help></span>
				<div class="inline">
					<input class="input narrow" type="number" min="1" title="Universe" disabled={!canEdit} bind:value={fromUniverse} />
					<span class="dot">.</span>
					<input class="input narrow" type="number" min="1" max="512" title="Address" disabled={!canEdit} bind:value={fromAddress} />
					<button class="btn" disabled={!canEdit} onclick={readdress}>Lay out</button>
				</div>
			{:else if one}
				<span class="label">Address</span>
				<span class="note">reached through its node, not DMX</span>
			{/if}
		</div>

		{#if one}
			<section>
				<h4>Position</h4>
				{#if canEdit}
					<PositionEditor position={one.position} onchange={(next) => data.fixtures.byId(one.id).position.set(next)} />
				{:else if one.position}
					{@const p = one.position.position}
					<span class="note">{p.x.toFixed(1)}, {p.y.toFixed(1)}, {p.z.toFixed(1)}</span>
				{:else}
					<span class="note">not placed</span>
				{/if}
			</section>
		{/if}

		{#if one && sharedType}
			<section>
				<h4>
					Home <Help label="Where a light rests"
						>Where each parameter goes when nothing is driving it. The type says where by default; a value set here is this unit's own and is outlined. <i>Take from now</i> keeps whatever it is putting out at this moment — aim it, look at it, keep it.</Help
					>
				</h4>
				<div class="homes">
					{#each sharedType.parameters as param (parameterKey(param.kind))}
						{@const key = parameterKey(param.kind)}
						{@const overridden = one.home_values[key]}
						<span class="home" class:overridden={overridden !== undefined}>
							<HomeValue
								label={kindLabel(param.kind)}
								value={overridden ?? param.default_value}
								onchange={(next) => canEdit && setHome(one, key, next)}
							/>
							{#if overridden !== undefined && canEdit}
								<button class="clear" title="Back to what the type says" aria-label="Clear the home value for {kindLabel(param.kind)}" onclick={() => setHome(one, key, null)}>×</button>
							{/if}
						</span>
					{/each}
				</div>
			</section>
		{/if}

		{#if canEdit}
			<footer>
				<button class="btn" onclick={takeHome} title="Keep what {one ? one.name : 'they'} {one ? 'is' : 'are'} putting out now as home">Take home from now</button>
				<button class="btn btn-danger" onclick={unpatch}><Trash2 size={13} />Unpatch{chosen.length > 1 ? ` ${chosen.length}` : ''}</button>
			</footer>
		{/if}
	{/if}
</div>

<style>
	.detail {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 12px;
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-raised);
		min-width: 0;
	}
	header {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.what {
		font-weight: 600;
		font-size: var(--font-md, 14px);
	}
	.sub,
	.note,
	.empty {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.empty {
		margin: 0;
		padding: 12px 0;
		text-align: center;
	}
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
		min-width: 0;
	}
	.inline .input:not(.narrow) {
		flex: 1;
		min-width: 0;
	}
	.input.narrow {
		width: 4.5rem;
	}
	.dot {
		color: var(--text-faint);
	}
	h4 {
		margin: 0 0 6px;
		font-size: var(--font-xs);
		font-weight: 500;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}
	.homes {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.home {
		display: inline-flex;
		align-items: center;
		gap: 2px;
		border-radius: 4px;
	}
	/* This unit's own answer rather than the type's, readable down the list. */
	.home.overridden {
		outline: 1px solid var(--accent);
	}
	.clear {
		background: none;
		border: 0;
		color: inherit;
		cursor: pointer;
		opacity: 0.6;
		padding: 0 2px;
	}
	.clear:hover {
		opacity: 1;
	}
	footer {
		display: flex;
		gap: 6px;
		flex-wrap: wrap;
		justify-content: space-between;
	}
</style>
