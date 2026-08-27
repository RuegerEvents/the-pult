<script lang="ts">
	/**
	 * Where a dragged tab can land: the middle of a tile, or any of its four edges.
	 *
	 * Only on screen while something is being dragged, and only then does anything
	 * here take a pointer — a tile covered in invisible targets would be a tile
	 * nobody could click.
	 */

	import type { DropSide, Path } from '$lib/layout.js';
	import { dropId, dropTarget } from '$lib/stores/layout.js';

	let { path }: { path: Path } = $props();

	const sides: DropSide[] = ['center', 'left', 'right', 'top', 'bottom'];
</script>

<div class="zones">
	{#each sides as side (side)}
		<div class="zone {side}" class:on={$dropTarget === dropId(path, side)} data-drop={dropId(path, side)}></div>
	{/each}
</div>

<style>
	.zones {
		position: absolute;
		inset: 0;
		z-index: 20;
	}

	.zone {
		position: absolute;
		transition: background 0.08s;
	}
	.zone.on {
		background: #4a9eff33;
		box-shadow: inset 0 0 0 1px var(--accent);
	}

	.center {
		inset: 28% 28%;
	}
	.left {
		inset: 0 72% 0 0;
	}
	.right {
		inset: 0 0 0 72%;
	}
	.top {
		inset: 0 0 72% 0;
	}
	.bottom {
		inset: 72% 0 0 0;
	}
</style>
