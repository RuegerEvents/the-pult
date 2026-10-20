<script lang="ts">
	/**
	 * One object, drawn the same way in every pool.
	 *
	 * A name; an edge in its own color, or its category's when nobody has painted it;
	 * a state line saying what it is doing or how much of it applies; a progress bar
	 * where something is moving. Every pool, board and list of objects is made of these,
	 * which is what makes them one system rather than five that each grew their own
	 * idea of what a click does. The grammar itself is `$lib/objects.ts`'s:
	 *
	 * - a tap **acts** (the kind's `act`, or `onact` when the panel needs to say more);
	 * - ⌘/Ctrl-click, or the corner dot, makes it the **subject** for the Inspector —
	 *   the dot is there for a finger and a pen, which have no modifier keys, and only
	 *   shows on a coarse pointer or under the mouse;
	 * - right-click or a long-press opens the **context menu**;
	 * - it can be **dragged** onto something that takes it, unless the Show lock is on;
	 * - and a card with a `level` takes a **sideways drag** as that level.
	 */

	import type { Snippet } from 'svelte';
	import { longPress } from '$lib/actions.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import {
		isSubject,
		kindOf,
		OBJECT_MIME,
		openMenu,
		renaming,
		rowOf,
		selectSubject,
		subject,
		type ObjectRef
	} from '$lib/objects.js';
	import { canEdit } from '$lib/stores/showLock.js';

	let {
		ref,
		name,
		color = null,
		state = '',
		progress = null,
		level = null,
		onlevel,
		onact,
		dim = false,
		on = false,
		children
	}: {
		ref: ObjectRef;
		name: string;
		/** The object's own color; `null` draws its category's. */
		color?: string | null;
		/** One line under the name: "running", "4 of 6", "I · C". */
		state?: string;
		/** 0..1 when something is moving, `null` otherwise. */
		progress?: number | null;
		/** 0..1 when the card has a level a sideways drag sets. */
		level?: number | null;
		onlevel?: (level: number) => void;
		/** What a tap does, when the kind's own `act` is not the whole story. */
		onact?: () => void;
		/** Drawn faint: it would do nothing right now. */
		dim?: boolean;
		/** Drawn as the one somebody is looking at, in its panel's own sense. */
		on?: boolean;
		children?: Snippet;
	} = $props();

	const kind = $derived(kindOf(ref.kind));
	const edge = $derived(color ?? categoryColor(kind?.category));
	const chosen = $derived(isSubject($subject, ref));
	const editingName = $derived(
		$renaming !== null && $renaming.kind === ref.kind && $renaming.id === ref.id
	);

	/** A sideways drag in progress, so the click it ends in is not also a tap. */
	let sliding = false;

	function act(event: MouseEvent) {
		if (sliding) {
			sliding = false;
			return;
		}
		if (event.metaKey || event.ctrlKey) {
			selectSubject(ref, true);
			return;
		}
		if (onact) return onact();
		const row = rowOf(ref);
		if (row && kind?.act) void kind.act(row);
	}

	function menuAt(event: MouseEvent) {
		event.preventDefault();
		openMenu(ref, event.clientX, event.clientY);
	}

	function dragStart(event: DragEvent) {
		if (!$canEdit || level !== null || !event.dataTransfer) {
			event.preventDefault();
			return;
		}
		event.dataTransfer.setData(OBJECT_MIME, JSON.stringify(ref));
		event.dataTransfer.setData('text/plain', name);
		event.dataTransfer.effectAllowed = 'copyLink';
	}

	/** Level: the pointer's travel across the card's width, from where it started. */
	function levelDown(event: PointerEvent) {
		if (level === null || !onlevel) return;
		const card = event.currentTarget as HTMLElement;
		const width = card.getBoundingClientRect().width || 1;
		const from = event.clientX;
		const start = level;
		let moved = false;
		const move = (e: PointerEvent) => {
			const dx = e.clientX - from;
			if (!moved && Math.abs(dx) < 6) return;
			moved = true;
			sliding = true;
			onlevel(Math.max(0, Math.min(1, start + dx / width)));
		};
		const up = () => {
			window.removeEventListener('pointermove', move);
			window.removeEventListener('pointerup', up);
		};
		window.addEventListener('pointermove', move);
		window.addEventListener('pointerup', up);
	}

	async function commitName(value: string) {
		renaming.set(null);
		const next = value.trim();
		const row = rowOf(ref);
		if (next && row && next !== row.name && kind?.rename) await kind.rename(row, next);
	}
</script>

<div
	class="card"
	class:dim
	class:on
	class:chosen
	style:--edge={edge}
	role="button"
	tabindex="0"
	draggable={$canEdit && level === null && !editingName}
	ondragstart={dragStart}
	onclick={act}
	onkeydown={(e) => {
		if (editingName) return;
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			if (onact) onact();
			else {
				const row = rowOf(ref);
				if (row && kind?.act) void kind.act(row);
			}
		}
		if (e.key === 'ContextMenu') {
			const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
			openMenu(ref, box.left, box.bottom);
		}
	}}
	oncontextmenu={menuAt}
	onpointerdown={levelDown}
	use:longPress={(x, y) => openMenu(ref, x, y)}
>
	{#if level !== null}
		<span class="level" style:width="{level * 100}%"></span>
	{/if}

	{#if editingName}
		<!-- svelte-ignore a11y_autofocus -->
		<input
			class="input rename"
			value={name}
			autofocus
			onclick={(e) => e.stopPropagation()}
			onkeydown={(e) => {
				e.stopPropagation();
				if (e.key === 'Enter') void commitName(e.currentTarget.value);
				if (e.key === 'Escape') renaming.set(null);
			}}
			onblur={(e) => void commitName(e.currentTarget.value)}
		/>
	{:else}
		<span class="name">{name}</span>
	{/if}
	{#if state}<span class="state">{state}</span>{/if}
	{#if children}{@render children()}{/if}

	{#if progress !== null}
		<span class="progress" style:width="{Math.max(0, Math.min(1, progress)) * 100}%"></span>
	{/if}

	<button
		class="dot"
		class:chosen
		aria-label={chosen ? `Stop inspecting ${name}` : `Inspect ${name}`}
		title="Inspect"
		onclick={(e) => {
			e.stopPropagation();
			selectSubject(ref, true);
		}}
	></button>
</div>

<style>
	.card {
		position: relative;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		justify-content: center;
		gap: 3px;
		min-width: 7.5rem;
		min-height: 48px;
		padding: 7px 26px 9px 11px;
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-left: 3px solid var(--edge);
		border-radius: var(--radius);
		color: var(--text);
		font-size: var(--font-sm);
		text-align: left;
		cursor: pointer;
		overflow: hidden;
		user-select: none;
		touch-action: pan-y;
		transition:
			border-color 0.1s,
			background 0.1s;
	}
	.card:hover {
		background: var(--bg-hover);
		border-color: var(--line-input);
		border-left-color: var(--edge);
	}
	.card:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}
	.card.on {
		border-color: var(--edge);
	}
	.card.chosen {
		box-shadow: 0 0 0 2px var(--accent);
	}
	.card.dim .name {
		color: var(--text-faint);
	}

	.name {
		max-width: 13rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		position: relative;
	}
	.state {
		font-size: var(--font-xs);
		color: var(--text-dim);
		white-space: nowrap;
		position: relative;
	}

	.rename {
		width: 100%;
		min-width: 8rem;
		position: relative;
	}

	.progress {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 3px;
		background: var(--edge);
	}
	.level {
		position: absolute;
		inset: 0 auto 0 0;
		background: color-mix(in srgb, var(--edge) 18%, transparent);
		pointer-events: none;
	}

	/* The corner dot: a finger's and a pen's way to say "inspect this". Under a mouse it
	   appears on hover, beside ⌘-click; on a touch screen it is always there. */
	.dot {
		position: absolute;
		top: 6px;
		right: 6px;
		width: 14px;
		height: 14px;
		padding: 0;
		border-radius: 50%;
		border: 1.5px solid var(--text-faint);
		background: none;
		cursor: pointer;
		opacity: 0;
	}
	.card:hover .dot,
	.dot:focus-visible,
	.dot.chosen {
		opacity: 1;
	}
	.dot.chosen {
		background: var(--accent);
		border-color: var(--accent);
	}
	@media (pointer: coarse) {
		.dot {
			opacity: 1;
			width: 18px;
			height: 18px;
		}
	}
</style>
