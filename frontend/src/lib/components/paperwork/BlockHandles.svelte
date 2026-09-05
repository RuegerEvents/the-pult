<script lang="ts">
	/**
	 * The blocks of a sheet, as things you can take hold of.
	 *
	 * An overlay of absolutely-positioned boxes on top of the preview rather than
	 * anything inside the SVG, for the reason the preview is a string at all: the
	 * drawing is a *rendering* of the model, and putting interaction inside it would
	 * mean the thing being dragged was the picture instead of the block. Here the
	 * geometry comes from `blocks[i].rect` and goes back to it, and the picture follows
	 * because it is drawn from the same field.
	 *
	 * # A drag is one act, and it commits per frame
	 *
	 * `beginGesture` when the pointer goes down, `endGesture` when it comes up, and one
	 * write per animation frame in between — which is `stores/editor.ts`'s rule for
	 * dragging a truss, applied to dragging a viewport. Writing per pointer event would
	 * be a few hundred writes across a sheet; writing only at the end would leave the
	 * preview frozen under the pointer, and the preview is the whole point.
	 */
	import type { Rect, SheetBlock } from '$lib/generated/index.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { resizedRect, writeBlock, type Grip } from '$lib/stores/paperwork.js';

	let {
		sheetId,
		blocks,
		zoom,
		paper,
		selected,
		onSelect
	}: {
		sheetId: string;
		blocks: SheetBlock[];
		/** Millimetres to CSS pixels: the preview's own scale. */
		zoom: number;
		/** The sheet's size in millimetres, so a block cannot be dragged off it. */
		paper: { w: number; h: number };
		selected: number | null;
		onSelect: (index: number | null) => void;
	} = $props();

	const CORNERS: { grip: Grip; style: string }[] = [
		{ grip: 'nw', style: 'left:-4px;top:-4px;cursor:nwse-resize' },
		{ grip: 'ne', style: 'right:-4px;top:-4px;cursor:nesw-resize' },
		{ grip: 'sw', style: 'left:-4px;bottom:-4px;cursor:nesw-resize' },
		{ grip: 'se', style: 'right:-4px;bottom:-4px;cursor:nwse-resize' },
		{ grip: 'n', style: 'left:50%;top:-4px;margin-left:-4px;cursor:ns-resize' },
		{ grip: 's', style: 'left:50%;bottom:-4px;margin-left:-4px;cursor:ns-resize' },
		{ grip: 'w', style: 'left:-4px;top:50%;margin-top:-4px;cursor:ew-resize' },
		{ grip: 'e', style: 'right:-4px;top:50%;margin-top:-4px;cursor:ew-resize' }
	];

	let drag: {
		index: number;
		grip: Grip;
		startX: number;
		startY: number;
		from: Rect;
		frame: number | null;
		pending: Rect | null;
	} | null = null;

	function begin(event: PointerEvent, index: number, grip: Grip) {
		event.preventDefault();
		event.stopPropagation();
		onSelect(index);
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		beginGesture();
		drag = {
			index,
			grip,
			startX: event.clientX,
			startY: event.clientY,
			from: { ...blocks[index].rect },
			frame: null,
			pending: null
		};
	}

	function moved(event: PointerEvent) {
		if (!drag) return;
		// In millimetres from the start, so the arithmetic is in the model's own units
		// and a zoom change mid-drag cannot make the block jump.
		const dx = (event.clientX - drag.startX) / zoom;
		const dy = (event.clientY - drag.startY) / zoom;
		drag.pending = resizedRect(drag.from, drag.grip, dx, dy, paper);
		if (drag.frame === null) {
			drag.frame = requestAnimationFrame(commit);
		}
	}

	function commit() {
		if (!drag) return;
		drag.frame = null;
		const rect = drag.pending;
		if (!rect) return;
		const block = blocks[drag.index];
		if (!block) return;
		// `gesture: false` — the gesture is the *drag*, opened at pointer-down. Letting
		// each frame open its own would make a two-second drag fifty acts.
		void writeBlock(sheetId, drag.index, { ...block, rect } as SheetBlock, false);
	}

	function end(event: PointerEvent) {
		if (!drag) return;
		if (drag.frame !== null) cancelAnimationFrame(drag.frame);
		commit();
		(event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
		drag = null;
		endGesture();
	}

	const label = (block: SheetBlock) =>
		block.type === 'Viewport' || block.type === 'Table' ? block.title || block.type : 'Text';
</script>

<!-- eslint-disable-next-line svelte/valid-compile -->
<div
	class="overlay"
	role="presentation"
	onpointerdown={() => onSelect(null)}
	onpointermove={moved}
	onpointerup={end}
	onpointercancel={end}
>
	{#each blocks as block, index (index)}
		<div
			class="block"
			class:selected={selected === index}
			style="left:{block.rect.x * zoom}px; top:{block.rect.y * zoom}px; width:{block.rect.w *
				zoom}px; height:{block.rect.h * zoom}px;"
			role="button"
			tabindex="0"
			aria-label={label(block)}
			onpointerdown={(e) => begin(e, index, 'move')}
			onkeydown={(e) => {
				if (e.key === 'Enter' || e.key === ' ') {
					e.preventDefault();
					onSelect(index);
				}
			}}
		>
			<span class="tag">{label(block)}</span>
			{#if selected === index}
				{#each CORNERS as corner (corner.grip)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<span
						class="grip"
						style={corner.style}
						onpointerdown={(e) => begin(e, index, corner.grip)}
					></span>
				{/each}
			{/if}
		</div>
	{/each}
</div>

<style>
	.overlay {
		position: absolute;
		inset: 0;
	}

	.block {
		position: absolute;
		border: 1px solid transparent;
		cursor: move;
	}

	.block:hover {
		border-color: rgba(74, 144, 217, 0.5);
	}

	.block.selected {
		border-color: var(--accent, #4a90d9);
		background: rgba(74, 144, 217, 0.06);
	}

	.tag {
		position: absolute;
		left: 0;
		top: -1.05rem;
		font-size: 0.62rem;
		color: var(--accent, #4a90d9);
		opacity: 0;
		white-space: nowrap;
		pointer-events: none;
	}

	.block:hover .tag,
	.block.selected .tag {
		opacity: 1;
	}

	.grip {
		position: absolute;
		width: 8px;
		height: 8px;
		background: var(--accent, #4a90d9);
		border: 1px solid #fff;
		border-radius: 1px;
	}
</style>
