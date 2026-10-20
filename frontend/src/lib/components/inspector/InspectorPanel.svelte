<script lang="ts">
	/**
	 * The Inspector: an editor for whatever is selected, whichever panel it lives in.
	 *
	 * It knows no kind of object. Each kind registers itself (`$lib/kinds/`) with the
	 * component that inspects it, and this panel draws what every kind shares — the
	 * name, edited in place, and the color — above whatever that component adds. So a
	 * preset's values and a group's question appear here because their kinds said so,
	 * and a new kind needs nothing changed in this file.
	 *
	 * Several of one kind at once: the shared fields edit all of them (a color painted
	 * onto five presets), and the kind's own component gets every row. Several kinds:
	 * one section per kind, each as above.
	 *
	 * Under the Show lock it still shows everything and changes nothing.
	 */

	import { categoryColor } from '$lib/layout/categories.js';
	import { clearSubject, SWATCHES, subjectRows, type ObjectKind, type ObjectRow } from '$lib/objects.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { canEdit } from '$lib/stores/showLock.js';

	async function paint(kind: ObjectKind, rows: ObjectRow[], color: string | null) {
		if (!kind.recolor) return;
		// One color onto five presets is one act, and one Ctrl-Z.
		beginGesture();
		try {
			for (const row of rows) await kind.recolor(row, color);
		} finally {
			endGesture();
		}
	}

	async function rename(kind: ObjectKind, row: ObjectRow, value: string) {
		const name = value.trim();
		if (name && name !== row.name && kind.rename) await kind.rename(row, name);
	}

	const sharedColor = (rows: ObjectRow[]): string | null | undefined => {
		const first = rows[0]?.color ?? null;
		return rows.every((r) => (r.color ?? null) === first) ? first : undefined;
	};
</script>

<div class="inspector">
	{#if $subjectRows.length === 0}
		<p class="empty">
			Nothing to inspect. ⌘/Ctrl-click a preset, a group, an effect or a surface — or tap
			the dot in its corner — and it opens here.
		</p>
	{:else}
		{#each $subjectRows as { kind, rows } (kind.kind)}
			{@const Icon = kind.icon}
			{@const Detail = kind.inspector}
			{@const color = sharedColor(rows)}
			<section style:--cat={categoryColor(kind.category)}>
				<header>
					<span class="kind"><Icon size={14} />{rows.length === 1 ? kind.label : `${rows.length} × ${kind.label}`}</span>
					<span class="spacer"></span>
					<button class="btn btn-ghost" onclick={clearSubject}>Done</button>
				</header>

				{#if rows.length === 1}
					{@const row = rows[0]}
					{#if $canEdit && kind.rename}
						<input
							class="input name"
							aria-label="Name"
							value={row.name}
							onchange={(e) => rename(kind, row, e.currentTarget.value)}
							onkeydown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
						/>
					{:else}
						<h3 class="name">{row.name}</h3>
					{/if}
				{:else}
					<p class="names">{rows.map((r) => r.name).join(', ')}</p>
				{/if}

				{#if $canEdit && kind.recolor}
					<div class="swatches" role="group" aria-label="Color">
						{#each SWATCHES as swatch (swatch ?? 'none')}
							<button
								class="swatch"
								class:none={swatch === null}
								class:on={color === swatch}
								style:background={swatch ?? 'transparent'}
								aria-label={swatch ?? 'Its category color'}
								title={swatch ?? 'Its category color'}
								onclick={() => paint(kind, rows, swatch)}
							></button>
						{/each}
						{#if color === undefined}<span class="mixed">mixed</span>{/if}
					</div>
				{/if}

				{#if Detail}
					<Detail {rows} />
				{/if}
			</section>
		{/each}
	{/if}
</div>

<style>
	.inspector {
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: var(--pad);
	}
	section {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding-left: 10px;
		border-left: 3px solid var(--cat);
	}
	header {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.kind {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--cat);
		font-weight: 600;
	}
	.spacer {
		flex: 1;
	}
	.name {
		font-size: var(--font-md);
		font-weight: 600;
	}
	input.name {
		width: 100%;
	}
	.names {
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.swatches {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
	}
	.swatch {
		width: 22px;
		height: 22px;
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
	.mixed {
		font-size: var(--font-xs);
		color: var(--text-faint);
	}
	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		max-width: 46ch;
	}
</style>
