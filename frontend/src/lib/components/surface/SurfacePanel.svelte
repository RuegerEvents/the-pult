<script lang="ts">
	/**
	 * Surfaces: the pages a phone runs the show from, made here.
	 *
	 * The surface as a phone sees it — pressable, so a desk can run the maze too — and
	 * the link to open it on one, beside the list of tiles to build it out of. Under the
	 * Show lock only the first half is drawn. A surface is show data and the phone view is
	 * `routes/surface/[[id]]`, which draws the same `SurfaceTiles` with no workspace
	 * around it.
	 *
	 * Every edit rewrites the tile list, one write inside one gesture, so one change is
	 * one Ctrl-Z however many fields of a tile it touched.
	 */

	import { focusOnMount } from '$lib/actions.js';
	import type {
		ParameterKind,
		Surface,
		SurfaceTile,
		TileTarget,
		VariableOp
	} from '$lib/generated/index.js';
	import {
		TILE_KINDS,
		defaultTarget,
		flowButtons,
		moveTile,
		newTile,
		patchTile,
		phoneHosts,
		phoneLocation,
		removeTile,
		surfaceUrl,
		tileLabel,
		type TileKind
	} from '$lib/surface.js';
	import { displayLabel, parameterKey } from '$lib/patch.js';
	import { collection } from '$lib/stores/show.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { surfaceShow } from '$lib/stores/surface.js';
	import { getDataContext, getStationContext } from '$lib/ws/context.js';
	import SurfaceTiles from './SurfaceTiles.svelte';
	import Card from '$lib/components/objects/Card.svelte';
	import QrCode from './QrCode.svelte';

	const data = getDataContext();
	const station = getStationContext();
	const surfaces = collection('surfaces');
	const networks = collection('station_networks');

	let chosen = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let adding = $state<TileKind>('VariableReadout');
	let problem = $state<string | null>(null);
	let showLink = $state(false);

	const surface = $derived<Surface | null>(
		$surfaces.find((s) => s.id === chosen) ?? $surfaces[0] ?? null
	);

	/** The addresses a phone could use, and which one this desk picked. */
	const hosts = $derived(
		phoneHosts($networks.find((n) => n.id === $station?.nodeId)?.interfaces ?? [])
	);
	let host = $state<string | null>(null);
	const chosenHost = $derived(hosts.find((h) => h.address === host)?.address ?? hosts[0]?.address ?? null);
	const onLoopback = $derived(
		typeof window !== 'undefined' && /^(localhost|127\.|\[::1\])/.test(window.location.hostname)
	);

	/** Where a phone on this network reaches the page. */
	const link = $derived.by(() => {
		if (!surface || typeof window === 'undefined') return '';
		return surfaceUrl(phoneLocation(window.location, chosenHost, $station?.port ?? null), surface.id);
	});

	async function write(tiles: SurfaceTile[]) {
		if (!surface) return;
		problem = null;
		beginGesture();
		try {
			await data.surfaces.byId(surface.id).tiles.set(tiles);
		} catch (e) {
			problem = e instanceof Error ? e.message : String(e);
		} finally {
			endGesture();
		}
	}

	const patch = (tile: SurfaceTile, change: Partial<SurfaceTile>) =>
		surface && write(patchTile(surface.tiles, tile.id, change));
	const retarget = (tile: SurfaceTile, target: TileTarget) => patch(tile, { target });

	async function createSurface() {
		const name = newName.trim();
		if (!name) return;
		const id = crypto.randomUUID();
		await data.surfaces.create({ id, name, tiles: [], color: null });
		chosen = id;
		newName = '';
		creating = false;
	}

	async function addTile() {
		if (!surface) return;
		const tile = newTile(adding, $surfaceShow);
		if (!tile) {
			problem = `The show has nothing a ${TILE_KINDS.find((k) => k.kind === adding)?.label.toLowerCase()} tile could point at yet.`;
			return;
		}
		await write([...surface.tiles, tile]);
	}

	/** Changing a tile's kind starts it on the first thing of that kind. */
	function changeKind(tile: SurfaceTile, kind: TileKind) {
		const target = defaultTarget(kind, $surfaceShow);
		if (!target) {
			problem = 'The show has nothing that kind of tile could point at yet.';
			return;
		}
		patch(tile, { target, label: kind === 'Heading' ? tile.label || 'Heading' : tile.label });
	}

	// ── The op of a setter, as a select and an amount ────────────────────────

	const opTag = (op: VariableOp): string =>
		typeof op === 'string' ? op : 'Set' in op ? 'Set' : op.Add >= 0 ? 'Up' : 'Down';
	const opAmount = (op: VariableOp): number =>
		typeof op === 'string' ? 1 : 'Set' in op ? op.Set : Math.abs(op.Add);
	function opFrom(tag: string, amount: number): VariableOp {
		if (tag === 'Toggle' || tag === 'Reset') return tag;
		if (tag === 'Set') return { Set: amount };
		return { Add: tag === 'Up' ? amount : -amount };
	}

	function parametersOf(fixtureId: string) {
		const fixture = $surfaceShow.fixtures.find((f) => f.id === fixtureId);
		const type = fixture && $surfaceShow.types.find((t) => t.id === fixture.fixture_type_id);
		return type?.parameters ?? [];
	}

	function pickParameter(tile: SurfaceTile, fixtureId: string, key?: string) {
		const parameters = parametersOf(fixtureId);
		const chosenParameter = parameters.find((p) => parameterKey(p.kind) === key) ?? parameters[0];
		if (!chosenParameter) return;
		retarget(tile, { kind: 'Parameter', fixture_id: fixtureId, parameter: chosenParameter.kind as ParameterKind });
	}
</script>

<div class="surface-panel">
	<header class="head">
		<h2>Surfaces</h2>
		<span class="spacer"></span>
		{#if surface}
			<button class="btn btn-ghost" onclick={() => (showLink = !showLink)}>
				{showLink ? 'Hide link' : 'Open on a phone'}
			</button>
		{/if}
		{#if $canEdit}
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Surface'}
			</button>
		{/if}
	</header>

	{#if creating && $canEdit}
		<form class="row" onsubmit={(e) => { e.preventDefault(); createSurface(); }}>
			<input class="input" placeholder="Name, e.g. Maze entrance" bind:value={newName} use:focusOnMount />
			<button class="btn btn-primary" type="submit">Create</button>
		</form>
	{/if}

	{#if problem}<p class="problem">{problem}</p>{/if}

	{#if $surfaces.length > 0}
		<!-- The surfaces as cards: a tap shows one here, and the shared menu opens it
		     full-screen, copies its phone link, renames, paints or deletes it. -->
		<div class="boards">
			{#each $surfaces as s (s.id)}
				<Card
					ref={{ kind: 'surface', id: s.id }}
					name={s.name}
					color={s.color}
					state="{s.tiles.length} tile{s.tiles.length === 1 ? '' : 's'}"
					on={surface?.id === s.id}
					onact={() => (chosen = s.id)}
				/>
			{/each}
		</div>
	{/if}

	{#if !surface}
		<p class="empty">
			No surfaces. A surface is a page of big buttons and readouts for a phone — an area's
			occupancy, a reset, the entrance light, the ambience on and off — built out of the
			show's variables, flow buttons, timelines and sequences.
		</p>
	{:else}
		{#if showLink}
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
					<p class="hint">It opens this surface full-screen, with nothing else of the console around it.</p>
				</div>
			</section>
		{/if}

		<div class="work">
			{#if $canEdit}
				<div class="build">
					<div class="row">
						<input
							class="input grow"
							value={surface.name}
							aria-label="Surface name"
							onchange={(e) => data.surfaces.byId(surface.id).name.set(e.currentTarget.value)}
						/>
						<button
							class="btn btn-danger"
							onclick={() => {
								const id = surface.id;
								chosen = null;
								data.surfaces.byId(id).delete();
							}}
						>Delete surface</button>
					</div>

					<ol class="editor">
						{#each surface.tiles as tile, index (tile.id)}
							{@const target = tile.target}
							<li class="tile-row">
								<div class="line">
									<select
										class="input"
										value={target.kind}
										aria-label="Kind"
										onchange={(e) => changeKind(tile, e.currentTarget.value as TileKind)}
									>
										{#each TILE_KINDS as k (k.kind)}<option value={k.kind}>{k.label}</option>{/each}
									</select>
									<input
										class="input grow"
										placeholder={tileLabel({ ...tile, label: '' }, $surfaceShow)}
										value={tile.label}
										aria-label="Label"
										onchange={(e) => patch(tile, { label: e.currentTarget.value })}
									/>
									<button class="btn btn-icon" title="Up" disabled={index === 0} onclick={() => write(moveTile(surface.tiles, index, index - 1))}>↑</button>
									<button class="btn btn-icon" title="Down" disabled={index === surface.tiles.length - 1} onclick={() => write(moveTile(surface.tiles, index, index + 1))}>↓</button>
									<button class="btn btn-danger btn-icon" title="Remove" onclick={() => write(removeTile(surface.tiles, tile.id))}>×</button>
								</div>

								<div class="line">
									{#if target.kind === 'FlowButton'}
										<select class="input" value={target.node_id} onchange={(e) => retarget(tile, { kind: 'FlowButton', node_id: e.currentTarget.value })}>
											{#each flowButtons($surfaceShow) as b (b.id)}<option value={b.id}>{b.name}</option>{/each}
										</select>
									{:else if target.kind === 'VariableReadout' || target.kind === 'VariableSetter'}
										<select
											class="input"
											value={target.variable_id}
											onchange={(e) => retarget(tile, { ...target, variable_id: e.currentTarget.value })}
										>
											{#each $surfaceShow.variables as v (v.id)}<option value={v.id}>{v.name}</option>{/each}
										</select>
										{#if target.kind === 'VariableSetter'}
											<select
												class="input"
												value={opTag(target.op)}
												onchange={(e) => retarget(tile, { ...target, op: opFrom(e.currentTarget.value, opAmount(target.op)) })}
											>
												<option value="Up">add</option>
												<option value="Down">subtract</option>
												<option value="Set">set to</option>
												<option value="Toggle">toggle</option>
												<option value="Reset">reset</option>
											</select>
											{#if opTag(target.op) === 'Up' || opTag(target.op) === 'Down' || opTag(target.op) === 'Set'}
												<input
													class="input narrow"
													type="number"
													value={opAmount(target.op)}
													aria-label="Amount"
													onchange={(e) => retarget(tile, { ...target, op: opFrom(opTag(target.op), e.currentTarget.valueAsNumber || 0) })}
												/>
											{/if}
										{/if}
									{:else if target.kind === 'Timeline'}
										<select class="input" value={target.timeline_id} onchange={(e) => retarget(tile, { ...target, timeline_id: e.currentTarget.value })}>
											{#each $surfaceShow.timelines as t (t.id)}<option value={t.id}>{t.name}</option>{/each}
										</select>
										<label class="check">
											<input
												type="checkbox"
												checked={target.from_start}
												onchange={(e) => retarget(tile, { ...target, from_start: e.currentTarget.checked })}
											/>
											play from the top
										</label>
									{:else if target.kind === 'SequenceGo' || target.kind === 'SequenceOff'}
										<select class="input" value={target.sequence_id} onchange={(e) => retarget(tile, { ...target, sequence_id: e.currentTarget.value })}>
											{#each $surfaceShow.sequences as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
										</select>
									{:else if target.kind === 'Parameter'}
										<select class="input" value={target.fixture_id} onchange={(e) => pickParameter(tile, e.currentTarget.value)}>
											{#each $surfaceShow.fixtures as f (f.id)}<option value={f.id}>{f.name}</option>{/each}
										</select>
										<select
											class="input"
											value={parameterKey(target.parameter)}
											onchange={(e) => pickParameter(tile, target.fixture_id, e.currentTarget.value)}
										>
											{#each parametersOf(target.fixture_id) as p (parameterKey(p.kind))}
												<option value={parameterKey(p.kind)}>{displayLabel(p.kind)}</option>
											{/each}
										</select>
									{/if}

									{#if target.kind !== 'Heading'}
										<select
											class="input"
											value={tile.size}
											aria-label="Width"
											onchange={(e) => patch(tile, { size: e.currentTarget.value as SurfaceTile['size'] })}
										>
											<option value="Half">half width</option>
											<option value="Full">full width</option>
										</select>
										<label class="check" title="The tile's own color">
											color
											<input
												type="color"
												value={tile.color ?? '#2d3a4f'}
												onchange={(e) => patch(tile, { color: e.currentTarget.value })}
											/>
										</label>
										{#if tile.color}
											<button class="btn btn-ghost" onclick={() => patch(tile, { color: null })}>no color</button>
										{/if}
									{/if}
								</div>

								{#if target.kind === 'VariableReadout'}
									<div class="line stops">
										<span class="hint">Color from a value up:</span>
										{#each target.stops as stop, s (s)}
											<span class="stop">
												≥
												<input
													class="input narrow"
													type="number"
													value={stop.at}
													aria-label="From"
													onchange={(e) =>
														retarget(tile, {
															...target,
															stops: target.stops.map((x, i) => (i === s ? { ...x, at: e.currentTarget.valueAsNumber || 0 } : x))
														})}
												/>
												<input
													type="color"
													value={stop.color}
													onchange={(e) =>
														retarget(tile, {
															...target,
															stops: target.stops.map((x, i) => (i === s ? { ...x, color: e.currentTarget.value } : x))
														})}
												/>
												<button
													class="btn btn-icon"
													title="Remove this color"
													onclick={() => retarget(tile, { ...target, stops: target.stops.filter((_, i) => i !== s) })}
												>×</button>
											</span>
										{/each}
										<button
											class="btn btn-ghost"
											onclick={() =>
												retarget(tile, {
													...target,
													stops: [
														...target.stops,
														target.stops.length === 0
															? { at: 0, color: '#22c55e' }
															: { at: Math.max(...target.stops.map((x) => x.at)) + 1, color: '#ef4444' }
													]
												})}
										>+ color</button>
									</div>
								{/if}
							</li>
						{/each}
					</ol>

					<div class="row">
						<select class="input" bind:value={adding} aria-label="Kind of tile to add">
							{#each TILE_KINDS as k (k.kind)}<option value={k.kind}>{k.label}</option>{/each}
						</select>
						<button class="btn btn-primary" onclick={addTile}>+ Tile</button>
					</div>
				</div>
			{/if}

			<div class="preview" class:framed={$canEdit}>
				{#if $canEdit}<p class="hint">As a phone draws it:</p>{/if}
				<SurfaceTiles {surface} show={$surfaceShow} />
			</div>
		</div>
	{/if}
</div>

<style>
	.surface-panel { padding: 16px; display: flex; flex-direction: column; gap: 12px; }
	.head, .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
	.spacer { flex: 1; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.grow { flex: 1; min-width: 120px; }
	.narrow { width: 72px; }
	.editor { list-style: none; display: flex; flex-direction: column; gap: 8px; }
	.tile-row { border: 1px solid #2e2e2e; border-radius: 4px; padding: 8px; display: flex; flex-direction: column; gap: 6px; }
	.line { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
	.check { display: flex; align-items: center; gap: 5px; color: #999; font-size: 12px; }
	.stop { display: inline-flex; align-items: center; gap: 4px; }
	.link { display: flex; gap: 16px; align-items: center; flex-wrap: wrap; background: #202020; border: 1px solid #333; border-radius: 6px; padding: 12px; }
	.link-text { display: flex; flex-direction: column; gap: 6px; min-width: 200px; flex: 1; font-size: 13px; color: #bbb; }
	.url { font-size: 18px; font-weight: 600; color: #8ab4ff; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
	/* The list and the preview side by side when the tile is wide enough, so an edit
	   is seen landing; stacked when it is not. */
	.work { display: flex; gap: 16px; flex-wrap: wrap; align-items: flex-start; }
	.build { flex: 1 1 400px; min-width: 0; display: flex; flex-direction: column; gap: 12px; }
	/* The preview at a phone's width, so a tile that wraps here wraps there. */
	.preview { flex: 0 1 380px; max-width: 420px; }
	.preview.framed { border: 1px dashed #3a3a3a; border-radius: 8px; padding: 10px; position: sticky; top: 0; }
	.hint { color: #777; font-size: 12px; }
	.empty { color: #777; font-size: 12px; line-height: 1.4; }
	.problem { color: #e05555; font-size: 12px; }
	.boards {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
</style>
