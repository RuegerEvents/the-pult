<script lang="ts">
	/**
	 * Where a node and the show call something differently and the station could not
	 * tell which is right — the node was renamed on its own page, reset, or is another
	 * module now. Nothing was written either way; this is where somebody picks.
	 */
	import Dialog from './Dialog.svelte';
	import { getClientContext } from '$lib/ws/context.js';
	import { addToast } from '$lib/toasts.js';
	import type { DiscoveredDevice, NameConflict, NameSide } from '$lib/generated/index.js';

	let { device, onclose }: { device: DiscoveredDevice; onclose: () => void } = $props();

	const client = getClientContext();
	const id = (c: NameConflict) => `${c.head}:${c.port ?? '-'}`;

	/** Per conflict, which side; the show's until somebody says otherwise. */
	let picked = $state<Record<string, NameSide>>({});
	let busy = $state(false);
	const sideOf = (c: NameConflict): NameSide => picked[id(c)] ?? 'show';

	function all(side: NameSide) {
		picked = Object.fromEntries(device.name_conflicts.map((c) => [id(c), side]));
	}

	async function settle() {
		busy = true;
		try {
			await client.call('device.names', {
				serial: device.serial,
				choices: device.name_conflicts.map((c) => ({ head: c.head, port: c.port, take: sideOf(c) }))
			});
			onclose();
		} catch (e) {
			addToast(`${device.name}: ${e}`);
		} finally {
			busy = false;
		}
	}

	const shown = (name: string) => name || '(no name)';
</script>

<Dialog title="{device.name}: names differ" size="wide" {onclose} onaccept={settle}>
	<div class="conflicts">
		<p class="note">
			The node and the show call these differently, and it is not clear which one changed. Pick which to keep;
			the other side is renamed to match.
		</p>
		<table>
			<thead>
				<tr>
					<th></th>
					<th><button class="link" onclick={() => all('node')}>On the node</button></th>
					<th><button class="link" onclick={() => all('show')}>In the show</button></th>
				</tr>
			</thead>
			<tbody>
				{#each device.name_conflicts as c (id(c))}
					<tr>
						<td class="what">
							{#if c.port !== null}<span class="mono dim">{c.port}</span>{/if}
							{c.what}
						</td>
						{#each ['node', 'show'] as side (side)}
							{@const value = side === 'node' ? c.node : c.show}
							<td>
								<label class="pick" class:on={sideOf(c) === side} class:empty={!value}>
									<input
										type="radio"
										name={id(c)}
										checked={sideOf(c) === side}
										onchange={() => (picked = { ...picked, [id(c)]: side as NameSide })}
									/>
									{shown(value)}
								</label>
							</td>
						{/each}
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>Later</button>
		<button class="btn btn-primary" disabled={busy || !device.online} onclick={settle}>Apply</button>
	{/snippet}
</Dialog>

<style>
	.conflicts {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.note {
		margin: 0;
		font-size: 0.75rem;
		color: #999;
		line-height: 1.5;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.78rem;
	}
	th {
		text-align: left;
		font-weight: 500;
		color: #888;
		padding: 2px 6px;
	}
	td {
		padding: 3px 6px;
		border-top: 1px solid #2a2a2a;
	}
	.what {
		color: #aaa;
	}
	.pick {
		display: flex;
		gap: 6px;
		align-items: center;
		cursor: pointer;
	}
	.pick.on {
		color: #4a9eff;
	}
	.pick.empty {
		font-style: italic;
		color: #777;
	}
	.link {
		background: none;
		border: none;
		color: inherit;
		font: inherit;
		cursor: pointer;
		padding: 0;
		text-decoration: underline dotted;
	}
	.dim {
		color: #666;
	}
	.mono {
		font-family: monospace;
	}
</style>
