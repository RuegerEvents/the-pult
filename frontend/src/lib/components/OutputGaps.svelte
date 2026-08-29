<script lang="ts">
	/**
	 * Fixtures that no output reaches, each with the one button that fixes it.
	 *
	 * The backend works out the gaps from show data (`output_coverage`, LOCAL);
	 * this only says them out loud and offers to add the output they name — an
	 * sACN output for the universe, or the OpenHaunt output that drives nodes.
	 * Shown wherever an operator would otherwise stare at a fader doing nothing.
	 */
	import { onMount } from 'svelte';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import type { OutputCoverage, OutputGap, OutputKind } from '$lib/generated/index.js';

	let { only = null }: { only?: OutputKind | null } = $props();

	const client = getClientContext();
	const data = getDataContext();

	let gaps = $state<OutputGap[]>([]);
	let thisStation = $state<string | null>(null);
	let busy = $state(false);

	const shown = $derived(only ? gaps.filter((g) => g.kind === only) : gaps);

	function describe(gap: OutputGap): string {
		const names = gap.fixture_names.slice(0, 3).join(', ');
		const more = gap.fixture_names.length > 3 ? ` and ${gap.fixture_names.length - 3} more` : '';
		if (gap.universe !== null)
			return `${names}${more} ${gap.fixture_names.length === 1 ? 'is' : 'are'} patched to universe ${gap.universe}, which no output carries.`;
		return `${names}${more} ${gap.fixture_names.length === 1 ? 'is' : 'are'} adopted, but nothing drives OpenHaunt nodes.`;
	}

	function fix(gap: OutputGap): string {
		return gap.universe !== null ? `Add sACN output for universe ${gap.universe}` : 'Add OpenHaunt output';
	}

	async function close(gap: OutputGap) {
		busy = true;
		try {
			await data.outputs.create({
				id: crypto.randomUUID(),
				name: gap.universe !== null ? `Universe ${gap.universe}` : 'OpenHaunt nodes',
				kind: gap.kind,
				target: null,
				universes: gap.universe !== null ? [gap.universe] : [],
				enabled: true,
				node_id: thisStation
			});
		} catch (e) {
			addToast(`Could not add the output: ${e}`);
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		const applyCoverage = (v: unknown) => {
			if (v && typeof v === 'object') gaps = (v as OutputCoverage).gaps ?? [];
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const stopCoverage = client.subscribe('output_coverage', applyCoverage);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['output_coverage']).then(applyCoverage);
			client.get(['session']).then(applySession);
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);
		return () => { stopCoverage(); stopSession(); stopConnect(); };
	});
</script>

{#if shown.length > 0}
	<div class="gaps">
		{#each shown as gap (gap.kind + (gap.universe ?? ''))}
			<div class="gap">
				<span class="mark">!</span>
				<span class="text">{describe(gap)}</span>
				<button class="fix" disabled={busy} onclick={() => close(gap)}>{fix(gap)}</button>
			</div>
		{/each}
	</div>
{/if}

<style>
	.gaps { display: flex; flex-direction: column; gap: 6px; margin: 0 0 12px; }
	.gap { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border: 1px solid #a3691f66; border-radius: 4px; background: #a3691f14; font-size: 13px; }
	.mark { flex-shrink: 0; width: 18px; height: 18px; border-radius: 50%; background: #d9932a; color: #000; font-weight: 700; font-size: 12px; display: flex; align-items: center; justify-content: center; }
	.text { flex: 1; color: #ddd; }
	.fix { background: none; border: 1px solid #d9932a88; border-radius: 3px; color: #f0b556; padding: 4px 10px; font: inherit; cursor: pointer; white-space: nowrap; }
	.fix:hover:not(:disabled) { background: #d9932a22; border-color: #d9932a; }
	.fix:disabled { opacity: 0.5; cursor: default; }
</style>
