<script lang="ts">
	/**
	 * The context menu: one, at the root, for every object.
	 *
	 * Mounted once rather than per card, so a menu cannot be clipped by the tile it was
	 * opened in, and so there is never more than one open. The verbs are the kind's
	 * (`verbsFor`), in the same order everywhere, with Delete last; under the Show lock
	 * only acts are offered. Color is a row of swatches rather than a verb that opens
	 * something else, because painting a preset should be one touch.
	 */

	import { closeMenu, kindOf, menu, rowOf, SWATCHES, verbsFor, type Verb } from '$lib/objects.js';
	import { showLocked } from '$lib/stores/showLock.js';
	import { addToast } from '$lib/toasts.js';

	const kind = $derived($menu ? kindOf($menu.ref.kind) : undefined);
	const row = $derived($menu ? rowOf($menu.ref) : undefined);
	const verbs = $derived($menu ? verbsFor($menu.ref, $showLocked) : []);
	const paints = $derived(!$showLocked && !!kind?.recolor && !!row);

	/** Kept on screen: a menu opened near the bottom-right corner opens up and left. */
	const place = $derived.by(() => {
		if (!$menu) return { left: 0, top: 0 };
		const w = 220;
		const h = 44 + verbs.length * 34 + (paints ? 40 : 0);
		const left = Math.min($menu.x, window.innerWidth - w - 8);
		const top = $menu.y + h > window.innerHeight - 8 ? Math.max(8, $menu.y - h) : $menu.y;
		return { left, top };
	});

	async function run(verb: Verb) {
		closeMenu();
		try {
			await verb.run();
		} catch (e) {
			addToast(`${verb.label}: ${e}`);
		}
	}

	async function paint(color: string | null) {
		const r = row;
		closeMenu();
		if (r && kind?.recolor) await kind.recolor(r, color);
	}

	function onkey(event: KeyboardEvent) {
		if ($menu && event.key === 'Escape') {
			event.stopPropagation();
			closeMenu();
		}
	}
</script>

<svelte:window onkeydown={onkey} />

{#if $menu && row}
	<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
	<div class="catch" onclick={closeMenu} oncontextmenu={(e) => { e.preventDefault(); closeMenu(); }}></div>
	<div
		class="menu"
		role="menu"
		tabindex="-1"
		style:left="{place.left}px"
		style:top="{place.top}px"
	>
		<header>{kind?.label ?? ''} · {row.name}</header>
		{#each verbs as verb (verb.id)}
			<button
				role="menuitem"
				class:danger={verb.danger}
				disabled={!!verb.disabled}
				title={verb.disabled || undefined}
				onclick={() => run(verb)}>{verb.label}</button
			>
		{/each}
		{#if paints}
			<div class="swatches" role="group" aria-label="Color">
				{#each SWATCHES as swatch (swatch ?? 'none')}
					<button
						class="swatch"
						class:none={swatch === null}
						class:on={(row.color ?? null) === swatch}
						style:background={swatch ?? 'transparent'}
						aria-label={swatch ?? 'Its category color'}
						title={swatch ?? 'Its category color'}
						onclick={() => paint(swatch)}
					></button>
				{/each}
			</div>
		{/if}
	</div>
{/if}

<style>
	.catch {
		position: fixed;
		inset: 0;
		z-index: 60;
	}
	.menu {
		position: fixed;
		z-index: 61;
		display: flex;
		flex-direction: column;
		width: 220px;
		padding: 4px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		box-shadow: 0 10px 30px rgb(0 0 0 / 55%);
	}
	header {
		font-size: var(--font-xs);
		color: var(--text-faint);
		padding: 6px 8px 5px;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	button[role='menuitem'] {
		text-align: left;
		min-height: 32px;
		padding: 0 10px;
		background: none;
		border: none;
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	button[role='menuitem']:hover:not(:disabled),
	button[role='menuitem']:focus-visible {
		background: var(--bg-hover);
		color: var(--text-bright);
		outline: none;
	}
	button[role='menuitem']:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
	.danger:hover:not(:disabled) {
		color: var(--bad) !important;
	}
	.swatches {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
		padding: 8px;
		border-top: 1px solid var(--line);
		margin-top: 4px;
	}
	.swatch {
		width: 18px;
		height: 18px;
		border-radius: 50%;
		border: 1px solid var(--line-input);
		cursor: pointer;
	}
	.swatch.none {
		background: repeating-linear-gradient(45deg, #444 0 3px, transparent 3px 6px) !important;
	}
	.swatch.on {
		outline: 2px solid var(--text-bright);
		outline-offset: 1px;
	}
</style>
