<script lang="ts">
	import {
		inputPorts,
		readNumber,
		unitLabel,
		type PortDescription,
		type Snapshot
	} from './node.js';

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

	// Whatever the node said it reads, split by the only thing that changes how a
	// terminal is driven from here: a boolean is a button, a number is a slider.
	const ports = $derived(inputPorts(node).filter((p) => p.dataType === 'boolean'));
	const measurements = $derived(inputPorts(node).filter((p) => p.dataType !== 'boolean'));

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
		return readNumber(node.inputs[String(port)]);
	}

	/// A slider needs ends. A port that declared none gets 0–1, which is what an
	/// undeclared number most often is.
	const low = (p: PortDescription) => p.minimum ?? 0;
	const high = (p: PortDescription) => p.maximum ?? 1;
	const step = (p: PortDescription) => (high(p) - low(p)) / 100;
</script>

{#if ports.length > 0 || measurements.length > 0}
	<section>
		<h2>Inputs</h2>

		{#if ports.length > 0}
			<div class="contacts">
				{#each ports as port (port.port)}
					<button class:closed={closed[port.port]} onclick={() => toggle(port.port)}>
						<span class="port mono">{port.port}</span>
						<span class="name">{port.name}</span>
						<span class="state">{closed[port.port] ? 'closed' : 'open'}</span>
					</button>
				{/each}
			</div>
		{/if}

		{#each measurements as sensor (sensor.port)}
			<label class="sensor">
				<span class="name">{sensor.name}</span>
				<input
					type="range"
					min={low(sensor)}
					max={high(sensor)}
					step={step(sensor)}
					value={readings[sensor.port] ?? low(sensor)}
					oninput={(e) => send(sensor.port, Number(e.currentTarget.value))}
				/>
				<span class="mono value">
					{(readings[sensor.port] ?? published(sensor.port) ?? low(sensor)).toFixed(1)}{unitLabel(
						sensor.unit
					)}
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
		grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
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

	button .name {
		font-size: 0.78rem;
		color: var(--text);
		text-align: left;
	}

	.state {
		font-size: 0.7rem;
		letter-spacing: 0.05em;
		text-transform: uppercase;
	}

	.sensor {
		display: grid;
		grid-template-columns: 8rem 1fr 7rem;
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
		/* A unit this panel has no short form for is the node's own word and can be
		   long. Better clipped on one line than four lines tall. */
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
