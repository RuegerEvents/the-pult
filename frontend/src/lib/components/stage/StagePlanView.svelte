<script lang="ts">
	import { untrack } from 'svelte';
	import type { Fixture, FixtureType, ParameterValue, StagePlan } from '$lib/generated/index.js';
	import {
		aimAt,
		beamSpot,
		fixtureBounds,
		fixturePoint,
		fixtureOutput,
		fixtureTint,
		pixelToPlan,
		planExtent,
		planToPixel
	} from '$lib/stage.js';
	import { selected, select, toggle } from '$lib/stores/selection.js';
	import { byKey, setValue } from '$lib/stores/programmer.js';
	import Quicksheet from '$lib/components/programmer/Quicksheet.svelte';
	import { asOneGesture } from '$lib/stores/gesture.js';

	type Mode = 'move' | 'program' | 'scale' | 'origin';

	let {
		plan,
		planUrl,
		fixtures,
		types,
		mode = 'move',
		onplace,
		onmeasure,
		onorigin
	}: {
		plan: StagePlan | null;
		planUrl: string | null;
		fixtures: Fixture[];
		types: FixtureType[];
		mode?: Mode;
		onplace?: (fixtureId: string, x: number, z: number) => void;
		onmeasure?: (a: { px: number; py: number }, b: { px: number; py: number }) => void;
		onorigin?: (px: number, py: number) => void;
	} = $props();

	let svg = $state<SVGSVGElement | null>(null);

	// The viewport, in metres. Panning and zooming move this rather than a transform,
	// so a fixture's coordinates are always the room's and never the screen's.
	let view = $state({ x: -12, z: -8, width: 24 });
	let box = $state({ w: 1, h: 1 });
	const height = $derived((view.width * box.h) / Math.max(box.w, 1));

	const placed = $derived(fixtures.filter((f) => fixturePoint(f) !== null));
	const unplaced = $derived(fixtures.filter((f) => fixturePoint(f) === null));
	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);

	/// Everything one symbol needs drawing, worked out once per delivery rather than
	/// four times inside the template.
	/// How far from a fixture the beam-spot handle may be drawn. Half a screen at
	/// whatever the view is showing, so it is always somewhere a pointer can reach.
	const maxThrow = $derived(Math.min(40, Math.max(4, view.width * 0.45)));

	const symbols = $derived(
		placed.map((fixture) => {
			const at = fixturePoint(fixture)!;
			const spot = beamSpot(fixture, typeOf(fixture), { maxThrow });
			return {
				fixture,
				at,
				tint: fixtureTint(fixture),
				level: fixtureOutput(fixture).level,
				/// Where the beam lands, in this symbol's own coordinates.
				reach: spot ? { dx: spot.x - at.x, dz: spot.z - at.z } : null
			};
		})
	);

	const chosen = $derived(symbols.filter((s) => $selected.has(s.fixture.id)));

	/// Two clicks make a measurement; the first is held here until the second lands.
	let firstMark = $state<{ px: number; py: number } | null>(null);
	let dragging = $state<{ id: string } | null>(null);
	let panning = $state<{ x: number; z: number; clientX: number; clientY: number } | null>(null);
	/// A beam being dragged by where it lands, and a level being dragged by its ring.
	///
	/// `offset` is how far the handle sat from the pointer when it was taken hold of,
	/// kept for the length of the drag: grabbing the disc slightly off-centre should
	/// not tug the light sideways before the drag has begun.
	let aiming = $state<{ id: string; offset: { x: number; z: number } } | null>(null);
	let levelling = $state<{ id: string; clientY: number; from: number } | null>(null);
	let sheetFor = $state<string | null>(null);

	const sheetFixture = $derived(fixtures.find((f) => f.id === sheetFor) ?? null);
	/// Where the sheet sits on screen. Worked out from the viewBox rather than from a
	/// CTM, so panning and zooming move it without anything having to ask the DOM.
	const sheetAt = $derived.by(() => {
		const at = sheetFixture ? fixturePoint(sheetFixture) : null;
		if (!at) return null;
		return {
			left: ((at.x - view.x) / view.width) * box.w,
			top: ((at.z - view.z) / Math.max(height, 1e-6)) * box.h
		};
	});

	/// Where a pointer is, in metres. `getScreenCTM` already knows about the viewBox,
	/// the element's size and any page zoom, so nothing here has to.
	function atPointer(event: PointerEvent | MouseEvent): { x: number; z: number } | null {
		if (!svg) return null;
		const ctm = svg.getScreenCTM();
		if (!ctm) return null;
		const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(ctm.inverse());
		return { x: point.x, z: point.y };
	}

	/// The same place expressed as a pixel of the plan image, for calibration.
	function asPlanPixel(at: { x: number; z: number }) {
		if (!plan) return null;
		return planToPixel(plan, at);
	}

	function onBackgroundDown(event: PointerEvent) {
		const at = atPointer(event);
		if (!at) return;

		if (mode === 'origin' && plan) {
			const pixel = asPlanPixel(at);
			if (pixel) onorigin?.(pixel.px, pixel.py);
			return;
		}
		if (mode === 'scale' && plan) {
			const pixel = asPlanPixel(at);
			if (!pixel) return;
			if (!firstMark) {
				firstMark = pixel;
			} else {
				onmeasure?.(firstMark, pixel);
				firstMark = null;
			}
			return;
		}

		panning = { x: view.x, z: view.z, clientX: event.clientX, clientY: event.clientY };
		touched = true;
		svg?.setPointerCapture(event.pointerId);
	}

	function onMove(event: PointerEvent) {
		if (aiming) {
			const at = atPointer(event);
			if (at) aim(aiming.id, { x: at.x + aiming.offset.x, z: at.z + aiming.offset.z });
			return;
		}
		if (levelling) {
			// Up is brighter, and 150 pixels is the whole travel: far enough to be
			// controllable, close enough that a full fade is one gesture.
			const next = levelling.from + (levelling.clientY - event.clientY) / 150;
			setValue([levelling.id], 'Intensity', {
				type: 'Float',
				value: Math.min(1, Math.max(0, next))
			});
			return;
		}
		if (dragging) {
			const at = atPointer(event);
			if (at) onplace?.(dragging.id, at.x, at.z);
			return;
		}
		if (!panning || !svg) return;
		const perPixel = view.width / Math.max(svg.clientWidth, 1);
		view.x = panning.x - (event.clientX - panning.clientX) * perPixel;
		view.z = panning.z - (event.clientY - panning.clientY) * perPixel;
	}

	/// A chip from the "Not placed" tray, dragged onto the plan: the fixture is
	/// placed where it is dropped and becomes the selection, so the next thing the
	/// operator does applies to it.
	const CHIP = 'application/x-pult-fixture';

	function onChipDragStart(event: DragEvent, id: string) {
		if (!event.dataTransfer) return;
		event.dataTransfer.setData(CHIP, id);
		event.dataTransfer.effectAllowed = 'move';
	}

	function onDragOver(event: DragEvent) {
		if (!event.dataTransfer?.types.includes(CHIP)) return;
		event.preventDefault();
		event.dataTransfer.dropEffect = 'move';
	}

	function onDrop(event: DragEvent) {
		const id = event.dataTransfer?.getData(CHIP);
		if (!id) return;
		event.preventDefault();
		const at = atPointer(event);
		if (!at) return;
		onplace?.(id, at.x, at.z);
		select(id);
	}

	function onUp(event: PointerEvent) {
		dragging = null;
		panning = null;
		aiming = null;
		levelling = null;
		svg?.releasePointerCapture?.(event.pointerId);
	}

	/// Point a head at somewhere on the floor. The two angles that puts it there are
	/// what goes into the programmer — the plan is a way of naming them, not a
	/// separate thing to store.
	function aim(fixtureId: string, target: { x: number; z: number }) {
		const fixture = fixtures.find((f) => f.id === fixtureId);
		if (!fixture) return;
		const { pan, tilt } = aimAt(fixture, typeOf(fixture), { x: target.x, y: 0, z: target.z });
		if (pan !== null) setValue([fixtureId], 'Pan', { type: 'Float', value: pan });
		if (tilt !== null) setValue([fixtureId], 'Tilt', { type: 'Float', value: tilt });
	}

	const heldHere = (fixtureId: string, key: string) => $byKey.has(`${fixtureId}/${key}`);
	const levelOf = (fixtureId: string, fallback: number) => {
		const held = $byKey.get(`${fixtureId}/Intensity`)?.value as ParameterValue | undefined;
		return held?.type === 'Float' ? held.value : fallback;
	};

	/// Zoom about the pointer, so the thing under the cursor stays under it.
	function onWheel(event: WheelEvent) {
		event.preventDefault();
		touched = true;
		const at = atPointer(event);
		const factor = Math.exp(event.deltaY * 0.0015);
		const next = Math.min(400, Math.max(0.5, view.width * factor));
		if (at) {
			const scale = next / view.width;
			view.x = at.x - (at.x - view.x) * scale;
			view.z = at.z - (at.z - view.z) * scale;
		}
		view.width = next;
	}

	/** Frame everything that has been placed, or a room-sized box if nothing has. */
	export function fit() {
		// Pressing Fit is taking hold of the view as much as panning is: it says
		// where the operator wants to be looking.
		touched = true;
		frame();
	}

	function frame() {
		const bounds = fixtureBounds(placed);
		const spread = plan
			? {
					minX: Math.min(bounds.minX, plan.origin.x),
					maxX: Math.max(bounds.maxX, plan.origin.x + planExtent(plan).width),
					minZ: Math.min(bounds.minZ, plan.origin.z),
					maxZ: Math.max(bounds.maxZ, plan.origin.z + planExtent(plan).depth)
				}
			: bounds;
		// Wide enough for whichever way round the room is: fitting only the width
		// would cut the top and bottom off a deep stage seen on a wide screen.
		const aspect = box.h / Math.max(box.w, 1);
		const width = Math.max(spread.maxX - spread.minX, (spread.maxZ - spread.minZ) / aspect, 4);
		view.width = width;
		view.x = (spread.minX + spread.maxX) / 2 - width / 2;
		// Derived from the width just chosen, not the one still on screen.
		view.z = (spread.minZ + spread.maxZ) / 2 - (width * aspect) / 2;
	}

	$effect(() => {
		if (!svg) return;
		const observer = new ResizeObserver(([entry]) => {
			box = { w: entry.contentRect.width, h: entry.contentRect.height };
		});
		observer.observe(svg);
		return () => observer.disconnect();
	});

	/// Set the first time the operator pans or zooms. From then on the view is
	/// theirs, and a fixture arriving must not move it under them.
	let touched = $state(false);

	// Frame the rig until somebody takes hold of the view. Not once and never again:
	// a panel dragged into a taller tile is measured twice, and framing only the
	// first measurement leaves the rig in a corner of the second.
	$effect(() => {
		const ready = box.w > 1 && box.h > 1 && (placed.length > 0 || plan !== null);
		if (touched || !ready) return;
		untrack(() => frame());
	});

	const planCorners = $derived(plan ? planExtent(plan) : null);
	/// One grid line a metre, thinning out as the view widens so it never turns solid.
	const gridStep = $derived(view.width > 60 ? 10 : view.width > 20 ? 5 : 1);
	/// Handles are drawn in metres, so they have to be sized from the view or they
	/// vanish when it is zoomed out and swamp the rig when it is zoomed in.
	const handle = $derived(view.width / 90);

	/**
	 * A fraction of a circle, drawn as an arc from twelve o'clock clockwise.
	 *
	 * A whole turn is drawn as two half-arcs rather than one. An arc is defined by
	 * where it ends, so an arc that comes all the way round ends where it began and
	 * says nothing about which circle it meant. Anything within a couple of degrees
	 * of a full turn is close enough to the same ambiguity, and the difference is
	 * finer than the stroke can show, so it goes the same way.
	 */
	function arc(cx: number, cz: number, r: number, fraction: number): string {
		const turn = Math.min(1, Math.max(0, fraction));
		if (turn <= 0) return '';
		if (turn >= 0.995) {
			return `M ${cx} ${cz - r} A ${r} ${r} 0 1 1 ${cx} ${cz + r} A ${r} ${r} 0 1 1 ${cx} ${cz - r}`;
		}
		const angle = turn * Math.PI * 2;
		const sweep = turn > 0.5 ? 1 : 0;
		return `M ${cx} ${cz - r} A ${r} ${r} 0 ${sweep} 1 ${cx + r * Math.sin(angle)} ${cz - r * Math.cos(angle)}`;
	}
</script>

<div class="wrap">
	<svg
		bind:this={svg}
		use:asOneGesture
		viewBox="{view.x} {view.z} {view.width} {height}"
		class:measuring={mode === 'scale' || mode === 'origin'}
		onpointerdown={onBackgroundDown}
		onpointermove={onMove}
		onpointerup={onUp}
		onpointercancel={onUp}
		onwheel={onWheel}
		ondragover={onDragOver}
		ondrop={onDrop}
		role="application"
		aria-label="Stage plan"
	>
		<defs>
			<pattern
				id="grid"
				width={gridStep}
				height={gridStep}
				patternUnits="userSpaceOnUse"
			>
				<path class="grid-line" d="M {gridStep} 0 L 0 0 0 {gridStep}" fill="none" />
			</pattern>
		</defs>
		<rect
			x={view.x}
			y={view.z}
			width={view.width}
			height={height}
			fill="url(#grid)"
		/>

		{#if plan && planUrl && plan.visible && planCorners}
			<image
				href={planUrl}
				x={plan.origin.x}
				y={plan.origin.z}
				width={planCorners.width}
				height={planCorners.depth}
				opacity={plan.opacity}
				transform="rotate({plan.rotation_deg} {plan.origin.x} {plan.origin.z})"
				preserveAspectRatio="none"
			/>
		{/if}

		<!-- Where the show's own origin is, so a plan can be lined up against it. -->
		<g class="origin">
			<line x1={-0.6} y1={0} x2={0.6} y2={0} />
			<line x1={0} y1={-0.6} x2={0} y2={0.6} />
		</g>

		{#each symbols as symbol (symbol.fixture.id)}
			<g
				class="fixture"
				class:on={$selected.has(symbol.fixture.id)}
				transform="translate({symbol.at.x} {symbol.at.z})"
				onpointerdown={(e) => {
					e.stopPropagation();
					if (e.shiftKey) toggle(symbol.fixture.id);
					else select(symbol.fixture.id);
					if (mode === 'move') {
						dragging = { id: symbol.fixture.id };
						svg?.setPointerCapture(e.pointerId);
					}
				}}
				role="button"
				tabindex="0"
				aria-label={symbol.fixture.name}
			>
				{#if symbol.reach}
					<!-- Which way it is pointing, drawn as the beam it would throw. The
					     wedge reaches where the beam actually lands, so a head tilted
					     towards its own feet draws a short one. -->
					{@const angle = Math.atan2(symbol.reach.dx, symbol.reach.dz)}
					{@const length = Math.hypot(symbol.reach.dx, symbol.reach.dz)}
					<path
						class="beam"
						d="M 0 0 L {Math.sin(angle - 0.21) * length} {Math.cos(angle - 0.21) * length} A {length} {length} 0 0 1 {Math.sin(angle + 0.21) * length} {Math.cos(angle + 0.21) * length} Z"
						fill={symbol.tint}
					/>
				{/if}
				<!-- The body is the symbol and the fill is what it is doing, so a
				     fixture that is off still reads as a fixture rather than a hole. -->
				<circle class="body" r="0.35" fill={symbol.tint} />
				<circle class="ring" r="0.46" fill="none" />
				<text y="0.82" text-anchor="middle" font-size="0.3">{symbol.fixture.name}</text>
			</g>
		{/each}

		{#if mode === 'program'}
			{#each chosen as symbol (symbol.fixture.id)}
				{@const id = symbol.fixture.id}
				<g class="handles">
					<!-- The level, as a ring that fills up. Dragged vertically, because
					     that is which way a fader goes. -->
					<circle
						class="level-track"
						cx={symbol.at.x}
						cy={symbol.at.z}
						r={handle * 0.9}
						fill="none"
					/>
					<path
						class="level-fill"
						class:held={heldHere(id, 'Intensity')}
						d={arc(symbol.at.x, symbol.at.z, handle * 0.9, levelOf(id, symbol.level))}
						fill="none"
						role="slider"
						tabindex="-1"
						aria-label="{symbol.fixture.name} level"
						aria-valuemin="0"
						aria-valuemax="1"
						aria-valuenow={levelOf(id, symbol.level)}
						onpointerdown={(e) => {
							e.stopPropagation();
							levelling = { id, clientY: e.clientY, from: levelOf(id, symbol.level) };
							svg?.setPointerCapture(e.pointerId);
						}}
					/>
					<!-- A ring to grab even at zero, where the arc above has no length.
					     The arc says what the level is; this is what a hand lands on. -->
					<circle
						class="level-grab"
						cx={symbol.at.x}
						cy={symbol.at.z}
						r={handle * 0.9}
						fill="none"
						role="presentation"
						onpointerdown={(e) => {
							e.stopPropagation();
							levelling = { id, clientY: e.clientY, from: levelOf(id, symbol.level) };
							svg?.setPointerCapture(e.pointerId);
						}}
					/>

					{#if symbol.reach}
						{@const spot = { x: symbol.at.x + symbol.reach.dx, z: symbol.at.z + symbol.reach.dz }}
						<!-- Where the light lands, and the handle for putting it
						     somewhere else. Dragging this is the plan's puppeteering. -->
						<line class="tether" x1={symbol.at.x} y1={symbol.at.z} x2={spot.x} y2={spot.z} />
						<circle
							class="spot"
							class:held={heldHere(id, 'Pan') || heldHere(id, 'Tilt')}
							cx={spot.x}
							cy={spot.z}
							r={handle * 0.55}
							role="button"
							tabindex="-1"
							aria-label="Aim {symbol.fixture.name}"
							onpointerdown={(e) => {
								e.stopPropagation();
								const at = atPointer(e);
								aiming = {
									id,
									offset: at ? { x: spot.x - at.x, z: spot.z - at.z } : { x: 0, z: 0 }
								};
								svg?.setPointerCapture(e.pointerId);
							}}
						/>
					{/if}

					<!-- The colour, and the way into everything else this fixture has. -->
					<rect
						class="swatch"
						x={symbol.at.x + handle * 1.1}
						y={symbol.at.z - handle * 0.5}
						width={handle}
						height={handle}
						rx={handle * 0.2}
						fill={symbol.tint}
						role="button"
						tabindex="-1"
						aria-label="Open {symbol.fixture.name}"
						onpointerdown={(e) => {
							e.stopPropagation();
							sheetFor = sheetFor === id ? null : id;
						}}
					/>
				</g>
			{/each}
		{/if}

		{#if firstMark && plan}
			{@const first = pixelToPlan(plan, firstMark.px, firstMark.py)}
			<circle cx={first.x} cy={first.z} r={view.width / 220} class="mark" />
		{/if}
	</svg>

	{#if sheetFixture && sheetAt}
		<div class="sheet-at" style:left="{sheetAt.left}px" style:top="{sheetAt.top}px">
			<Quicksheet fixture={sheetFixture} onclose={() => (sheetFor = null)} />
		</div>
	{/if}

	{#if unplaced.length > 0}
		<footer class="tray">
			<span class="label">Not placed</span>
			{#each unplaced as fixture (fixture.id)}
				<button
					class="chip"
					class:on={$selected.has(fixture.id)}
					draggable="true"
					ondragstart={(e) => onChipDragStart(e, fixture.id)}
					onclick={(e) => (e.shiftKey ? toggle(fixture.id) : select(fixture.id))}
					ondblclick={() => onplace?.(fixture.id, 0, 0)}
					title="Drag it onto the plan, or double-click to drop it at the origin"
				>
					{fixture.name}
				</button>
			{/each}
		</footer>
	{/if}
</div>

<style>
	.wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; position: relative; }
	/* Dragging a handle must not sweep up the fixture labels as selected text. */
	svg { flex: 1; min-height: 0; width: 100%; background: #141414; touch-action: none; cursor: grab; user-select: none; }
	svg.measuring { cursor: crosshair; }

	/*
	 * Every stroke in this view is measured in screen pixels, not in metres, so a
	 * hairline stays a hairline however far the plan is zoomed. The alternative is
	 * what was here before: a dozen widths each worked out as some fraction of the
	 * view, all of them saying "about a pixel" the long way round.
	 */
	svg :is(circle, rect, line, path, text) { vector-effect: non-scaling-stroke; }

	/*
	 * And no focus ring drawn by the browser.
	 *
	 * A fixture is a focusable group, and Chrome rings a focused SVG element in the
	 * element's own coordinates — which here are metres. A clicked fixture wore a
	 * white-and-blue band several metres thick, shaped like the bounding box of its
	 * beam, over most of the room. Keyboard focus is shown by the ring the symbol
	 * already has, in units the view can make sense of.
	 */
	svg :focus { outline: none; }
	.fixture:focus-visible .ring { stroke: #fff; }

	.grid-line { stroke: #2a2a2a; stroke-width: 1; }
	.origin line { stroke: #4a9eff; stroke-width: 1.5; opacity: 0.7; }

	.fixture { cursor: pointer; }
	.fixture text { fill: #999; pointer-events: none; paint-order: stroke; stroke: #141414; stroke-width: 3; }
	.fixture .body { stroke: #8a8a8a; stroke-width: 1.5; }
	.fixture .ring { stroke: transparent; stroke-width: 2; }
	.fixture.on .ring { stroke: #4a9eff; }
	.fixture.on text { fill: #4a9eff; }
	/* A beam is a hint about where the light lands, not a render of it. */
	.beam { opacity: 0.16; pointer-events: none; }
	.mark { fill: #fbbf24; }

	/* Handles are amber when the programmer holds the parameter they move, which is
	   the same amber that marks a live cue: amber is what is reaching the rig. */
	.level-track, .level-fill, .level-grab { stroke-width: 6; }
	.level-track { stroke: #ffffff18; pointer-events: none; }
	.level-fill { stroke: #4a9eff; pointer-events: none; }
	.level-fill.held { stroke: #f59e0b; }
	.level-grab { stroke: transparent; cursor: ns-resize; }
	.tether { stroke: #4a9eff66; stroke-width: 1; stroke-dasharray: 4 4; pointer-events: none; }
	.spot { fill: #4a9eff33; stroke: #4a9eff; stroke-width: 1.5; cursor: move; }
	.spot.held { fill: #f59e0b33; stroke: #f59e0b; }
	.swatch { stroke: #ffffff55; stroke-width: 1; cursor: pointer; }

	.sheet-at { position: absolute; z-index: 5; transform: translate(-50%, calc(-100% - 18px)); }

	.tray { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 8px 14px; border-top: 1px solid #2a2a2a; flex: none; }
	.label { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; }
	.chip { background: #252525; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 3px 9px; font: inherit; font-size: 12px; cursor: pointer; }
	.chip:hover { border-color: #555; color: #fff; }
	.chip.on { border-color: #4a9eff; color: #4a9eff; }
</style>
