<script lang="ts">
	import { contacts, readValue, sensors, type Snapshot } from './node.js';

	let {
		node,
		oncontact,
		onreading
	}: {
		node: Snapshot;
		oncontact: (port: number, state: boolean) => void;
		onreading: (port: number, value: number) => void;
	} = $props();

	// The panel holds what it has sent, because a contact is an edge rather than a
	// level on the wire: the node publishes the change and keeps nothing, so the
	// last thing published is all there is to read back.
	let closed = $state<Record<number, boolean>>({});
	let readings = $state<Record<number, number>>({});

	const ports = $derived(contacts(node.module));
	const measurements = $derived(sensors(node.module));

	function toggle(port: number) {
		closed[port] = !closed[port];
		oncontact(port, closed[port]);
	}

	function send(port: number, value: number) {
		readings[port] = value;
		onreading(port, value);
	}

	/** What the node last actually published, as opposed to what was asked for. */
	function published(port: number): number | null {
		return readValue(node.inputs[String(port)]);
	}
</script>

{#if ports.length > 0 || measurements.length > 0}
	<section>
		<h2>Inputs</h2>

		{#if ports.length > 0}
			<div class="contacts">
				{#each ports as port (port)}
					<button class:closed={closed[port]} onclick={() => toggle(port)}>
						<span class="port mono">{port}</span>
						<span class="state">{closed[port] ? 'closed' : 'open'}</span>
					</button>
				{/each}
			</div>
		{/if}

		{#each measurements as sensor (sensor.port)}
			<label class="sensor">
				<span class="name">{sensor.label}</span>
				<input
					type="range"
					min={sensor.min}
					max={sensor.max}
					step="0.5"
					value={readings[sensor.port] ?? sensor.min}
					oninput={(e) => send(sensor.port, Number(e.currentTarget.value))}
				/>
				<span class="mono value">
					{(readings[sensor.port] ?? published(sensor.port) ?? sensor.min).toFixed(1)}{sensor.unit}
				</span>
			</label>
		{/each}
	</section>
{/if}

<style>
	section {
		padding: 16px 20px;
		border-bottom: 1px solid var(--line);
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.contacts {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(88px, 1fr));
		gap: 8px;
	}

	button {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 4px;
		padding: 10px 12px;
		background: #20261f;
		border: 1px solid var(--line);
		border-radius: 5px;
		color: var(--dim);
		cursor: pointer;
		font: inherit;
		transition:
			background 0.15s,
			border-color 0.15s,
			color 0.15s;
	}

	button:hover {
		border-color: #3d463f;
	}

	button.closed {
		background: #1f3a22;
		border-color: var(--live);
		color: var(--text);
	}

	.port {
		font-size: 0.95rem;
		font-weight: 600;
	}

	.state {
		font-size: 0.7rem;
		letter-spacing: 0.05em;
		text-transform: uppercase;
	}

	.sensor {
		display: grid;
		grid-template-columns: 8rem 1fr 4.5rem;
		align-items: center;
		gap: 12px;
	}

	.name {
		font-size: 0.8rem;
	}

	input[type='range'] {
		width: 100%;
		accent-color: var(--live);
	}

	.value {
		text-align: right;
		color: var(--dim);
	}
</style>
