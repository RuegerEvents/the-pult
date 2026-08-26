<script lang="ts">
	import ShowPanel from '$lib/components/ShowPanel.svelte';
	import SequenceRunner from '$lib/components/SequenceRunner.svelte';
	import SessionPanel from '$lib/components/SessionPanel.svelte';
	import DevicesPanel from '$lib/components/DevicesPanel.svelte';
	import PatchPanel from '$lib/components/PatchPanel.svelte';
	import TriggersPanel from '$lib/components/TriggersPanel.svelte';
	import OutputsPanel from '$lib/components/OutputsPanel.svelte';
	import StationsPanel from '$lib/components/StationsPanel.svelte';

	type View = 'playback' | 'patch' | 'triggers' | 'outputs' | 'stations';
	let view = $state<View>('playback');
</script>

<div class="layout">
	<aside class="sidebar">
		<ShowPanel />
		<SessionPanel />
		<DevicesPanel />
	</aside>
	<main class="main">
		<nav class="tabs">
			<button class:active={view === 'playback'} onclick={() => (view = 'playback')}>Playback</button>
			<button class:active={view === 'patch'} onclick={() => (view = 'patch')}>Patch</button>
			<button class:active={view === 'triggers'} onclick={() => (view = 'triggers')}>Triggers</button>
			<button class:active={view === 'outputs'} onclick={() => (view = 'outputs')}>Outputs</button>
			<button class:active={view === 'stations'} onclick={() => (view = 'stations')}>Stations</button>
		</nav>
		{#if view === 'playback'}
			<SequenceRunner />
		{:else if view === 'patch'}
			<PatchPanel />
		{:else if view === 'triggers'}
			<TriggersPanel />
		{:else if view === 'outputs'}
			<OutputsPanel />
		{:else}
			<StationsPanel />
		{/if}
	</main>
</div>

<style>
	.layout {
		display: grid;
		grid-template-columns: 220px 1fr;
		height: 100%;
		overflow: hidden;
	}

	.sidebar {
		border-right: 1px solid #2a2a2a;
		padding: 14px;
		display: flex;
		flex-direction: column;
		gap: 12px;
		overflow-y: auto;
	}

	.main {
		overflow-y: auto;
	}

	.tabs {
		display: flex;
		gap: 2px;
		padding: 10px 20px 0;
		border-bottom: 1px solid #2a2a2a;
	}

	.tabs button {
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: #888;
		font: inherit;
		padding: 6px 12px;
		cursor: pointer;
	}

	.tabs button.active {
		color: #fff;
		border-bottom-color: #2f6fd0;
	}

	.tabs button:hover {
		color: #ddd;
	}
</style>
