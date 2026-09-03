<script lang="ts">
	/**
	 * The console, or the offer of one.
	 *
	 * A station with no show open is a real state and the one this comes up in when
	 * nobody named a show: everything runs, and there is simply nothing to look at
	 * yet. The top bar stays either way, and the connecting cover sits over both.
	 */

	import Workspace from '$lib/components/layout/Workspace.svelte';
	import Welcome from '$lib/components/welcome/Welcome.svelte';
	import { getStationContext } from '$lib/ws/context.js';

	const station = getStationContext();
</script>

{#if $station && !$station.show}
	<Welcome
		version={$station.version}
		nodeId={$station.nodeId}
		repository={$station.repository ?? ''}
	/>
{:else}
	<Workspace />
{/if}
