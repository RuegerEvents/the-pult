<script lang="ts">
	import ShowPanel from '$lib/components/ShowPanel.svelte';
	import SequenceRunner from '$lib/components/SequenceRunner.svelte';
	import SessionPanel from '$lib/components/SessionPanel.svelte';
	import DevicesPanel from '$lib/components/DevicesPanel.svelte';
	import PatchPanel from '$lib/components/PatchPanel.svelte';
	import FlowEditor from '$lib/components/flow/FlowEditor.svelte';
	import OutputsPanel from '$lib/components/OutputsPanel.svelte';
	import StationsPanel from '$lib/components/StationsPanel.svelte';

	type View = 'playback' | 'patch' | 'flows' | 'outputs' | 'stations';

	/// Views that own their own scrolling. The flow canvas sizes itself to what is
	/// left of the window, so the main area must not grow a scrollbar around it.
	const fills = (v: View) => v === 'flows';
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
			<button class:active={view === 'flows'} onclick={() => (view = 'flows')}>Flows</button>
			<button class:active={view === 'outputs'} onclick={() => (view = 'outputs')}>Outputs</button>
			<button class:active={view === 'stations'} onclick={() => (view = 'stations')}>Stations</button>
		</nav>
		<div class="view" class:fills={fills(view)}>
			{#if view === 'playback'}
				<SequenceRunner />
			{:else if view === 'patch'}
				<PatchPanel />
			{:else if view === 'flows'}
				<FlowEditor />
			{:else if view === 'outputs'}
				<OutputsPanel />
			{:else}
				<StationsPanel />
			{/if}
		</div>
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
		display: flex;
		flex-direction: column;
		min-height: 0;
	}

	.view {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}

	.view.fills {
		overflow: hidden;
	}

	.tabs {
		display: flex;
		gap: 2px;
		padding: 10px 20px 0;
		border-bottom: 1px solid #2a2a2a;
		flex: none;
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
