<script lang="ts">
	/**
	 * A layout, full-screen: what a phone at the maze entrance is handed to see where the
	 * groups are.
	 *
	 * `/layout/<id>` is one layout and `/layout` the list of them — the surface page's
	 * shape, and for the surface page's reasons: the station serves the app for any path
	 * it has no file for, the root layout leaves this page with no top bar and no cover,
	 * and when the console goes away the drawing stays, dimmed, under a banner that says
	 * so, until the socket's own reconnect brings it back.
	 *
	 * **Nothing on it presses anything.** A layout's presses are the desk's; this page
	 * only looks. What it does with a finger is look closer: pinch to zoom about the
	 * fingers, drag to move about, double-tap to fit the whole drawing again — and turning
	 * the phone refits it, unless somebody has zoomed in to look at one corner.
	 */

	import { page } from '$app/state';
	import LayoutDrawing from '$lib/components/layouts/LayoutDrawing.svelte';
	import {
		clampView,
		fitView,
		isFitted,
		layoutUrl,
		zoomAt,
		type Size,
		type View
	} from '$lib/layoutView.js';
	import { connection } from '$lib/stores/connection.js';
	import { layouts } from '$lib/stores/layouts.js';
	import { surfaceShow } from '$lib/stores/surface.js';
	import { getClientContext, getStationContext } from '$lib/ws/context.js';
	import Maximize from '@lucide/svelte/icons/maximize';
	import Minimize from '@lucide/svelte/icons/minimize';

	const client = getClientContext();
	const station = getStationContext();

	const id = $derived(page.params.id ?? null);
	const layout = $derived(id ? ($layouts.find((l) => l.id === id) ?? null) : null);
	const live = $derived($connection.connected);

	const linkTo = (layoutId: string) =>
		new URL(layoutUrl({ origin: page.url.origin, search: page.url.search }, layoutId)).pathname + page.url.search;

	$effect(() => {
		document.title = layout ? `${layout.name} — the-pult` : 'Layouts — the-pult';
	});

	// ── Full screen ──────────────────────────────────────────────────────────
	//
	// The page itself, with the browser's bars gone: the Fullscreen API where there is
	// one (Android, an iPad, a desk), prefixed on older WebKit. An iPhone has none for
	// anything but a video, so there the button says how to get the same thing — added
	// to the Home Screen, the page opens without Safari's bars (`app.html` says it may).

	type Prefixed = Document & { webkitFullscreenEnabled?: boolean; webkitFullscreenElement?: Element | null; webkitExitFullscreen?: () => void };
	const doc = () => document as Prefixed;
	const canFullscreen = () => !!(doc().fullscreenEnabled || doc().webkitFullscreenEnabled);
	let full = $state(false);
	let howTo = $state(false);
	/** Opened from the Home Screen: already without the browser's bars. */
	const standalone =
		typeof window !== 'undefined' &&
		(window.matchMedia?.('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true);

	$effect(() => {
		const note = () => (full = !!(doc().fullscreenElement || doc().webkitFullscreenElement));
		document.addEventListener('fullscreenchange', note);
		document.addEventListener('webkitfullscreenchange', note);
		return () => {
			document.removeEventListener('fullscreenchange', note);
			document.removeEventListener('webkitfullscreenchange', note);
		};
	});

	async function toggleFullscreen() {
		if (!canFullscreen()) {
			howTo = !howTo;
			return;
		}
		const d = doc();
		if (d.fullscreenElement || d.webkitFullscreenElement) {
			if (d.exitFullscreen) await d.exitFullscreen().catch(() => {});
			else d.webkitExitFullscreen?.();
			return;
		}
		const root = document.documentElement as HTMLElement & { webkitRequestFullscreen?: () => void };
		if (root.requestFullscreen) await root.requestFullscreen().catch(() => (howTo = true));
		else root.webkitRequestFullscreen?.();
	}

	// ── Fitting, and looking closer ──────────────────────────────────────────

	let stage = $state<HTMLDivElement | null>(null);
	let screen = $state<Size>({ width: 0, height: 0 });
	let view = $state<View | null>(null);
	/** Whether the whole drawing is in view, so a rotation knows to refit it. */
	let fitted = true;

	const canvas = $derived<Size>(layout ? { width: layout.width, height: layout.height } : { width: 1, height: 1 });

	$effect(() => {
		if (!stage) return;
		const observer = new ResizeObserver(([entry]) => {
			const next = { width: entry.contentRect.width, height: entry.contentRect.height };
			const turned = next.width > next.height !== screen.width > screen.height;
			screen = next;
			// A phone turned, or a drawing nobody zoomed into: the whole of it again. A
			// zoomed one keeps where it is looking — a URL bar sliding away is a resize too.
			view = !view || fitted || turned ? fitView(canvas, next) : clampView(view, canvas, next);
			fitted = isFitted(view, canvas, next);
		});
		observer.observe(stage);
		return () => observer.disconnect();
	});

	// A different layout, or this one given another size: start from the whole of it.
	$effect(() => {
		void canvas.width;
		void canvas.height;
		if (screen.width > 0) {
			view = fitView(canvas, screen);
			fitted = true;
		}
	});

	const pointers = new Map<number, { x: number; y: number }>();
	let pinch: { distance: number; mid: { x: number; y: number }; from: View } | null = null;
	let pan: { x: number; y: number; from: View } | null = null;
	let lastTap: { at: number; x: number; y: number } | null = null;
	let travelled = 0;

	const local = (event: PointerEvent | WheelEvent) => {
		const bounds = stage!.getBoundingClientRect();
		return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
	};
	const spread = () => {
		const [a, b] = [...pointers.values()];
		return { distance: Math.hypot(a.x - b.x, a.y - b.y) || 1, mid: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } };
	};

	function down(event: PointerEvent) {
		if (!view) return;
		stage!.setPointerCapture(event.pointerId);
		pointers.set(event.pointerId, local(event));
		if (pointers.size === 1) {
			travelled = 0;
			const at = local(event);
			pan = { x: at.x, y: at.y, from: view };
		} else if (pointers.size === 2) {
			pan = null;
			const { distance, mid } = spread();
			pinch = { distance, mid, from: view };
		}
	}

	function move(event: PointerEvent) {
		if (!view || !pointers.has(event.pointerId)) return;
		const at = local(event);
		const before = pointers.get(event.pointerId)!;
		travelled += Math.hypot(at.x - before.x, at.y - before.y);
		pointers.set(event.pointerId, at);
		if (pinch && pointers.size >= 2) {
			const { distance, mid } = spread();
			const zoomed = zoomAt(pinch.from, distance / pinch.distance, pinch.mid, canvas, screen);
			view = clampView({ ...zoomed, x: zoomed.x + mid.x - pinch.mid.x, y: zoomed.y + mid.y - pinch.mid.y }, canvas, screen);
		} else if (pan) {
			view = clampView({ ...pan.from, x: pan.from.x + at.x - pan.x, y: pan.from.y + at.y - pan.y }, canvas, screen);
		}
		fitted = isFitted(view, canvas, screen);
	}

	function up(event: PointerEvent) {
		if (!pointers.has(event.pointerId)) return;
		const at = pointers.get(event.pointerId)!;
		pointers.delete(event.pointerId);
		if (pointers.size < 2) pinch = null;
		if (pointers.size === 1 && view) {
			// One finger lifted off a pinch: the other carries on as a drag from here.
			const [rest] = [...pointers.values()];
			pan = { x: rest.x, y: rest.y, from: view };
		}
		if (pointers.size > 0) return;
		pan = null;
		if (travelled > 10) return;
		const now = performance.now();
		if (lastTap && now - lastTap.at < 320 && Math.hypot(at.x - lastTap.x, at.y - lastTap.y) < 40) {
			view = fitView(canvas, screen);
			fitted = true;
			lastTap = null;
		} else {
			lastTap = { at: now, x: at.x, y: at.y };
		}
	}

	/** A wheel or a trackpad, for the page opened on a laptop. */
	function wheel(event: WheelEvent) {
		if (!view) return;
		event.preventDefault();
		view = zoomAt(view, Math.exp(-event.deltaY * 0.002), local(event), canvas, screen);
		fitted = isFitted(view, canvas, screen);
	}
</script>

<div class="phone">
	<header class="bar">
		<span class="title">{layout?.name ?? 'Layouts'}</span>
		<span class="status" class:live>
			<span class="dot"></span>{live ? 'Connected' : 'Not connected'}
		</span>
		{#if !standalone}
			<button
				class="full"
				aria-label={full ? 'Leave full screen' : 'Full screen'}
				title={full ? 'Leave full screen' : 'Full screen'}
				onclick={toggleFullscreen}
			>
				{#if full}<Minimize size={18} />{:else}<Maximize size={18} />{/if}
			</button>
		{/if}
	</header>
	{#if howTo}
		<div class="how-to" role="note">
			<span>This browser cannot go full screen. Tap <strong>Share</strong>, then <strong>Add to Home Screen</strong>, and open the layout from there: it runs without the browser's bars.</span>
			<button onclick={() => (howTo = false)}>OK</button>
		</div>
	{/if}

	{#if !live && $connection.everConnected}
		<div class="lost" role="alert">
			<strong>Not connected to the console.</strong>
			<span>What you see is what it last said, and it is not changing until it is back.</span>
			<button class="retry" onclick={() => client.retryNow()}>Try again now</button>
		</div>
	{/if}

	{#if $station && !$station.show}
		<div class="body"><p class="note">This console has no show open.</p></div>
	{:else if layout}
		<div
			class="stage"
			class:stale={!live}
			bind:this={stage}
			role="img"
			aria-label={layout.name}
			onpointerdown={down}
			onpointermove={move}
			onpointerup={up}
			onpointercancel={up}
			onwheel={wheel}
		>
			{#if view && screen.width > 0}
				<LayoutDrawing {layout} show={$surfaceShow} {view} {screen} />
			{/if}
		</div>
	{:else}
		<div class="body">
			{#if id}
				<p class="note">This show has no layout with that link. Pick one:</p>
			{/if}
			{#if $layouts.length === 0}
				<p class="note">This show has no layouts yet. Make one in the Layouts panel on a desk.</p>
			{/if}
			<nav class="list">
				{#each $layouts as l (l.id)}
					<a class="pick" href={linkTo(l.id)}>{l.name}</a>
				{/each}
			</nav>
		</div>
	{/if}
</div>

<style>
	.phone {
		height: 100%;
		display: flex;
		flex-direction: column;
		background: #0f1114;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px 14px;
		padding-top: max(10px, env(safe-area-inset-top));
		background: #0d0d0d;
		border-bottom: 1px solid #2a2a2a;
		flex-shrink: 0;
	}
	.title {
		flex: 1;
		font-size: 17px;
		font-weight: 700;
		color: #fff;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.status {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		color: #f87171;
	}
	.status.live {
		color: #86efac;
	}
	.full {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		margin: -6px -6px -6px 0;
		padding: 0;
		background: transparent;
		border: 1px solid #333;
		border-radius: 8px;
		color: #ddd;
	}
	.how-to {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px 14px;
		font-size: 14px;
		line-height: 1.4;
		color: #e5e7eb;
		background: #1f2937;
		border-bottom: 1px solid #2a2a2a;
	}
	.how-to button {
		padding: 6px 12px;
		border-radius: 6px;
		border: 1px solid #444;
		background: #111;
		color: #fff;
	}
	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
		background: currentColor;
	}
	.lost {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 12px 14px;
		background: #7f1d1d;
		color: #fee2e2;
		font-size: 14px;
		line-height: 1.35;
		flex-shrink: 0;
	}
	.retry {
		align-self: flex-start;
		min-height: 44px;
		padding: 0 16px;
		border-radius: 8px;
		border: 1px solid #fca5a5;
		background: transparent;
		color: #fff;
		font: inherit;
		font-weight: 600;
	}
	/* The drawing takes every pixel the bar leaves, and owns every gesture on it: the
	   browser's own pinch would zoom the bar and the banner with it. */
	.stage {
		flex: 1;
		min-height: 0;
		overflow: hidden;
		touch-action: none;
		padding-bottom: env(safe-area-inset-bottom);
	}
	.stage.stale :global(svg) {
		opacity: 0.45;
		filter: grayscale(0.6);
	}
	.body {
		flex: 1;
		overflow-y: auto;
		padding: 12px;
		padding-bottom: max(12px, env(safe-area-inset-bottom));
	}
	.note {
		color: #999;
		font-size: 15px;
		line-height: 1.4;
		margin-bottom: 12px;
	}
	.list {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.pick {
		display: flex;
		align-items: center;
		min-height: 64px;
		padding: 0 16px;
		border-radius: 10px;
		background: #2d3a4f;
		color: #fff;
		font-size: 17px;
		font-weight: 600;
		text-decoration: none;
	}
</style>
