<script lang="ts">
	import { onMount } from 'svelte';
	import { focusOnMount } from '$lib/actions.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import type { OutputConfig, OutputKind, OutputStatus } from '$lib/generated/index.js';

	const client = getClientContext();
	const data = getDataContext();

	type Statuses = Record<string, OutputStatus>;

	let outputs = $state<OutputConfig[]>([]);
	let statuses = $state<Statuses>({});
	let thisStation = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let newKind = $state<OutputKind>('Artnet');

	const KINDS: { value: OutputKind; label: string; hint: string }[] = [
		{ value: 'Artnet', label: 'Art-Net', hint: 'needs an address' },
		{ value: 'Sacn', label: 'sACN', hint: 'multicast unless you give an address' },
		{ value: 'OpenHaunt', label: 'OpenHaunt nodes', hint: 'adopted devices' }
	];

	const needsTarget = (kind: OutputKind) => kind === 'Artnet';
	const statusOf = (output: OutputConfig): OutputStatus | undefined => statuses[output.id];

	/// Universes as the operator types them: "1, 5, 7", empty for all.
	const universeList = (output: OutputConfig) => output.universes.join(', ');

	function parseUniverses(text: string): number[] {
		return text
			.split(/[,\s]+/)
			.map((s) => Number(s.trim()))
			.filter((n) => Number.isFinite(n) && n > 0);
	}

	/// "sending · 40/s" or the reason it is not.
	function summarise(output: OutputConfig): string {
		if (!output.enabled) return 'off';
		const status = statusOf(output);
		if (!status) {
			return output.node_id && output.node_id !== thisStation
				? 'another station'
				: 'not started';
		}
		if (status.last_error && status.error_count > 0) return `${status.error_count} errors`;
		if (!status.last_send) return 'no frames yet';
		return `${status.frames_per_second.toFixed(0)}/s`;
	}

	function healthy(output: OutputConfig): boolean {
		const status = statusOf(output);
		return !!status && !!status.last_send && status.error_count === 0;
	}

	async function createOutput() {
		const name = newName.trim();
		if (!name) return;
		await data.outputs.create({
			id: crypto.randomUUID(),
			name,
			kind: newKind,
			target: null,
			universes: [],
			enabled: true,
			// This station by default. Leaving it unset makes every station send the
			// same frames, which is a choice rather than a default.
			node_id: thisStation
		});
		newName = '';
		creating = false;
	}

	onMount(() => {
		const stop = data.outputs.subscribeDeep((v) => { outputs = v; });

		// output_status and session are LOCAL, subscribed by path like devices.
		const applyStatus = (v: unknown) => {
			if (v && typeof v === 'object') statuses = v as Statuses;
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const stopStatus = client.subscribe('output_status', applyStatus);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['output_status']).then(applyStatus);
			client.get(['session']).then(applySession);
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);

		return () => { stop(); stopStatus(); stopSession(); stopConnect(); };
	});
</script>

<div class="outputs">
	<section class="block">
		<header class="block-head">
			<h2>Outputs</h2>
			<button class="ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Output'}
			</button>
		</header>

		{#if creating}
			<form class="new-row" onsubmit={(e) => { e.preventDefault(); createOutput(); }}>
				<input class="text-input" placeholder="What is it feeding?" bind:value={newName} use:focusOnMount />
				<select class="text-input" bind:value={newKind}>
					{#each KINDS as kind (kind.value)}
						<option value={kind.value}>{kind.label}</option>
					{/each}
				</select>
				<button class="primary" type="submit">Add</button>
			</form>
			<p class="hint">{KINDS.find((k) => k.value === newKind)?.hint}</p>
		{/if}

		{#if outputs.length === 0}
			<p class="empty">
				Nothing is being sent anywhere. Add an output to put the show on a wire.
			</p>
		{:else}
			<table class="wires">
				<thead>
					<tr>
						<th>Name</th><th>Protocol</th><th>Address</th><th>Universes</th>
						<th>Station</th><th>On</th><th>Status</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each outputs as output (output.id)}
						{@const status = statusOf(output)}
						<tr class:off={!output.enabled}>
							<td>
								<input
									class="text-input"
									value={output.name}
									onchange={(e) => data.outputs.byId(output.id).name.set(e.currentTarget.value)}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={output.kind}
									onchange={(e) =>
										data.outputs.byId(output.id).kind.set(e.currentTarget.value as OutputKind)}
								>
									{#each KINDS as kind (kind.value)}
										<option value={kind.value}>{kind.label}</option>
									{/each}
								</select>
							</td>
							<td>
								{#if output.kind === 'OpenHaunt'}
									<span class="hint">adopted devices</span>
								{:else}
									<input
										class="text-input"
										placeholder={needsTarget(output.kind) ? '10.0.0.5' : 'multicast'}
										value={output.target ?? ''}
										onchange={(e) =>
											data.outputs
												.byId(output.id)
												.target.set(e.currentTarget.value.trim() || null)}
									/>
								{/if}
							</td>
							<td>
								<input
									class="text-input narrow"
									placeholder="all"
									value={universeList(output)}
									onchange={(e) =>
										data.outputs
											.byId(output.id)
											.universes.set(parseUniverses(e.currentTarget.value))}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={output.node_id ?? ''}
									onchange={(e) =>
										data.outputs.byId(output.id).node_id.set(e.currentTarget.value || null)}
								>
									<option value={thisStation ?? ''}>This station</option>
									<option value="">Every station</option>
									{#if output.node_id && output.node_id !== thisStation}
										<option value={output.node_id}>
											{output.node_id.slice(0, 8)}…
										</option>
									{/if}
								</select>
							</td>
							<td>
								<input
									type="checkbox"
									checked={output.enabled}
									onchange={(e) =>
										data.outputs.byId(output.id).enabled.set(e.currentTarget.checked)}
								/>
							</td>
							<td class="status">
								<span class="dot" class:on={healthy(output)} class:bad={!!status?.last_error}></span>
								<span class="summary" title={status?.last_error ?? ''}>{summarise(output)}</span>
							</td>
							<td>
								<button
									class="danger"
									title="Delete output"
									onclick={() => data.outputs.byId(output.id).delete()}>×</button
								>
							</td>
						</tr>
						{#if status?.last_error}
							<tr class="error-row">
								<td colspan="8">{status.last_error}</td>
							</tr>
						{/if}
					{/each}
				</tbody>
			</table>
			<p class="note">
				Status is what this station is doing. An output set to <em>Every station</em> is sent by
				each of them, which is two copies on the wire unless that is what you wanted.
			</p>
		{/if}
	</section>
</div>

<style>
	.outputs { padding: 16px 20px; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.wires { width: 100%; border-collapse: collapse; font-size: 13px; }
	.wires th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding: 0 6px 6px 0; }
	.wires td { padding: 3px 6px 3px 0; vertical-align: middle; }
	.wires tr.off td { opacity: 0.5; }
	.error-row td { color: #e05555; font-size: 12px; padding-bottom: 6px; }
	.status { display: flex; align-items: center; gap: 6px; }
	.dot { width: 7px; height: 7px; border-radius: 50%; background: #555; flex-shrink: 0; }
	.dot.on { background: #4ade80; }
	.dot.bad { background: #e05555; }
	.summary { color: #bbb; font-variant-numeric: tabular-nums; }
	.hint { color: #777; font-size: 12px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 4px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.note { color: #666; font-size: 12px; margin-top: 10px; font-style: italic; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.text-input.narrow { width: 84px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost:hover { border-color: #555; color: #fff; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
