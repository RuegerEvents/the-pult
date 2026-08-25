<script lang="ts">
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type { Show } from '$lib/generated/index.js';

	const data = getDataContext();

	let show = $state<Show | null>(null);
	let editingName = $state(false);
	let draftName = $state('');
	let saving = $state(false);

	async function initShow() {
		await data.show.set({
			id: crypto.randomUUID(),
			name: 'My Show',
			created_at: new Date().toISOString(),
			is_running: false,
			active_sequence: null
		});
	}

	async function saveName() {
		if (!show || !draftName.trim()) return;
		saving = true;
		await data.show.set({ ...show, name: draftName.trim() });
		editingName = false;
		saving = false;
	}

	async function toggleRunning() {
		if (!show) return;
		await data.show.set({ ...show, is_running: !show.is_running });
	}

	onMount(() => {
		// subscribe auto-fetches the current value and re-fetches on reconnect
		return data.show.subscribe(v => { show = v as Show | null; });
	});
</script>

<div class="panel">
	<div class="panel-header">
		<span class="panel-title">Show</span>
		{#if show}
			<button
				class="run-btn"
				class:running={show.is_running}
				onclick={toggleRunning}
				title={show.is_running ? 'Stop show' : 'Run show'}
			>
				{show.is_running ? '■ Running' : '▶ Stopped'}
			</button>
		{/if}
	</div>

	{#if !show}
		<p class="empty-hint">No show file loaded.</p>
		<button class="action-btn" onclick={initShow}>Initialize Show</button>
	{:else}
		<div class="field">
			<span class="label">Name</span>
			{#if editingName}
				<form
					class="inline-form"
					onsubmit={(e) => {
						e.preventDefault();
						saveName();
					}}
				>
					<!-- svelte-ignore a11y_autofocus -->
					<input
						class="inline-input"
						bind:value={draftName}
						disabled={saving}
						autofocus
						onkeydown={(e) => {
							if (e.key === 'Escape') editingName = false;
						}}
					/>
					<button class="save-btn" type="submit" disabled={saving}>✓</button>
					<button class="cancel-btn" type="button" onclick={() => (editingName = false)}>✕</button>
				</form>
			{:else}
				<button
					class="name-value"
					onclick={() => {
						draftName = show?.name ?? '';
						editingName = true;
					}}
					title="Click to edit"
				>
					{show.name}
					<span class="edit-hint">✎</span>
				</button>
			{/if}
		</div>
		<div class="field">
			<span class="label">Show ID</span>
			<span class="mono dim">{show.id.slice(0, 8)}…</span>
		</div>
	{/if}
</div>

<style>
	.panel {
		background: #252525;
		border: 1px solid #333;
		border-radius: 6px;
		padding: 12px 14px;
	}

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 10px;
	}

	.panel-title {
		font-size: 0.68rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #777;
	}

	.run-btn {
		font-size: 0.7rem;
		padding: 2px 8px;
		border-radius: 3px;
		border: 1px solid #555;
		background: transparent;
		color: #aaa;
		cursor: pointer;
		transition: all 0.15s;
	}
	.run-btn.running {
		border-color: #22c55e;
		color: #22c55e;
	}
	.run-btn:hover {
		background: #333;
	}

	.field {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 5px 0;
		border-bottom: 1px solid #2e2e2e;
	}
	.field:last-child {
		border-bottom: none;
	}

	.label {
		font-size: 0.78rem;
		color: #888;
	}

	.name-value {
		background: none;
		border: none;
		color: #e0e0e0;
		font-size: 0.85rem;
		cursor: pointer;
		padding: 0;
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.name-value:hover .edit-hint {
		opacity: 1;
	}
	.edit-hint {
		opacity: 0;
		font-size: 0.7rem;
		color: #666;
		transition: opacity 0.15s;
	}

	.inline-form {
		display: flex;
		gap: 4px;
		align-items: center;
	}
	.inline-input {
		background: #1a1a1a;
		border: 1px solid #555;
		border-radius: 3px;
		color: #e0e0e0;
		font-size: 0.85rem;
		padding: 2px 6px;
		width: 140px;
	}
	.save-btn,
	.cancel-btn {
		background: none;
		border: 1px solid #444;
		border-radius: 3px;
		color: #aaa;
		cursor: pointer;
		font-size: 0.75rem;
		padding: 1px 5px;
	}
	.save-btn:hover {
		border-color: #22c55e;
		color: #22c55e;
	}
	.cancel-btn:hover {
		border-color: #ef4444;
		color: #ef4444;
	}

	.mono {
		font-family: monospace;
		font-size: 0.78rem;
		color: #888;
	}
	.dim {
		color: #666;
	}

	.empty-hint {
		font-size: 0.8rem;
		color: #555;
		font-style: italic;
		margin-bottom: 10px;
	}

	.action-btn {
		font-size: 0.78rem;
		padding: 4px 10px;
		border-radius: 4px;
		border: 1px solid #4a9eff;
		background: transparent;
		color: #4a9eff;
		cursor: pointer;
	}
	.action-btn:hover {
		background: #4a9eff22;
	}
</style>
