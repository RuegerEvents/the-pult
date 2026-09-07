<script lang="ts">
	import { onMount } from 'svelte';
	import { focusOnMount } from '$lib/actions.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import OutputGaps from './OutputGaps.svelte';
	import type {
		InputConfig,
		InputKind,
		InputStatus,
		OutputConfig,
		OutputKind,
		OutputStatus
	} from '$lib/generated/index.js';

	const client = getClientContext();
	const data = getDataContext();

	type Statuses = Record<string, OutputStatus>;
	type InputStatuses = Record<string, InputStatus>;

	let outputs = $state<OutputConfig[]>([]);
	let statuses = $state<Statuses>({});
	let inputs = $state<InputConfig[]>([]);
	let inputStatuses = $state<InputStatuses>({});
	let thisStation = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let newKind = $state<OutputKind>('Artnet');
	let creatingInput = $state(false);
	let newInputName = $state('');
	let newInputKind = $state<InputKind>('Sacn');

	const INPUT_KINDS: { value: InputKind; label: string }[] = [
		{ value: 'Sacn', label: 'sACN' },
		{ value: 'Artnet', label: 'Art-Net' }
	];

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

	/** The universe map as an operator types it: `1→1, 2→5`. Empty listens to nothing. */
	const universeMap = (input: InputConfig) =>
		Object.entries(input.universes)
			.map(([wire, patch]) => `${wire}→${patch}`)
			.join(', ');

	/**
	 * Read a map back. Both arrows are accepted and so is a bare number, which means
	 * "this wire universe as itself" — the common case, and one nobody should have to
	 * type twice.
	 */
	function parseUniverseMap(text: string): Record<number, number> {
		const out: Record<number, number> = {};
		for (const part of text.split(/[,;]+/)) {
			const trimmed = part.trim();
			if (!trimmed) continue;
			const [from, to] = trimmed.split(/\s*(?:→|->|:)\s*/);
			const wire = Number(from);
			const patch = to === undefined ? wire : Number(to);
			if (!Number.isFinite(wire) || !Number.isFinite(patch) || wire <= 0 || patch <= 0) continue;
			out[wire] = patch;
		}
		return out;
	}

	function summariseInput(input: InputConfig): string {
		if (!input.enabled) return 'off';
		if (!input.node_id) return 'nobody is listening';
		if (input.node_id !== thisStation) return 'another station';
		const status = inputStatuses[input.id];
		if (!status) return 'not started';
		if (status.last_error && status.error_count > 0) return `${status.error_count} errors`;
		if (!status.last_packet) return 'nothing has arrived';
		const sources = status.sources === 1 ? '1 source' : `${status.sources} sources`;
		return `${status.packets_per_second.toFixed(0)}/s · ${sources}`;
	}

	const inputHealthy = (input: InputConfig): boolean => {
		const status = inputStatuses[input.id];
		return !!status && !!status.last_packet && status.error_count === 0;
	};

	async function createInput() {
		const name = newInputName.trim();
		if (!name) return;
		await data.inputs.create({
			id: crypto.randomUUID(),
			name,
			kind: newInputKind,
			// This station, and there is no "every station" here: a socket is one
			// machine's, and an input nobody is named on is an input nobody is
			// listening to — which the row says in words rather than falling back to
			// the leader the way an output does.
			node_id: thisStation,
			interfaces: {},
			// Empty listens to nothing. Deliberate: an input that guessed the identity
			// mapping would decode a guest console's universe 1 straight over the
			// house rig's.
			universes: {},
			enabled: true
		});
		newInputName = '';
		creatingInput = false;
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
			// This station by default. Leaving it unset means "whichever station may",
			// which for sACN is every one of them and for Art-Net and OpenHaunt is the
			// leader — those two have no way to arbitrate between two senders.
			node_id: thisStation,
			// Told nothing, so this output falls through to the station's own
			// `[network]` preference and then to every interface. Filling one in here
			// would bake a cable into a row that replicates to machines that have
			// never heard of it.
			interfaces: {},
			priority: 'Auto'
		});
		newName = '';
		creating = false;
	}

	onMount(() => {
		const stop = data.outputs.subscribeDeep((v) => { outputs = v; });
		const stopInputs = data.inputs.subscribeDeep((v) => { inputs = v; });

		// output_status and session are LOCAL, subscribed by path like devices.
		const applyStatus = (v: unknown) => {
			if (v && typeof v === 'object') statuses = v as Statuses;
		};
		const applySession = (v: unknown) => {
			if (v && typeof v === 'object') thisStation = (v as { node_id: string | null }).node_id;
		};
		const applyInputStatus = (v: unknown) => {
			if (v && typeof v === 'object') inputStatuses = v as InputStatuses;
		};
		const stopStatus = client.subscribe('output_status', applyStatus);
		const stopInputStatus = client.subscribe('input_status', applyInputStatus);
		const stopSession = client.subscribe('session', applySession);
		const fetchLocal = () => {
			client.get(['output_status']).then(applyStatus);
			client.get(['input_status']).then(applyInputStatus);
			client.get(['session']).then(applySession);
		};
		fetchLocal();
		const stopConnect = client.addConnectListener(fetchLocal);

		return () => {
			stop();
			stopInputs();
			stopStatus();
			stopInputStatus();
			stopSession();
			stopConnect();
		};
	});
</script>

<div class="io">
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

		<OutputGaps />

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
									title="Which universes this output carries. Empty is every one in the patch; a list is a routing, so two outputs can split a rig between two interfaces."
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

	<section class="block">
		<header class="block-head">
			<h2>Inputs</h2>
			<button class="ghost" onclick={() => (creatingInput = !creatingInput)}>
				{creatingInput ? 'Cancel' : '+ Input'}
			</button>
		</header>

		{#if creatingInput}
			<form class="new-row" onsubmit={(e) => { e.preventDefault(); createInput(); }}>
				<input
					class="text-input"
					placeholder="Whose console is it?"
					bind:value={newInputName}
					use:focusOnMount
				/>
				<select class="text-input" bind:value={newInputKind}>
					{#each INPUT_KINDS as kind (kind.value)}
						<option value={kind.value}>{kind.label}</option>
					{/each}
				</select>
				<button class="primary" type="submit">Add</button>
			</form>
			<p class="hint">Then say which wire universes land where in this patch.</p>
		{/if}

		{#if inputs.length === 0}
			<p class="empty">
				Nothing is being listened to. Add an input to grab from another console, or to record
				a take onto a timeline.
			</p>
		{:else}
			<table class="wires">
				<thead>
					<tr>
						<th>Name</th><th>Protocol</th><th>Universes</th>
						<th>Station</th><th>On</th><th>Status</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each inputs as input (input.id)}
						{@const status = inputStatuses[input.id]}
						<tr class:off={!input.enabled}>
							<td>
								<input
									class="text-input"
									value={input.name}
									onchange={(e) => data.inputs.byId(input.id).name.set(e.currentTarget.value)}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={input.kind}
									onchange={(e) =>
										data.inputs.byId(input.id).kind.set(e.currentTarget.value as InputKind)}
								>
									{#each INPUT_KINDS as kind (kind.value)}
										<option value={kind.value}>{kind.label}</option>
									{/each}
								</select>
							</td>
							<td>
								<input
									class="text-input wide"
									placeholder="1→1, 2→5"
									title="Wire universe to patch universe. Empty listens to nothing, so that the other console's 1 never lands on this rig's 1 by accident. A bare number means that universe as itself."
									value={universeMap(input)}
									onchange={(e) =>
										data.inputs
											.byId(input.id)
											.universes.set(parseUniverseMap(e.currentTarget.value))}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={input.node_id ?? ''}
									onchange={(e) =>
										data.inputs.byId(input.id).node_id.set(e.currentTarget.value || null)}
								>
									<option value={thisStation ?? ''}>This station</option>
									<option value="">Nobody</option>
									{#if input.node_id && input.node_id !== thisStation}
										<option value={input.node_id}>{input.node_id.slice(0, 8)}…</option>
									{/if}
								</select>
							</td>
							<td>
								<input
									type="checkbox"
									checked={input.enabled}
									onchange={(e) =>
										data.inputs.byId(input.id).enabled.set(e.currentTarget.checked)}
								/>
							</td>
							<td class="status">
								<span
									class="dot"
									class:on={inputHealthy(input)}
									class:bad={!!status?.last_error}
								></span>
								<span class="summary" title={status?.last_error ?? ''}>{summariseInput(input)}</span>
							</td>
							<td>
								<button
									class="danger"
									title="Delete input"
									onclick={() => data.inputs.byId(input.id).delete()}>×</button
								>
							</td>
						</tr>
						{#if status?.last_error}
							<tr class="error-row">
								<td colspan="7">{status.last_error}</td>
							</tr>
						{/if}
					{/each}
				</tbody>
			</table>
			<p class="note">
				An input holds a socket, so it belongs to one machine: there is no <em>every
				station</em> here, and a row nobody is named on is a row nobody is listening to.
				What arrives stays on that station until somebody grabs it or records it.
			</p>
		{/if}
	</section>
</div>

<style>
	.io { padding: 16px 20px; }
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
	.text-input.wide { width: 140px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost:hover { border-color: #555; color: #fff; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
