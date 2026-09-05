<script lang="ts">
	/**
	 * The paperwork: the sheet set on the left, the sheet in the middle, what one block
	 * of it is on the right, and one button that writes the lot.
	 *
	 * It is an **editor**, and the thing being edited is on the same screen as the
	 * result — a block is dragged and resized on the preview itself, because a sheet is
	 * a layout and laying one out by typing millimetres into a form is not laying it
	 * out. The numbers on the right are the readout you correct rather than the way you
	 * place anything.
	 *
	 * The preview is the *same drawing* the PDF is written from, rendered to SVG. Not a
	 * likeness of it and not a canvas approximation: `composeSheet` produces one model
	 * and `svg.ts` and `pdf.ts` are two renderings of it, so what is on screen here is
	 * what comes out of the printer, down to where each word sits.
	 *
	 * Three things are waited for before anything is drawn, and each of them is a
	 * decision rather than an implementation detail.
	 *
	 * **The font**, because layout depends on its metrics and a fallback would be a
	 * second answer to the question the whole feature is arranged around.
	 *
	 * **Every mesh the sheet needs.** `geometry.ts` hands back a placeholder box for a
	 * file it cannot load, which is right for a rig view — a view that went blank over
	 * one bad mesh would be worse — and wrong for a rigging plan, where a box is a
	 * drawing of something that is not there. So the export resolves them all first and
	 * refuses on a real failure, naming the file.
	 *
	 * **The tables**, from `paperwork.tables`, because the arithmetic is the station's:
	 * the weight this panel prints is the weight a plugin and the command line get.
	 */
	import { onMount } from 'svelte';
	import { derived } from 'svelte/store';

	import type {
		ParameterValue,
		Sheet,
		Table,
		TableBlock,
		ViewportBlock
	} from '$lib/generated/index.js';
	import { readingOf, type Showing } from '$lib/stores/output.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import { piece, stockSize } from '$lib/stock.js';
	import { getClientContext } from '$lib/ws/context.js';
	import { meshSizes, measureAll, PLACEHOLDER_SIZE } from '$lib/geometry.js';
	import { metrics, type Metrics } from '$lib/paperwork/font.js';
	import { composeSheet, type SheetContext } from '$lib/paperwork/sheet.js';
	import { toPdf } from '$lib/paperwork/pdf.js';
	import { toSvg } from '$lib/paperwork/svg.js';
	import { toCsv, csvName } from '$lib/paperwork/csv.js';
	import { canvasToPng, pixelsFor, type Raster } from '$lib/paperwork/raster.js';
	import Rig3D from '$lib/components/stage/Rig3D.svelte';
	import type { Projection, RenderMode, ViewSettings } from '$lib/stores/view.js';
	import type { ViewPreset } from '$lib/camera.js';
	import {
		addBlock,
		addSheet,
		currentSheet,
		deleteSheet,
		download,
		duplicateSheet,
		exportName,
		moveSheet,
		newTable,
		newText,
		newViewport,
		PAPERS,
		previewZoom,
		removeBlock,
		selectedBlock,
		setSheetField,
		sheets
	} from '$lib/stores/paperwork.js';
	import BlockEditor from './BlockEditor.svelte';
	import BlockHandles from './BlockHandles.svelte';
	import { layers, namedAssets, sceneObjects } from '$lib/stores/scene.js';
	import { collection, show } from '$lib/stores/show.js';
	import type { Vec3 } from '$lib/generated/index.js';

	const fixtures = collection('fixtures');
	const fixtureTypes = collection('fixture_types');
	const plans = collection('stage_plans');
	/** Every cue, ordered the way a cue list is, for the picture viewports' state. */
	const cueList = derived(collection('cues'), ($cues) =>
		[...$cues].sort((a, b) => a.number - b.number || a.name.localeCompare(b.name))
	);

	const client = getClientContext();

	let font = $state<Metrics | null>(null);
	let fontError = $state<string | null>(null);
	let tables = $state<Map<string, Table>>(new Map());
	let busy = $state<string | null>(null);
	let problem = $state<string | null>(null);

	/**
	 * The offscreen renderer the picture viewports are drawn by, and the settings it is
	 * currently wearing.
	 *
	 * It exists only while an export is running. A hidden WebGL context kept open for
	 * the life of the panel would be a second renderer competing with the rig panel for
	 * the GPU, on a machine where somebody is programming a show.
	 */
	let capturing = $state<{
		width: number;
		height: number;
		view: Partial<ViewSettings>;
		showing: Showing | null;
	} | null>(null);
	let stage = $state<HTMLDivElement | null>(null);
	let rig = $state<Rig3D | null>(null);

	onMount(() => {
		metrics()
			.then((m) => (font = m))
			.catch((e) => (fontError = String(e)));
	});

	const selected = $derived(
		$sheets.find((sheet) => sheet.id === $currentSheet) ?? $sheets[0] ?? null
	);

	$effect(() => {
		if (selected && $currentSheet === null) currentSheet.set(selected.id);
	});

	// A block is a position in an array, so a selection into a *different* sheet's
	// array means nothing. Cleared on the way rather than followed, which is the rule
	// `removeBlock` follows for the same reason.
	$effect(() => {
		void $currentSheet;
		selectedBlock.set(null);
	});

	/**
	 * CSS pixels per millimetre of paper.
	 *
	 * 96 dpi is what a browser calls an inch, so this is the number that turns the
	 * drawing's own units into where the overlay's handles go. The preview is scaled by
	 * a CSS transform, and the handles are placed in the same space, so the two cannot
	 * drift apart under a zoom.
	 */
	const pxPerMm = $derived((96 / 25.4) * $previewZoom);

	/** How big the selected sheet is, in millimetres, the way round it is. */
	const paperMm = $derived.by(() => {
		const portrait: Record<string, [number, number]> = {
			A4: [210, 297],
			A3: [297, 420],
			A2: [420, 594],
			A1: [594, 841]
		};
		const [w, h] = portrait[selected?.paper ?? 'A3'] ?? portrait.A3;
		return selected?.landscape ? { w: h, h: w } : { w, h };
	});

	/**
	 * Where a new block goes.
	 *
	 * Stepped down and across from the last one rather than dropped in the middle: a
	 * block placed exactly on top of the one before it looks like nothing happened, and
	 * the operator's next act is to drag it off anyway. Wrapped back to the top when
	 * the steps run out of paper.
	 */
	function freeSpace(): { x: number; y: number; w: number; h: number } {
		const step = 6;
		const count = selected?.blocks.length ?? 0;
		const wrap = 6;
		return {
			x: 20 + (count % wrap) * step,
			y: 20 + (count % wrap) * step,
			w: 120,
			h: 80
		};
	}

	// Meshes have to be measured before a drafting viewport can size anything.
	$effect(() => {
		const references = $sceneObjects.flatMap((object) => object.geometry);
		if (references.length > 0) measureAll(references, $namedAssets);
	});

	/**
	 * How big each object is, as a box: the catalogue's own dimensions for a stock
	 * piece, the measured bounds for a mesh. See `SheetContext.sizes` for why a box.
	 */
	const sizes = $derived.by(() => {
		const out = new Map<string, Vec3>();
		for (const object of $sceneObjects) {
			if (object.catalogue) {
				const entry = piece(object.catalogue);
				if (entry) {
					out.set(object.id, stockSize(entry, object.properties));
					continue;
				}
			}
			const reference = object.geometry[0];
			const measured = reference ? $meshSizes.get(reference.asset) : undefined;
			if (measured) out.set(object.id, { x: measured.x, y: measured.y, z: measured.z });
			else if (reference)
				out.set(object.id, {
					x: PLACEHOLDER_SIZE,
					y: PLACEHOLDER_SIZE,
					z: PLACEHOLDER_SIZE
				});
		}
		return out;
	});

	/**
	 * Rendered pictures, per sheet, keyed by the block's index.
	 *
	 * Filled twice over: cheaply and in the background for the preview, and again at the
	 * sheet's own dpi when somebody exports. A preview is looked at while it is being
	 * arranged, so it is rendered at screen resolution and re-rendered when what it
	 * shows changes; a file is made once.
	 */
	let rasters = $state<Map<string, Map<number, Raster>>>(new Map());

	/**
	 * What each rendered picture in the cache was rendered *from*.
	 *
	 * A picture is expensive — an offscreen WebGL context, a scene, and a wait for every
	 * mesh — so it is redone only when something it depends on actually moved. The
	 * signature is everything that changes the pixels and nothing that does not: moving
	 * the block on the page is not in it, and neither is the sheet's paper size.
	 */
	let renderedFrom = $state<Map<string, string>>(new Map());

	/** The values a set of cues comes to, keyed by the cues themselves. */
	let cueStates = new Map<string, Showing>();

	/**
	 * Everything about a picture viewport that decides what it looks like.
	 *
	 * The rig's own revision is in it — a fixture moved or re-aimed changes every
	 * picture — and so is the fixture list's length, which is the cheapest thing that
	 * catches a patch. What is deliberately *not* in it is the block's rectangle: a
	 * viewport dragged across the page shows the same picture, and re-rendering on every
	 * frame of a drag would make dragging one unusable.
	 */
	function signatureOf(block: ViewportBlock, rig: string): string {
		if (block.style.type !== 'Picture') return '';
		return JSON.stringify([
			block.view,
			block.projection,
			block.style.mode,
			block.style.work_light,
			block.style.cues,
			block.layers,
			rig
		]);
	}

	/** A cheap stand-in for "the rig changed", good enough to invalidate a picture. */
	const rigRevision = $derived(
		JSON.stringify([
			$fixtures.length,
			$sceneObjects.length,
			$fixtures.map((f) => f.position?.position.x ?? 0).reduce((a, b) => a + b, 0)
		])
	);

	/** Every table block on every sheet, keyed so one fetch serves every sheet. */
	const tableKey = (sheetId: string, index: number) => `${sheetId}:${index}`;

	$effect(() => {
		const wanted: Array<{ key: string; block: TableBlock }> = [];
		for (const sheet of $sheets) {
			sheet.blocks.forEach((block, index) => {
				if (block.type === 'Table') wanted.push({ key: tableKey(sheet.id, index), block });
			});
		}
		void refreshTables(wanted);
	});

	async function refreshTables(wanted: Array<{ key: string; block: TableBlock }>) {
		const next = new Map<string, Table>();
		for (const { key, block } of wanted) {
			try {
				const answer = (await client.call('paperwork.tables', {
					kind: block.kind,
					grouping: block.grouping,
					rows: block.rows,
					layers: block.layers
				})) as Table;
				next.set(key, answer);
			} catch (e) {
				problem = `The station could not build the ${block.kind} table: ${e}`;
			}
		}
		tables = next;
	}

	function contextFor(sheet: Sheet, number: number): SheetContext | null {
		if (!font) return null;
		const forSheet = new Map<number, Table>();
		sheet.blocks.forEach((_, index) => {
			const table = tables.get(tableKey(sheet.id, index));
			if (table) forSheet.set(index, table);
		});
		return {
			metrics: font,
			showName: $show?.name ?? 'Show',
			production: $show?.production ?? {
				title: '',
				venue: '',
				address: '',
				dates: '',
				designer: '',
				contact: '',
				revision: ''
			},
			sheetNumber: number,
			sheetCount: $sheets.length,
			fixtures: $fixtures,
			fixtureTypes: $fixtureTypes,
			sceneObjects: $sceneObjects,
			layers: $layers,
			sizes,
			tables: forSheet,
			// Empty in the preview and filled on export. A hidden WebGL context redrawn
			// on every preview keystroke would compete with whatever else this browser
			// is doing, so the frame says the view is rendered on export rather than
			// showing a stale picture of one.
			rasters: rasters.get(sheet.id) ?? new Map(),
			thumbnails: new Map()
		};
	}

	const preview = $derived.by(() => {
		if (!selected || !font) return null;
		const number = $sheets.findIndex((s) => s.id === selected.id) + 1;
		const ctx = contextFor(selected, number);
		return ctx ? toSvg(composeSheet(selected, ctx), font) : null;
	});

	const PRESET: Record<string, ViewPreset> = {
		Plan: 'plan',
		Front: 'front',
		Section: 'section',
		ThreeQuarter: 'quarter',
		Focus: 'front'
	};
	const MODE: Record<string, RenderMode> = {
		Wireframe: 'wireframe',
		Cones: 'cones',
		Real: 'real',
		Photoreal: 'photoreal'
	};

	/**
	 * The state a picture is drawn in: what the cues it names come to.
	 *
	 * Asked of the station, which works it out **without taking anything** — see
	 * `paperwork.cueValues`. A viewport naming no cues gets `null` and the renderer
	 * draws what the rig is actually doing, which is what somebody documenting a look
	 * they have just built wants.
	 */
	async function stateFor(block: ViewportBlock): Promise<Showing | null> {
		if (block.style.type !== 'Picture' || block.style.cues.length === 0) return null;
		const key = JSON.stringify(block.style.cues);
		const cached = cueStates.get(key);
		if (cached) return cached;
		try {
			const values = (await client.call('paperwork.cueValues', {
				cues: block.style.cues
			})) as Record<string, ParameterValue>;
			const showing = readingOf(values, consoleNow() ?? 0);
			cueStates.set(key, showing);
			return showing;
		} catch (e) {
			problem = `The station could not work out what those cues look like: ${e}`;
			return null;
		}
	}

	/** Render one picture viewport at a given dpi. */
	async function renderOne(block: ViewportBlock, dpi: number): Promise<Raster | null> {
		if (block.style.type !== 'Picture') return null;
		const pixels = pixelsFor(block.rect, dpi);
		const state = await stateFor(block);
		capturing = {
			width: pixels.width,
			height: pixels.height,
			view: {
				mode: MODE[block.style.mode] ?? 'real',
				workLight: block.style.work_light,
				// One device pixel per CSS pixel, so the drawing buffer is exactly the
				// size asked for however the display is scaled — a sheet must not come
				// out sharper on a Retina machine than on a projector.
				resolution: 1,
				projection: block.projection === 'Perspective' ? 'perspective' : 'ortho'
			},
			showing: state
		};
		// Two frames for the component to take the new size and settings, and then a
		// wait for the meshes: a group enters the scene when its row does and its
		// geometry is downloaded afterwards, so a capture taken two frames in draws a
		// rig with no trusses in it and says nothing.
		await nextFrame();
		await nextFrame();
		await settled();
		const projection: Projection = block.projection === 'Perspective' ? 'perspective' : 'ortho';
		const canvas = rig?.captureFrame(PRESET[block.view] ?? 'front', projection);
		if (!canvas) return null;
		return { data: await canvasToPng(canvas), dpi: pixels.dpi };
	}

	/** Render every picture viewport in the set at its own dpi, for the file. */
	async function renderPictures() {
		const out = new Map<string, Map<number, Raster>>();
		for (const sheet of $sheets) {
			for (const [index, block] of sheet.blocks.entries()) {
				if (block.type !== 'Viewport' || block.style.type !== 'Picture') continue;
				busy = `Rendering ${sheet.name} — ${block.title || 'view'}…`;
				const raster = await renderOne(block, block.style.dpi);
				if (!raster) continue;
				const forSheet = out.get(sheet.id) ?? new Map<number, Raster>();
				forSheet.set(index, raster);
				out.set(sheet.id, forSheet);
			}
		}
		rasters = out;
		capturing = null;
	}

	/**
	 * Keep the previews of the sheet being looked at up to date, quietly.
	 *
	 * **Debounced, and only the visible sheet.** A picture costs an offscreen WebGL
	 * context and a wait for every mesh, so rendering one per keystroke while somebody
	 * drags a work-light slider would make the panel unusable — and rendering the other
	 * five sheets' pictures would pay for six views to answer a question about one.
	 *
	 * At screen resolution rather than the sheet's own: the preview is looked at on a
	 * screen, and a 300 dpi A3 render to fill a 190 mm box on a monitor is four times
	 * the pixels nobody can see.
	 */
	const PREVIEW_DPI = 96;
	let previewTimer: ReturnType<typeof setTimeout> | null = null;
	let previewing = false;

	async function refreshPreviews(sheet: Sheet) {
		if (previewing || busy) return;
		previewing = true;
		try {
			for (const [index, block] of sheet.blocks.entries()) {
				if (block.type !== 'Viewport' || block.style.type !== 'Picture') continue;
				const key = `${sheet.id}:${index}`;
				const want = signatureOf(block, rigRevision);
				if (renderedFrom.get(key) === want) continue;
				const raster = await renderOne(block, PREVIEW_DPI);
				capturing = null;
				if (!raster) continue;
				const forSheet = new Map(rasters.get(sheet.id) ?? []);
				forSheet.set(index, raster);
				rasters = new Map(rasters).set(sheet.id, forSheet);
				renderedFrom = new Map(renderedFrom).set(key, want);
			}
		} catch {
			// A preview that could not be drawn leaves the frame saying so. It is not
			// worth a banner: the export is where a failure has to stop somebody.
		} finally {
			capturing = null;
			previewing = false;
		}
	}

	$effect(() => {
		const sheet = selected;
		// Read what a picture depends on, so this effect re-runs when any of it moves.
		const wanted = sheet?.blocks
			.filter((b) => b.type === 'Viewport' && b.style.type === 'Picture')
			.map((b) => signatureOf(b as ViewportBlock, rigRevision))
			.join('|');
		if (!sheet || !wanted || !font) return;
		if (previewTimer) clearTimeout(previewTimer);
		previewTimer = setTimeout(() => void refreshPreviews(sheet), 700);
		return () => {
			if (previewTimer) clearTimeout(previewTimer);
		};
	});

	/**
	 * Wait for the next frame — or for a timer, whichever comes first.
	 *
	 * **A backgrounded tab is served no animation frames at all.** Every wait in the
	 * render path was a bare `requestAnimationFrame`, so switching to another tab
	 * mid-export parked the whole thing on the first one until the mesh wait gave up
	 * thirty seconds later and reported a failure that had not happened. The renderer
	 * itself does not need the frame — `captureFrame` draws synchronously — so the wait
	 * is only for Svelte to have applied the new size and settings, and a timer does
	 * that just as well.
	 */
	const nextFrame = () =>
		new Promise<void>((resolve) => {
			let done = false;
			const finish = () => {
				if (done) return;
				done = true;
				resolve();
			};
			requestAnimationFrame(finish);
			setTimeout(finish, 34);
		});

	/**
	 * Wait until every piece of the drawing has its geometry, or refuse.
	 *
	 * The rule the whole export follows, and the one place it is enforced: a rig view
	 * that draws a placeholder box for a mesh it could not load is degrading helpfully,
	 * and a rigging plan that does the same is a drawing of something that is not
	 * there. So this waits, and then stops the export rather than printing what it has.
	 */
	async function settled(): Promise<void> {
		const deadline = performance.now() + 30_000;
		for (;;) {
			const waiting = rig?.pendingGeometry() ?? 0;
			if (waiting === 0) return;
			if (performance.now() > deadline) {
				throw new Error(
					`${waiting} ${waiting === 1 ? 'piece' : 'pieces'} of the drawing did not load. ` +
						'Rather than print a sheet with them missing, the export has stopped.'
				);
			}
			await nextFrame();
		}
	}

	async function exportPdf() {
		if (!font || $sheets.length === 0) return;
		busy = 'Rendering…';
		problem = null;
		try {
			await renderPictures();
			busy = 'Writing the PDF…';
			const drawings = $sheets.map((sheet, index) => {
				const ctx = contextFor(sheet, index + 1);
				if (!ctx) throw new Error('the font is not loaded');
				return composeSheet(sheet, ctx);
			});
			const bytes = await toPdf(drawings, font, {
				title: `${$show?.name ?? 'Show'} — paperwork`,
				author: $show?.production?.designer ?? ''
			});
			download(bytes, exportName($show?.name ?? 'Show', 'pdf'), 'application/pdf');
		} catch (e) {
			problem = `The export failed: ${e}`;
		} finally {
			busy = null;
			capturing = null;
		}
	}

	function exportCsv() {
		const written = new Set<string>();
		for (const sheet of $sheets) {
			sheet.blocks.forEach((block, index) => {
				if (block.type !== 'Table') return;
				const table = tables.get(tableKey(sheet.id, index));
				if (!table) return;
				const title = block.title || block.kind;
				if (written.has(title)) return;
				written.add(title);
				download(
					toCsv(table),
					csvName($show?.name ?? 'Show', title),
					'text/csv;charset=utf-8'
				);
			});
		}
		if (written.size === 0) problem = 'There are no tables on any sheet to export.';
	}
</script>

<div class="paperwork">
	<aside>
		<header>
			<h2>Sheets</h2>
			<button class="small" onclick={() => void addSheet()} title="Add a sheet">+</button>
		</header>

		{#if $sheets.length === 0}
			<p class="empty">
				This show has no sheets. They are seeded into a show when it is created, so a
				showfile made before paperwork existed has none — start one with <b>+</b>.
			</p>
		{:else}
			<ul>
				{#each $sheets as sheet, index (sheet.id)}
					<li>
						<button
							class="sheet"
							class:selected={selected?.id === sheet.id}
							onclick={() => currentSheet.set(sheet.id)}
						>
							<span class="number">{index + 1}</span>
							<span class="name">{sheet.name}</span>
							<span class="paper">{sheet.paper}{sheet.landscape ? ' ↔' : ''}</span>
						</button>
					</li>
				{/each}
			</ul>
		{/if}

		{#if selected}
			<div class="sheet-tools">
				<button class="small" onclick={() => void moveSheet(selected.id, -1)} title="Earlier">↑</button>
				<button class="small" onclick={() => void moveSheet(selected.id, 1)} title="Later">↓</button>
				<button class="small" onclick={() => void duplicateSheet(selected.id)}>Duplicate</button>
				<button class="small" onclick={() => void deleteSheet(selected.id)}>Delete</button>
			</div>

			<fieldset>
				<legend>This sheet</legend>
				<label class="row">
					Name
					<input
						value={selected.name}
						oninput={(e) => void setSheetField(selected.id, 'name', e.currentTarget.value)}
					/>
				</label>
				<label class="row">
					Paper
					<select
						value={selected.paper}
						onchange={(e) => void setSheetField(selected.id, 'paper', e.currentTarget.value as never)}
					>
						{#each PAPERS as paper (paper)}
							<option value={paper}>{paper}</option>
						{/each}
					</select>
				</label>
				<label class="check">
					<input
						type="checkbox"
						checked={selected.landscape}
						onchange={(e) => void setSheetField(selected.id, 'landscape', e.currentTarget.checked)}
					/>
					Landscape
				</label>
				<label class="check">
					<input
						type="checkbox"
						checked={selected.frame}
						onchange={(e) => void setSheetField(selected.id, 'frame', e.currentTarget.checked)}
					/>
					Border and grid
				</label>
				<label class="check">
					<input
						type="checkbox"
						checked={selected.title_block}
						onchange={(e) => void setSheetField(selected.id, 'title_block', e.currentTarget.checked)}
					/>
					Title block
				</label>
			</fieldset>

			<fieldset>
				<legend>Add a block</legend>
				<div class="adders">
					<button class="small" onclick={() => void addBlock(selected.id, newViewport(freeSpace()))}>
						Viewport
					</button>
					<button class="small" onclick={() => void addBlock(selected.id, newTable(freeSpace()))}>
						Table
					</button>
					<button class="small" onclick={() => void addBlock(selected.id, newText(freeSpace()))}>
						Text
					</button>
				</div>
			</fieldset>
		{/if}

		<div class="actions">
			<button onclick={exportPdf} disabled={busy !== null || $sheets.length === 0}>
				{busy ?? 'Export PDF'}
			</button>
			<button onclick={exportCsv} disabled={tables.size === 0}>Export tables as CSV</button>
		</div>

		<label class="zoom">
			Zoom
			<input type="range" min="0.25" max="3" step="0.05" bind:value={$previewZoom} />
			<span>{Math.round($previewZoom * 100)}%</span>
		</label>

		{#if problem}<p class="problem">{problem}</p>{/if}
		{#if fontError}
			<p class="problem">
				The drawing font did not load, so no sheet can be laid out: {fontError}
			</p>
		{/if}
	</aside>

	<div class="stage">
		{#if preview && selected}
			<div class="paper-shadow" style="transform: scale({$previewZoom}); transform-origin: top left;">
				{@html preview}
			</div>
			<!--
				The handles sit over the preview at the same scale, in a second layer that
				is *not* transformed: a CSS scale on a border draws a scaled border, and a
				handle that grows with the zoom is a handle that covers what it is
				resizing at 300%.
			-->
			<div class="handles">
				<BlockHandles
					sheetId={selected.id}
					blocks={selected.blocks}
					zoom={pxPerMm}
					paper={paperMm}
					selected={$selectedBlock}
					onSelect={(index) => selectedBlock.set(index)}
				/>
			</div>
		{:else if fontError}
			<p class="waiting">Waiting for the drawing font.</p>
		{:else}
			<p class="waiting">Loading…</p>
		{/if}
	</div>

	<aside class="right">
		{#if selected && $selectedBlock !== null && selected.blocks[$selectedBlock]}
			<BlockEditor
				sheetId={selected.id}
				index={$selectedBlock}
				block={selected.blocks[$selectedBlock]}
				layers={$layers}
				cues={$cueList}
				onRemove={() => void removeBlock(selected.id, $selectedBlock ?? 0)}
			/>
		{:else}
			<p class="empty">
				Pick a block on the sheet to change what it shows, or drag one to move it.
			</p>
		{/if}
	</aside>
</div>

{#if capturing}
	<!--
		The offscreen renderer. Off the left of the page rather than `display: none`,
		because a hidden element has no layout and a WebGL canvas with no size renders
		nothing at all — which would be a sheet full of blank viewports and no error.
	-->
	<div
		class="offscreen"
		bind:this={stage}
		style="width: {capturing.width}px; height: {capturing.height}px;"
	>
		<Rig3D
			bind:this={rig}
			fixtures={$fixtures}
			types={$fixtureTypes}
			plan={null}
			planUrl={null}
			show={$show}
			viewOverride={capturing.view}
			showingOverride={capturing.showing}
			forCapture
		/>
	</div>
{/if}

<style>
	.offscreen {
		position: fixed;
		left: -20000px;
		top: 0;
		pointer-events: none;
	}

	.paperwork {
		display: grid;
		grid-template-columns: 15rem 1fr 15rem;
		height: 100%;
		min-height: 0;
		font-size: 0.8rem;
	}

	aside {
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
		padding: 0.6rem;
		border-right: 1px solid var(--line, #2a2a2a);
		overflow-y: auto;
	}

	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
	}

	h2 {
		margin: 0;
		font-size: 0.85rem;
		font-weight: 600;
	}

	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}

	li button.sheet {
		display: grid;
		grid-template-columns: 1.2rem 1fr auto;
		gap: 0.4rem;
		align-items: baseline;
		width: 100%;
		padding: 0.3rem 0.4rem;
		text-align: left;
		background: none;
		border: 1px solid transparent;
		border-radius: 3px;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	li button.sheet:hover {
		background: var(--hover, #1e1e1e);
	}

	li button.sheet.selected {
		background: var(--selected, #26323d);
		border-color: var(--accent, #4a90d9);
	}

	.number {
		opacity: 0.5;
	}

	.paper {
		opacity: 0.5;
		font-size: 0.7rem;
	}

	.actions {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}

	.actions button {
		padding: 0.4rem;
		border-radius: 3px;
		border: 1px solid var(--line, #2a2a2a);
		background: var(--panel, #1a1a1a);
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	.actions button:disabled {
		opacity: 0.45;
		cursor: default;
	}

	.zoom {
		display: grid;
		grid-template-columns: auto 1fr auto;
		gap: 0.4rem;
		align-items: center;
		opacity: 0.8;
	}

	.empty,
	.problem,
	.waiting {
		opacity: 0.7;
		line-height: 1.4;
	}

	.problem {
		color: var(--warn, #e0a33e);
	}

	.stage {
		overflow: auto;
		padding: 1rem;
		background: #2b2b2b;
		position: relative;
	}

	.paper-shadow {
		display: inline-block;
		box-shadow: 0 2px 14px rgba(0, 0, 0, 0.5);
		background: #fff;
	}

	/*
	 * The handles sit in their own untransformed layer over the preview. Inside the
	 * scaled one they would be scaled too — a 1 px border drawn 3 px wide at 300%, and
	 * grips that cover the block they are resizing.
	 */
	.handles {
		position: absolute;
		left: 1rem;
		top: 1rem;
	}

	aside.right {
		border-right: none;
		border-left: 1px solid var(--line, #2a2a2a);
	}

	.small {
		background: var(--panel, #1a1a1a);
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		color: inherit;
		font: inherit;
		font-size: 0.7rem;
		padding: 0.15rem 0.4rem;
		cursor: pointer;
	}

	.sheet-tools,
	.adders {
		display: flex;
		gap: 0.25rem;
		flex-wrap: wrap;
	}

	fieldset {
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		padding: 0.3rem 0.45rem 0.45rem;
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}

	legend {
		font-size: 0.65rem;
		opacity: 0.6;
		padding: 0 0.25rem;
	}

	.row {
		display: grid;
		grid-template-columns: 3.6rem 1fr;
		gap: 0.4rem;
		align-items: center;
	}

	.check {
		display: flex;
		gap: 0.35rem;
		align-items: center;
	}

	input,
	select {
		background: var(--field, #141414);
		border: 1px solid var(--line, #2a2a2a);
		border-radius: 3px;
		color: inherit;
		font: inherit;
		font-size: 0.75rem;
		padding: 0.15rem 0.25rem;
		min-width: 0;
	}

	.zoom input,
	.check input {
		background: none;
		border: none;
		padding: 0;
	}
</style>
