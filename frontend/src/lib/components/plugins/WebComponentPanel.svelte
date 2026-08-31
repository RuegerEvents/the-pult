<script lang="ts">
	/**
	 * A panel a plugin ships as its own JavaScript.
	 *
	 * The manifest names a script (served by the backend from the plugin's
	 * assets) and the custom element it defines; this component loads the one
	 * and mounts the other. The element gets a `pult` property before it is
	 * attached — a small bridge for calling its plugin and subscribing to
	 * data — so a plugin's panel talks to the console without knowing anything
	 * about the socket underneath.
	 */

	import { onMount } from 'svelte';

	import type { PluginStatus } from '$lib/generated/index.js';
	import { showClient } from '$lib/stores/show.js';

	let {
		pluginId,
		element,
		script,
		status
	}: { pluginId: string; element: string; script: string; status: PluginStatus } = $props();

	let host = $state<HTMLDivElement | null>(null);
	let problem = $state<string | null>(null);

	const failed = $derived(status.state === 'Failed' ? status.reason : null);

	/** Scripts already injected this session, by URL — a module runs once. */
	const loaded: Set<string> = ((globalThis as Record<string, unknown>).__pultPanelScripts ??=
		new Set<string>()) as Set<string>;

	function inject(url: string): Promise<void> {
		if (loaded.has(url)) return Promise.resolve();
		return new Promise((resolve, reject) => {
			const tag = document.createElement('script');
			tag.type = 'module';
			tag.src = url;
			tag.onload = () => {
				loaded.add(url);
				resolve();
			};
			tag.onerror = () => reject(new Error(`could not load ${url}`));
			document.head.appendChild(tag);
		});
	}

	onMount(() => {
		if (failed !== null) return;
		let gone = false;
		const url = `/api/plugins/${pluginId}/assets/${script}`;
		inject(url)
			.then(() => {
				if (gone || !host) return;
				const el = document.createElement(element) as HTMLElement & {
					pult?: {
						call(method: string, args?: unknown): Promise<unknown>;
						get(path: string[]): Promise<unknown>;
						subscribe(pattern: string, cb: (value: unknown) => void): () => void;
					};
				};
				el.pult = {
					call: (method, args = {}) =>
						showClient().call(`plugin.${pluginId}.${method}`, { payload: args }),
					get: (path) => showClient().get(path),
					subscribe: (pattern, cb) => showClient().subscribe(pattern, cb)
				};
				host.appendChild(el);
			})
			.catch((e: Error) => (problem = e.message));
		return () => {
			gone = true;
		};
	});
</script>

{#if failed !== null}
	<p class="dead">This panel's plugin ({pluginId}) is not running: {failed}</p>
{:else if problem}
	<p class="dead">{problem}</p>
{:else}
	<div class="host" bind:this={host}></div>
{/if}

<style>
	.host {
		height: 100%;
		min-height: 0;
	}
	.host :global(> *) {
		display: block;
		height: 100%;
	}
	.dead {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 14px;
	}
</style>
