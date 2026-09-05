<script lang="ts">
	/**
	 * What one block on a sheet is, in numbers and words.
	 *
	 * Every control writes the whole block back through `writeBlock`, because
	 * `Sheet::blocks` is one field: there is no path to a block's `scale` on its own,
	 * and inventing one would mean giving a block an identity nothing else refers to.
	 * A rewrite of a handful of blocks is cheap and it is the unit an operator thinks
	 * in anyway.
	 *
	 * The one thing worth knowing about the layout of this panel: **the geometry is at
	 * the top and the rest is below it**. A block is dragged on the preview far more
	 * often than it is typed, so the numbers here are the readout you check and correct
	 * rather than the way you place anything.
	 */
	import type { Cue, Layer, SheetBlock } from '$lib/generated/index.js';
	import {
		GROUPINGS,
		LABEL_FIELDS,
		SCALES,
		TABLE_KINDS,
		VIEWS,
		writeBlock
	} from '$lib/stores/paperwork.js';

	let {
		sheetId,
		index,
		block,
		layers,
		cues,
		onRemove
	}: {
		sheetId: string;
		index: number;
		block: SheetBlock;
		layers: Layer[];
		/** Every cue in the show, for a picture viewport to stand in one. */
		cues: Cue[];
		onRemove: () => void;
	} = $props();

	/** Write a changed copy. Every control in here goes through this. */
	function put(change: Partial<SheetBlock>) {
		void writeBlock(sheetId, index, { ...block, ...change } as SheetBlock);
	}

	function putRect(field: 'x' | 'y' | 'w' | 'h', value: number) {
		if (!Number.isFinite(value)) return;
		put({ rect: { ...block.rect, [field]: value } } as Partial<SheetBlock>);
	}

	const isViewport = $derived(block.type === 'Viewport');
	const isTable = $derived(block.type === 'Table');
	const isText = $derived(block.type === 'Text');

	/** A viewport's scale, as the control sees it: `fit` or a denominator. */
	const scaleValue = $derived(
		block.type === 'Viewport' && block.scale.type === 'Ratio'
			? String(block.scale.denominator)
			: 'fit'
	);

	function setScale(value: string) {
		put({
			scale: value === 'fit' ? { type: 'Fit' } : { type: 'Ratio', denominator: Number(value) }
		} as Partial<SheetBlock>);
	}

	/** Which layers this block shows. `null` is every layer, and is not the same as none. */
	function toggleLayer(id: string, on: boolean) {
		if (!('layers' in block)) return;
		const current = block.layers ?? layers.map((layer) => layer.id);
		const next = on ? [...new Set([...current, id])] : current.filter((each) => each !== id);
		// All of them means all of them: a list that happens to name every layer would
		// stop showing one somebody adds later, which is not what "everything" means.
		put({ layers: next.length === layers.length ? null : next } as Partial<SheetBlock>);
	}

	function toggleLabel(field: (typeof LABEL_FIELDS)[number]['value'], on: boolean) {
		if (block.type !== 'Viewport') return;
		const next = on
			? [...block.labels, field]
			: block.labels.filter((each) => each !== field);
		put({ labels: next } as Partial<SheetBlock>);
	}

	/** Rewrite one of the cues this picture stands in. */
	function setShot(at: number, shot: { cue: string; at_ms: number }) {
		if (block.type !== 'Viewport' || block.style.type !== 'Picture') return;
		const cues = block.style.cues.map((each, index) => (index === at ? shot : each));
		put({ style: { ...block.style, cues } } as never);
	}

	function removeShot(at: number) {
		if (block.type !== 'Viewport' || block.style.type !== 'Picture') return;
		const next = block.style.cues.filter((_, index) => index !== at);
		put({ style: { ...block.style, cues: next } } as never);
	}

	function addShot() {
		if (block.type !== 'Viewport' || block.style.type !== 'Picture' || cues.length === 0) return;
		// Five seconds in, which is past the fade on almost any cue somebody writes and
		// far enough into an effect to be somewhere other than its first frame. The same
		// default `CueShot::settled` uses on the station.
		const next = [...block.style.cues, { cue: cues[0].id, at_ms: 5000 }];
		put({ style: { ...block.style, cues: next } } as never);
	}

	function setStyleKind(kind: 'Drafting' | 'Picture') {
		if (block.type !== 'Viewport') return;
		put({
			style:
				kind === 'Drafting'
					? { type: 'Drafting', lines: 'Hidden', ink: 'Mono' }
					: // Lit for paper rather than for a screen, the way the seeded sheets
						// are: a dark studio on A3 is a near-black rectangle.
						{ type: 'Picture', mode: 'Cones', work_light: 1, dpi: 300 }
		} as Partial<SheetBlock>);
	}
</script>

<div class="block">
	<header>
		<h3>{block.type}</h3>
		<button class="remove" onclick={onRemove} title="Remove this block">Remove</button>
	</header>

	<div class="rect">
		{#each [['x', 'X'], ['y', 'Y'], ['w', 'W'], ['h', 'H']] as const as [field, label] (field)}
			<label>
				{label}
				<input
					type="number"
					step="1"
					value={Math.round(block.rect[field] * 10) / 10}
					oninput={(e) => putRect(field, Number(e.currentTarget.value))}
				/>
			</label>
		{/each}
		<span class="unit">mm</span>
	</div>

	{#if isViewport && block.type === 'Viewport'}
		<label class="row">
			Title
			<input value={block.title} oninput={(e) => put({ title: e.currentTarget.value })} />
		</label>

		<label class="row">
			Looking from
			<select value={block.view} onchange={(e) => put({ view: e.currentTarget.value } as never)}>
				{#each VIEWS as view (view.value)}
					<option value={view.value}>{view.label}</option>
				{/each}
			</select>
		</label>

		<label class="row">
			Projection
			<select
				value={block.projection}
				onchange={(e) => put({ projection: e.currentTarget.value } as never)}
			>
				<option value="Orthographic">Orthographic</option>
				<option value="Perspective">Perspective</option>
			</select>
		</label>

		<label class="row">
			Scale
			<select value={scaleValue} onchange={(e) => setScale(e.currentTarget.value)}>
				<option value="fit">Fit</option>
				{#each SCALES as scale (scale)}
					<option value={String(scale)}>1:{scale}</option>
				{/each}
			</select>
		</label>
		<p class="note">
			{#if block.style.type === 'Picture'}
				A picture is fitted by the rig renderer, so it prints NTS whatever this says.
			{:else if block.projection === 'Perspective'}
				A perspective view has no scale and prints NTS.
			{:else if block.scale.type === 'Fit'}
				Fit snaps to the nearest ratio a rule is cut for, so the drawing can be measured.
			{/if}
		</p>

		<label class="row">
			Drawn as
			<select
				value={block.style.type}
				onchange={(e) => setStyleKind(e.currentTarget.value as 'Drafting' | 'Picture')}
			>
				<option value="Drafting">Lines (vector)</option>
				<option value="Picture">Rendered picture</option>
			</select>
		</label>

		{#if block.style.type === 'Drafting'}
			<label class="row">
				Solids
				<select
					value={block.style.lines}
					onchange={(e) =>
						put({
							style: { ...block.style, lines: e.currentTarget.value }
						} as never)}
				>
					<option value="Hidden">Filled outlines</option>
					<option value="Wireframe">Wireframe</option>
				</select>
			</label>
			<label class="row">
				Ink
				<select
					value={block.style.ink}
					onchange={(e) => put({ style: { ...block.style, ink: e.currentTarget.value } } as never)}
				>
					<option value="Mono">Black on white</option>
					<option value="ByLayer">By layer</option>
					<option value="ByClass">By class</option>
				</select>
			</label>
		{:else}
			<label class="row">
				Render mode
				<select
					value={block.style.mode}
					onchange={(e) => put({ style: { ...block.style, mode: e.currentTarget.value } } as never)}
				>
					<option value="Wireframe">Wireframe</option>
					<option value="Cones">Cones</option>
					<option value="Real">Real</option>
					<option value="Photoreal">Photoreal</option>
				</select>
			</label>
			<p class="note">
				{#if block.style.mode === 'Real' || block.style.mode === 'Photoreal'}
					A beam is additive light, so this one keeps its dark ground — there is no
					version of it on white.
				{:else}
					Drawn on white, with the models in flat grey.
				{/if}
			</p>
			<label class="row">
				Work light
				<input
					type="range"
					min="0"
					max="1"
					step="0.05"
					value={block.style.work_light}
					oninput={(e) =>
						put({
							style: { ...block.style, work_light: Number(e.currentTarget.value) }
						} as never)}
				/>
			</label>
			<label class="row">
				Resolution
				<select
					value={String(block.style.dpi)}
					onchange={(e) =>
						put({ style: { ...block.style, dpi: Number(e.currentTarget.value) } } as never)}
				>
					<option value="150">150 dpi</option>
					<option value="300">300 dpi</option>
					<option value="600">600 dpi</option>
				</select>
			</label>
			<p class="note">Capped by what this machine's GPU will allocate; the sheet prints what it got.</p>

			<!--
				Which cues the rig is standing in for the picture, and how far into each.
				The time is the point: a cue sampled at zero is the state *before* it —
				a five-second fade up from black renders black — and an effect at zero
				has every head at the same phase. Both are the least useful frame.
			-->
			<fieldset>
				<legend>Standing in</legend>
				{#each block.style.cues as shot, at (at)}
					<div class="shot">
						<select
							value={shot.cue}
							onchange={(e) => setShot(at, { ...shot, cue: e.currentTarget.value })}
						>
							{#each cues as cue (cue.id)}
								<option value={cue.id}>{cue.number} {cue.name}</option>
							{/each}
						</select>
						<input
							type="number"
							step="0.5"
							min="0"
							value={shot.at_ms / 1000}
							title="Seconds into the cue"
							onchange={(e) =>
								setShot(at, {
									...shot,
									at_ms: Math.max(0, Number(e.currentTarget.value) * 1000)
								})}
						/>
						<span class="unit">s</span>
						<button class="remove" onclick={() => removeShot(at)}>✕</button>
					</div>
				{/each}
				{#if cues.length > 0}
					<button class="remove wide" onclick={addShot}>Add a cue</button>
				{:else}
					<p class="note">This show has no cues yet.</p>
				{/if}
				<p class="note">
					{#if block.style.cues.length === 0}
						Nothing named, so this draws what the rig is doing now.
					{:else}
						Worked out without taking anything — the playback is not touched.
					{/if}
				</p>
			</fieldset>
		{/if}

		<!--
			Labels are a *drafting* thing, and there is no version of them for a picture:
			a rendered view is a photograph of the rig, and a photograph with the channel
			numbers written over it is neither one thing nor the other. The composer
			draws none on a picture either, so showing the controls here would be
			offering a setting that does nothing.
		-->
		{#if block.style.type === 'Drafting'}
			<fieldset>
				<legend>Labels</legend>
			{#each LABEL_FIELDS as field (field.value)}
				<label class="check">
					<input
						type="checkbox"
						checked={block.labels.includes(field.value)}
						onchange={(e) => toggleLabel(field.value, e.currentTarget.checked)}
					/>
					{field.label}
				</label>
			{/each}
		</fieldset>

		<label class="check">
			<input
				type="checkbox"
				checked={block.scale_bar}
				onchange={(e) => put({ scale_bar: e.currentTarget.checked })}
			/>
			Scale bar
		</label>
		<label class="check">
			<input
				type="checkbox"
				checked={block.orientation_mark}
				onchange={(e) => put({ orientation_mark: e.currentTarget.checked })}
			/>
			Upstage mark
		</label>

		<fieldset>
			<legend>Dimensions</legend>
			<label class="check">
				<input
					type="checkbox"
					checked={block.dimensions !== null}
					onchange={(e) =>
						put({
							dimensions: e.currentTarget.checked
								? { datum: 'Left', running: true, above: false }
								: null
						} as never)}
				/>
				Measure along each bar
			</label>
			{#if block.dimensions}
				<label class="row">
					From
					<select
						value={block.dimensions.datum}
						onchange={(e) =>
							put({
								dimensions: { ...block.dimensions, datum: e.currentTarget.value }
							} as never)}
					>
						<option value="Left">The left end</option>
						<option value="Right">The right end</option>
						<option value="Centre">The centre line</option>
					</select>
				</label>
				<label class="check indent">
					<input
						type="checkbox"
						checked={block.dimensions.running}
						onchange={(e) =>
							put({
								dimensions: { ...block.dimensions, running: e.currentTarget.checked }
							} as never)}
					/>
					Distances from that end, as well as the gaps
				</label>
				<label class="check indent">
					<input
						type="checkbox"
						checked={block.dimensions.above}
						onchange={(e) =>
							put({
								dimensions: { ...block.dimensions, above: e.currentTarget.checked }
							} as never)}
					/>
					Draw them above the bar
				</label>
			{/if}
		</fieldset>
		{/if}
	{/if}

	{#if isTable && block.type === 'Table'}
		<label class="row">
			Title
			<input value={block.title} oninput={(e) => put({ title: e.currentTarget.value })} />
		</label>
		<label class="row">
			Table
			<select value={block.kind} onchange={(e) => put({ kind: e.currentTarget.value } as never)}>
				{#each TABLE_KINDS as kind (kind.value)}
					<option value={kind.value}>{kind.label}</option>
				{/each}
			</select>
		</label>
		<p class="note">{TABLE_KINDS.find((k) => k.value === block.kind)?.blurb}</p>
		<label class="row">
			Grouped
			<select
				value={block.grouping}
				onchange={(e) => put({ grouping: e.currentTarget.value } as never)}
			>
				{#each GROUPINGS as grouping (grouping.value)}
					<option value={grouping.value}>{grouping.label}</option>
				{/each}
			</select>
		</label>
	{/if}

	{#if isText && block.type === 'Text'}
		<label class="row column">
			Text
			<textarea rows="3" value={block.text} oninput={(e) => put({ text: e.currentTarget.value })}
			></textarea>
		</label>
		<label class="row">
			Size
			<input
				type="number"
				step="0.5"
				min="1"
				value={block.size_mm}
				oninput={(e) => put({ size_mm: Number(e.currentTarget.value) })}
			/>
		</label>
		<label class="check">
			<input type="checkbox" checked={block.bold} onchange={(e) => put({ bold: e.currentTarget.checked })} />
			Bold
		</label>
	{/if}

	{#if 'layers' in block && layers.length > 0}
		<fieldset>
			<legend>Layers</legend>
			<label class="check">
				<input
					type="checkbox"
					checked={block.layers === null}
					onchange={(e) => put({ layers: e.currentTarget.checked ? null : [] } as never)}
				/>
				Every layer
			</label>
			{#if block.layers !== null}
				{#each layers as layer (layer.id)}
					<label class="check indent">
						<input
							type="checkbox"
							checked={block.layers.includes(layer.id)}
							onchange={(e) => toggleLayer(layer.id, e.currentTarget.checked)}
						/>
						{layer.name}
					</label>
				{/each}
			{/if}
		</fieldset>
		{#if isTable && block.layers !== null}
			<p class="note">A table that leaves something out says so under its own totals.</p>
		{/if}
	{/if}
</div>

<style>
	.block {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
	}

	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
	}

	h3 {
		margin: 0;
		font-size: 0.8rem;
		font-weight: 600;
	}

	.remove {
		background: none;
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		color: inherit;
		font: inherit;
		font-size: 0.7rem;
		padding: 0.1rem 0.35rem;
		cursor: pointer;
	}

	.rect {
		display: grid;
		grid-template-columns: repeat(4, 1fr) auto;
		gap: 0.25rem;
		align-items: end;
	}

	.rect label {
		display: flex;
		flex-direction: column;
		font-size: 0.65rem;
		opacity: 0.7;
	}

	.rect input {
		width: 100%;
		min-width: 0;
	}

	.unit {
		font-size: 0.65rem;
		opacity: 0.5;
		padding-bottom: 0.2rem;
	}

	.row {
		display: grid;
		grid-template-columns: 5.5rem 1fr;
		gap: 0.4rem;
		align-items: center;
	}

	.row.column {
		grid-template-columns: 1fr;
	}

	.check {
		display: flex;
		gap: 0.35rem;
		align-items: center;
	}

	.check.indent {
		padding-left: 0.9rem;
	}

	.shot {
		display: grid;
		grid-template-columns: 1fr 3rem auto auto;
		gap: 0.25rem;
		align-items: center;
	}

	.remove.wide {
		width: 100%;
	}

	fieldset {
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		padding: 0.3rem 0.45rem 0.45rem;
		margin: 0.2rem 0;
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	legend {
		font-size: 0.65rem;
		opacity: 0.6;
		padding: 0 0.25rem;
	}

	.note {
		margin: 0;
		font-size: 0.68rem;
		opacity: 0.6;
		line-height: 1.35;
	}

	input,
	select,
	textarea {
		background: var(--field, #141414);
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		color: inherit;
		font: inherit;
		font-size: 0.75rem;
		padding: 0.15rem 0.25rem;
		min-width: 0;
	}
</style>
