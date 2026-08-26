<script lang="ts">
	import { untrack } from 'svelte';
	import type { Fixture, FixtureType, StagePlan } from '$lib/generated/index.js';
	import {
		fixtureBounds,
		fixturePoint,
		fixtureTint,
		panAngle,
		pixelToPlan,
		planExtent,
		planToPixel
	} from '$lib/stage.js';
	import { selected, select, toggle } from '$lib/stores/selection.js';

	type Mode = 'move' | 'scale' | 'origin';

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

	/// Two clicks make a measurement; the first is held here until the second lands.
	let firstMark = $state<{ px: number; py: number } | null>(null);
	let dragging = $state<{ id: string } | null>(null);
	let panning = $state<{ x: number; z: number; clientX: number; clientY: number } | null>(null);

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
		svg?.setPointerCapture(event.pointerId);
	}

	function onMove(event: PointerEvent) {
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

	function onUp(event: PointerEvent) {
		dragging = null;
		panning = null;
		svg?.releasePointerCapture?.(event.pointerId);
	}

	/// Zoom about the pointer, so the thing under the cursor stays under it.
	function onWheel(event: WheelEvent) {
		event.preventDefault();
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

	// Frame the rig the first time there is one, and never again — after that the
	// view is the operator's, and a fixture arriving must not move it under them.
	let framed = $state(false);
	$effect(() => {
		const ready = box.w > 1 && (placed.length > 0 || plan !== null);
		if (framed || !ready) return;
		framed = true;
		untrack(() => fit());
	});

	const planCorners = $derived(plan ? planExtent(plan) : null);
	/// One grid line a metre, thinning out as the view widens so it never turns solid.
	const gridStep = $derived(view.width > 60 ? 10 : view.width > 20 ? 5 : 1);
</script>

<div class="wrap">
	<svg
		bind:this={svg}
		viewBox="{view.x} {view.z} {view.width} {height}"
		class:measuring={mode !== 'move'}
		onpointerdown={onBackgroundDown}
		onpointermove={onMove}
		onpointerup={onUp}
		onpointercancel={onUp}
		onwheel={onWheel}
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
				<path d="M {gridStep} 0 L 0 0 0 {gridStep}" fill="none" stroke="#2a2a2a" stroke-width={view.width / 2000} />
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
		<g class="origin" stroke-width={view.width / 900}>
			<line x1={-0.6} y1={0} x2={0.6} y2={0} />
			<line x1={0} y1={-0.6} x2={0} y2={0.6} />
		</g>

		{#each placed as fixture (fixture.id)}
			{@const at = fixturePoint(fixture)}
			{@const angle = panAngle(fixture, typeOf(fixture))}
			{@const tint = fixtureTint(fixture)}
			{#if at}
				<g
					class="fixture"
					class:on={$selected.has(fixture.id)}
					transform="translate({at.x} {at.z})"
					onpointerdown={(e) => {
						e.stopPropagation();
						if (e.shiftKey) toggle(fixture.id);
						else select(fixture.id);
						if (mode === 'move') {
							dragging = { id: fixture.id };
							svg?.setPointerCapture(e.pointerId);
						}
					}}
					role="button"
					tabindex="0"
					aria-label={fixture.name}
				>
					{#if angle !== null}
						<!-- Which way it is pointing, drawn as the beam it would throw. -->
						<path
							class="beam"
							d="M 0 0 L {Math.sin(((angle - 12) * Math.PI) / 180) * 3} {Math.cos(((angle - 12) * Math.PI) / 180) * 3} A 3 3 0 0 1 {Math.sin(((angle + 12) * Math.PI) / 180) * 3} {Math.cos(((angle + 12) * Math.PI) / 180) * 3} Z"
							fill={tint}
						/>
					{/if}
					<!-- The body is the symbol and the fill is what it is doing, so a
					     fixture that is off still reads as a fixture rather than a hole. -->
					<circle class="body" r="0.35" fill={tint} stroke-width="0.07" />
					<circle class="ring" r="0.46" fill="none" stroke-width="0.08" />
					<text y="0.82" text-anchor="middle" font-size="0.3">{fixture.name}</text>
				</g>
			{/if}
		{/each}

		{#if firstMark && plan}
			{@const first = pixelToPlan(plan, firstMark.px, firstMark.py)}
			<circle cx={first.x} cy={first.z} r={view.width / 220} class="mark" />
		{/if}
	</svg>

	{#if unplaced.length > 0}
		<footer class="tray">
			<span class="label">Not placed</span>
			{#each unplaced as fixture (fixture.id)}
				<button
					class="chip"
					class:on={$selected.has(fixture.id)}
					onclick={() => select(fixture.id)}
					ondblclick={() => onplace?.(fixture.id, 0, 0)}
					title="Double-click to drop it at the origin, then drag it into place"
				>
					{fixture.name}
				</button>
			{/each}
		</footer>
	{/if}
</div>

<style>
	.wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	svg { flex: 1; min-height: 0; width: 100%; background: #141414; touch-action: none; cursor: grab; }
	svg.measuring { cursor: crosshair; }

	.origin line { stroke: #4a9eff; opacity: 0.7; }

	.fixture { cursor: pointer; }
	.fixture text { fill: #999; pointer-events: none; paint-order: stroke; stroke: #141414; stroke-width: 0.06; }
	.fixture .body { stroke: #8a8a8a; }
	.fixture .ring { stroke: transparent; }
	.fixture.on .ring { stroke: #4a9eff; }
	.fixture.on text { fill: #4a9eff; }
	/* A beam is a hint about where the light lands, not a render of it. */
	.beam { opacity: 0.16; pointer-events: none; }
	.mark { fill: #fbbf24; }

	.tray { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding: 8px 14px; border-top: 1px solid #2a2a2a; flex: none; }
	.label { color: #777; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; }
	.chip { background: #252525; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 3px 9px; font: inherit; font-size: 12px; cursor: pointer; }
	.chip:hover { border-color: #555; color: #fff; }
	.chip.on { border-color: #4a9eff; color: #4a9eff; }
</style>
