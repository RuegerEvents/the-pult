<script lang="ts">
	import ShowPanel from '$lib/components/ShowPanel.svelte';
	import SequenceRunner from '$lib/components/SequenceRunner.svelte';
	import SessionPanel from '$lib/components/SessionPanel.svelte';
</script>

<div class="layout">
	<aside class="sidebar">
		<ShowPanel />
		<SessionPanel />
	</aside>
	<main class="main">
		<SequenceRunner />
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
</style>
