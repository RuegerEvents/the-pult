<script lang="ts">
	/**
	 * The board's own controls, handed out to the editor around it.
	 *
	 * `useSvelteFlow` answers only inside `<SvelteFlow>`, and the editor that wants to
	 * place a node at the middle of the view or put a flow's view back is outside it.
	 * So this sits inside, draws nothing, and gives the editor the handle once.
	 */
	import { onMount } from 'svelte';
	import { useSvelteFlow } from '@xyflow/svelte';

	let { onready }: { onready: (api: ReturnType<typeof useSvelteFlow>) => void } = $props();

	const api = useSvelteFlow();
	onMount(() => onready(api));
</script>
