<script lang="ts">
	/**
	 * The wing's pages, drawn: a tab per page, its name, and the playback section as it
	 * sits on the desk — ten fader columns, each with the key above it and the key under
	 * it, and the sixteen X keys — with what each control does on that page written on
	 * it. So a page can be read without turning the wing to it.
	 *
	 * Looking at a page here does not turn the wing; PAGE+ and PAGE− do that. The tab
	 * the wing is on is marked, and this follows it until another tab is clicked.
	 */

	import type { BindingAction, ControlBinding, SurfaceControl } from '$lib/generated/index.js';
	import { playbackLayout, type WingMap } from '$lib/wing.js';
	import { allWingBindings, controlKey, learned, pageNames, renamePage, wingPage, wingPages } from '$lib/stores/wing.js';
	import { canEdit } from '$lib/stores/showLock.js';

	let {
		map,
		describe
	}: { map: WingMap; describe: (action: BindingAction) => string } = $props();

	/** The tab clicked, or `null` to follow the wing. */
	let picked = $state<number | null>(null);
	const viewing = $derived(picked ?? $wingPage);

	const layout = $derived(playbackLayout(map));

	const onPage = $derived.by(() => {
		const out = new Map<string, ControlBinding>();
		for (const b of $allWingBindings) if ((b.page ?? 1) === viewing) out.set(controlKey(b.control), b);
		return out;
	});

	const what = (control: SurfaceControl): string | null => {
		const b = onPage.get(controlKey(control));
		return b ? b.label || describe(b.action) : null;
	};

	/** A control is learned onto the page the wing is on, so only that page edits. */
	const editable = $derived($canEdit && viewing === $wingPage);
	const edit = (control: SurfaceControl) => {
		if (editable) learned.set(control);
	};

	let name = $state('');
	$effect(() => {
		name = $pageNames.get(viewing) ?? '';
	});
	const saveName = () => {
		if (name.trim() !== ($pageNames.get(viewing) ?? '')) void renamePage(viewing, name);
	};
</script>

<section class="pages">
	<div class="tabs" role="tablist" aria-label="Pages">
		{#each $wingPages as page (page)}
			<button
				role="tab"
				aria-selected={page === viewing}
				class:on={page === viewing}
				class:wing={page === $wingPage}
				title={page === $wingPage ? 'The page the wing is on' : 'Look at this page'}
				onclick={() => (picked = page === $wingPage ? null : page)}
			>
				<span class="n">{page}</span>
				{$pageNames.get(page) ?? ''}
			</button>
		{/each}
	</div>

	<div class="head">
		{#if $canEdit}
			<input
				class="text name"
				placeholder="Page {viewing}"
				aria-label="Name of page {viewing}"
				bind:value={name}
				onblur={saveName}
				onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
			/>
		{:else}
			<strong>{$pageNames.get(viewing) ?? `Page ${viewing}`}</strong>
		{/if}
		<span class="dim">
			{viewing === $wingPage ? 'The wing is on this page' : `The wing is on page ${$wingPage}; PAGE+ and PAGE− turn it`}
		</span>
	</div>

	<div class="desk">
		{#each layout.columns as column, i (column.fader)}
			{@const fader = { kind: 'Fader', index: column.fader } as SurfaceControl}
			<div class="column">
				{#each [column.off, null, column.go] as key, row (row)}
					{#if row === 1}
						{@const does = what(fader)}
						<button class="fader" class:bound={does} disabled={!editable} onclick={() => edit(fader)} title={does ?? 'Nothing on this page'}>
							<span class="slot"></span>
							<span class="label">{does ?? ''}</span>
						</button>
					{:else if key !== null}
						{@const control = { kind: 'Key', index: key } as SurfaceControl}
						{@const does = what(control)}
						<button class="key" class:bound={does} disabled={!editable} onclick={() => edit(control)} title={does ?? (row === 0 ? 'Off of the fader, when it is a master' : 'Go of the fader, when it is a master')}>
							{does ?? (row === 0 ? 'off' : 'go')}
						</button>
					{:else}
						<span class="key none"></span>
					{/if}
				{/each}
				<span class="num">{i + 1}</span>
			</div>
		{/each}
	</div>

	{#if layout.xKeys.length > 0}
		<div class="xkeys">
			{#each layout.xKeys as x (x.index)}
				{@const control = { kind: 'Key', index: x.index } as SurfaceControl}
				{@const does = what(control)}
				<button class="key" class:bound={does} disabled={!editable} onclick={() => edit(control)} title={does ?? `X${x.n}: nothing on this page`}>
					<span class="xn">X{x.n}</span>
					{does ?? ''}
				</button>
			{/each}
		</div>
	{/if}
</section>

<style>
	.pages {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.tabs {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.tabs button {
		background: none;
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: 6px;
		color: var(--text, #ddd);
		font: inherit;
		font-size: var(--font-sm, 12px);
		padding: 3px 8px;
		cursor: pointer;
		display: inline-flex;
		gap: 6px;
		align-items: center;
	}
	.tabs button.on {
		background: var(--bg-raised, #2a2a2a);
		color: var(--text-bright, #fff);
		border-color: var(--accent, #4a8cff);
	}
	.tabs .n {
		font-weight: 600;
	}
	/* The page the wing is on, whichever is being looked at. */
	.tabs button.wing .n {
		color: var(--accent, #4a8cff);
	}
	.head {
		display: flex;
		gap: 10px;
		align-items: center;
	}
	.name {
		width: 14em;
	}
	.desk {
		display: grid;
		grid-template-columns: repeat(10, minmax(0, 1fr));
		gap: 6px;
	}
	.column {
		display: flex;
		flex-direction: column;
		gap: 4px;
		align-items: stretch;
		min-width: 0;
	}
	.key,
	.fader {
		background: var(--bg-sunken, #161616);
		border: 1px solid var(--line, #2c2c2c);
		border-radius: 4px;
		color: var(--text-faint, #666);
		font: inherit;
		font-size: 10px;
		line-height: 1.2;
		padding: 3px;
		min-height: 30px;
		overflow: hidden;
		text-overflow: ellipsis;
		overflow-wrap: anywhere;
		cursor: default;
	}
	.key.none {
		visibility: hidden;
	}
	.fader {
		min-height: 110px;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
	}
	.fader .slot {
		width: 4px;
		flex: 1;
		border-radius: 2px;
		background: var(--line-strong, #3a3a3a);
	}
	.fader .label {
		max-height: 3.6em;
		overflow: hidden;
	}
	.bound {
		color: var(--text-bright, #fff);
		border-color: var(--cat-playback, #4a8cff);
	}
	.fader.bound .slot {
		background: var(--cat-playback, #4a8cff);
	}
	button:not(:disabled) {
		cursor: pointer;
	}
	button:not(:disabled):hover {
		background: var(--bg-hover, #2a2a2a);
	}
	.num {
		text-align: center;
		font-size: 10px;
		color: var(--text-faint, #666);
	}
	.xkeys {
		display: grid;
		grid-template-columns: repeat(8, minmax(0, 1fr));
		gap: 6px;
	}
	.xn {
		display: block;
		font-weight: 600;
		color: var(--text-faint, #666);
	}
</style>
