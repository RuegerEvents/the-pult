<script lang="ts">
	/**
	 * A layout itself, in the Inspector: how big its canvas is, what a dragged element
	 * snaps to, the drawing behind it, and how a reading on it decays when an element
	 * says nothing of its own.
	 *
	 * Each field is one write of one column, so each change is one Ctrl-Z. Under the
	 * Show lock it shows and changes nothing.
	 */

	import type { Decay, Layout } from '$lib/generated/index.js';
	import { DEFAULT_ACTIVE_COLOR, DEFAULT_DECAY_MS, DEFAULT_IDLE_COLOR } from '$lib/layoutView.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { addToast } from '$lib/toasts.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { uploadPlan } from '$lib/components/stage/upload.js';

	let { rows }: { rows: Layout[] } = $props();

	const data = getDataContext();
	const client = getClientContext();

	let uploading = $state(false);

	const proxy = (layout: Layout) => data.layouts.byId(layout.id);
	/** `backdrop` is optional on the row, so the proxy types it so; the path is always there. */
	const setBackdrop = (layout: Layout, sha: string | null) => proxy(layout).backdrop!.set(sha);

	/** A positive number out of a field, or nothing when it is not one. */
	const positive = (value: number) => (Number.isFinite(value) && value > 0 ? value : null);

	async function setDecay(layout: Layout, change: Partial<Decay>) {
		await proxy(layout).decay.set({ ...layout.decay, ...change });
	}

	async function upload(layout: Layout, file: File | undefined) {
		if (!file) return;
		uploading = true;
		try {
			// A PDF is rasterized on the way in, exactly as a stage plan is.
			const { sha256 } = await uploadPlan(file, client.httpUrl('/assets'));
			await setBackdrop(layout, sha256);
		} catch (e) {
			addToast(e instanceof Error ? e.message : String(e));
		} finally {
			uploading = false;
		}
	}
</script>

{#each rows as layout (layout.id)}
	{@const decay = layout.decay}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{layout.name}</span>{/if}

		<span class="label">Canvas</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				min="1"
				disabled={!$canEdit}
				value={layout.width}
				aria-label="Width"
				onchange={(e) => {
					const v = positive(e.currentTarget.valueAsNumber);
					if (v) void proxy(layout).width.set(v);
				}}
			/>
			×
			<input
				class="input narrow"
				type="number"
				min="1"
				disabled={!$canEdit}
				value={layout.height}
				aria-label="Height"
				onchange={(e) => {
					const v = positive(e.currentTarget.valueAsNumber);
					if (v) void proxy(layout).height.set(v);
				}}
			/>
			<span class="hint">units</span>
		</div>

		<span class="label">Grid</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				min="0"
				disabled={!$canEdit}
				value={layout.grid}
				aria-label="Grid"
				onchange={(e) => {
					const v = e.currentTarget.valueAsNumber;
					if (Number.isFinite(v) && v >= 0) void proxy(layout).grid.set(v);
				}}
			/>
			<span class="hint">{layout.grid > 0 ? 'a dragged element snaps to it' : 'no snapping'}</span>
		</div>

		<span class="label">Drawing</span>
		<div class="inline">
			{#if layout.backdrop}
				<span class="hint">a picture behind it</span>
				{#if $canEdit}
					<button class="btn btn-ghost" onclick={() => setBackdrop(layout, null)}>Remove</button>
				{/if}
			{:else}
				<span class="hint">none</span>
			{/if}
			{#if $canEdit}
				<label class="btn btn-ghost file" class:busy={uploading}>
					{uploading ? 'Uploading…' : layout.backdrop ? 'Replace…' : 'Add…'}
					<input
						type="file"
						accept="image/png,image/jpeg,image/webp,application/pdf"
						disabled={uploading}
						onchange={(e) => {
							void upload(layout, e.currentTarget.files?.[0]);
							e.currentTarget.value = '';
						}}
					/>
				</label>
			{/if}
		</div>

		<span class="wide section">How a reading decays — what every element says unless it says otherwise</span>

		<span class="label">On</span>
		<div class="inline">
			<input
				type="color"
				disabled={!$canEdit}
				value={decay.active_color || DEFAULT_ACTIVE_COLOR}
				aria-label="Active color"
				onchange={(e) => setDecay(layout, { active_color: e.currentTarget.value })}
			/>
			{#if decay.active_color && $canEdit}
				<button class="btn btn-ghost" onclick={() => setDecay(layout, { active_color: null })}>default</button>
			{/if}
		</div>

		<span class="label">Idle</span>
		<div class="inline">
			<input
				type="color"
				disabled={!$canEdit}
				value={decay.idle_color || DEFAULT_IDLE_COLOR}
				aria-label="Idle color"
				onchange={(e) => setDecay(layout, { idle_color: e.currentTarget.value })}
			/>
			{#if decay.idle_color && $canEdit}
				<button class="btn btn-ghost" onclick={() => setDecay(layout, { idle_color: null })}>default</button>
			{/if}
		</div>

		<span class="label">Fades</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				min="0"
				step="1"
				disabled={!$canEdit}
				value={(decay.decay_ms ?? DEFAULT_DECAY_MS) / 1000}
				aria-label="Decay time in seconds"
				onchange={(e) => {
					const s = e.currentTarget.valueAsNumber;
					if (Number.isFinite(s) && s >= 0) void setDecay(layout, { decay_ms: Math.round(s * 1000) });
				}}
			/>
			<span class="hint">seconds after it goes off</span>
		</div>

		<span class="wide hint">{layout.elements.length} element{layout.elements.length === 1 ? '' : 's'}.</span>
	</div>
{/each}

<style>
	.fields {
		display: grid;
		grid-template-columns: 64px minmax(0, 1fr);
		align-items: center;
		gap: 8px;
		font-size: 12px;
		color: #999;
	}
	.wide {
		grid-column: 1 / -1;
	}
	.section {
		margin-top: 6px;
		color: #bbb;
	}
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}
	.narrow {
		width: 72px;
	}
	.hint {
		color: #777;
	}
	.file {
		position: relative;
		overflow: hidden;
	}
	.file input {
		position: absolute;
		inset: 0;
		opacity: 0;
		cursor: pointer;
	}
	.busy {
		opacity: 0.6;
	}
</style>
