<script lang="ts">
	/**
	 * A rig in, and a rig out.
	 *
	 * Both stage toolbars carry these, because both are places somebody is looking at
	 * the drawing and might want to hand it to somebody else. An import is a POST of
	 * the whole archive — too large for the socket, which is why it is a route — and
	 * what comes back is the report, shown until it is dismissed rather than as a
	 * toast that goes before it has been read: an import that warned about eleven
	 * fixtures is something to work through, not something to glance at.
	 */
	import { getClientContext } from '$lib/ws/context.js';
	import { layers, hiddenLayers } from '$lib/stores/scene.js';

	const client = getClientContext();

	type Report = {
		created: number;
		updated: number;
		missing: string[];
		warnings: string[];
	};

	let importing = $state(false);
	let report = $state<Report | null>(null);
	let failed = $state<string | null>(null);
	let fileInput = $state<HTMLInputElement | null>(null);

	async function importMvr(file: File) {
		importing = true;
		report = null;
		failed = null;
		try {
			const response = await fetch(client.httpUrl('/api/import/mvr'), {
				method: 'POST',
				headers: { 'content-type': 'application/vnd.mvr-scene+zip' },
				body: file
			});
			if (!response.ok) {
				failed = (await response.text()) || `the console answered ${response.status}`;
				return;
			}
			report = await response.json();
		} catch (error) {
			failed = String(error);
		} finally {
			importing = false;
		}
	}

	/// Only the layers this browser is showing, which is the one place hiding a layer
	/// means more than "do not draw it": what is on screen is what gets handed over.
	const shownLayers = $derived($layers.filter((layer) => !$hiddenLayers.has(layer.id)));
	const exportUrl = $derived(
		$hiddenLayers.size === 0
			? client.httpUrl('/api/export/mvr')
			: client.httpUrl(`/api/export/mvr?layers=${shownLayers.map((l) => l.id).join(',')}`)
	);
</script>

<input
	type="file"
	accept=".mvr,application/vnd.mvr-scene+zip"
	bind:this={fileInput}
	onchange={(e) => {
		const file = e.currentTarget.files?.[0];
		if (file) void importMvr(file);
		e.currentTarget.value = '';
	}}
	hidden
/>

<button class="ghost" onclick={() => fileInput?.click()} disabled={importing}>
	{importing ? 'Importing…' : 'Import MVR'}
</button>
<a class="ghost" href={exportUrl} download="rig.mvr">
	Export{$hiddenLayers.size > 0 ? ' shown' : ''}
</a>

{#if failed}
	<span class="failed" role="alert">
		{failed}
		<button class="dismiss" onclick={() => (failed = null)} aria-label="Dismiss">×</button>
	</span>
{:else if report}
	<span class="report">
		{report.created} new, {report.updated} updated{report.missing.length > 0
			? `, ${report.missing.length} gone`
			: ''}{report.warnings.length > 0 ? `, ${report.warnings.length} warnings` : ''}
		{#if report.warnings.length > 0 || report.missing.length > 0}
			<details>
				<summary>Details</summary>
				<ul>
					{#each report.missing as gone (gone)}
						<li class="gone">no longer in the drawing: {gone}</li>
					{/each}
					{#each report.warnings as warning (warning)}
						<li>{warning}</li>
					{/each}
				</ul>
			</details>
		{/if}
		<button class="dismiss" onclick={() => (report = null)} aria-label="Dismiss">×</button>
	</span>
{/if}

<style>
	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		color: var(--text);
		padding: 4px 10px;
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
		text-decoration: none;
	}
	.ghost:hover { border-color: var(--line-input); }
	.ghost[disabled] { opacity: 0.5; cursor: default; }

	.report, .failed {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: var(--font-xs);
		color: var(--text-dim);
		position: relative;
	}
	.failed { color: var(--danger, #d66); }

	details { position: relative; }
	summary { cursor: pointer; }
	/* Over the view rather than in the toolbar: a rig with eleven warnings would
	   otherwise push every other control off the bar. */
	ul {
		position: absolute;
		z-index: 10;
		top: 1.4em;
		right: 0;
		width: 34rem;
		max-height: 20rem;
		overflow: auto;
		margin: 0;
		padding: 8px 8px 8px 24px;
		background: var(--panel, #1a1a1a);
		border: 1px solid var(--line-strong);
		border-radius: 3px;
	}
	li { margin: 2px 0; }
	.gone { color: var(--text); }

	.dismiss {
		background: none;
		border: none;
		color: inherit;
		cursor: pointer;
		font-size: 14px;
		line-height: 1;
		padding: 0 2px;
	}
</style>
