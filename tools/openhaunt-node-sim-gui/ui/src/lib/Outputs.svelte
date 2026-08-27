<script lang="ts">
	import {
		outputPorts,
		readColor,
		readNumber,
		readState,
		readText,
		unitLabel,
		type Snapshot
	} from './node.js';

	let { node }: { node: Snapshot } = $props();

	// The node said what each terminal is; the panel draws it accordingly. Nothing
	// here knows a relay from a strip — the data type does the deciding.
	const ports = $derived(outputPorts(node));

	/// A number as the port's own unit asks for it: `percent` is the one a
	/// console sends as 0–1 and an operator reads as 0–100.
	function format(level: number, unit: string | undefined): string {
		if (unit === 'percent') return `${Math.round(level * 100)}%`;
		return `${level}${unitLabel(unit)}`;
	}

	/** Anything the console sent to a port this node never described. */
	const unrecognised = $derived(
		Object.entries(node.outputs).filter(
			([port]) => !ports.some((p) => String(p.port) === port)
		)
	);
</script>


<section>
	<h2>Outputs</h2>

	<!-- Read-only on purpose: these are what the console drives. A panel that could
	     flip them would be inventing state the console does not know about. -->
	{#each ports as port (port.port)}
		{#if port.dataType === 'boolean'}
			{@const on = readState(node.outputs[String(port.port)])}
			<div class="relay" class:on>
				<span class="port mono">{port.port}</span>
				<span class="lamp"></span>
				<span class="name">{port.name}</span>
				<span class="state">{on ? 'closed' : 'open'}</span>
			</div>
		{:else if port.dataType === 'color'}
			{@const colour = readColor(node.outputs[String(port.port)])}
			<div class="strip" style:background={colour ?? '#000'}>
				<span class="mono">{colour ?? 'unlit'}</span>
			</div>
		{:else if port.dataType === 'number'}
			{@const level = readNumber(node.outputs[String(port.port)])}
			<div class="reading">
				<span class="port mono">{port.port}</span>
				<span class="name">{port.name}</span>
				<span class="mono value">
					{level === null ? '—' : format(level, port.unit)}
				</span>
			</div>
		{:else}
			<div class="oled mono">{readText(node.outputs[String(port.port)]) ?? ''}</div>
		{/if}
	{/each}

	{#if node.config.dmx}
		<p class="note">
			A gateway has no ports of its own — what it is sent arrives as sACN, below.
		</p>
	{/if}

	{#if unrecognised.length > 0}
		<pre class="mono raw">{JSON.stringify(Object.fromEntries(unrecognised), null, 2)}</pre>
	{:else if ports.length === 0 && !node.config.dmx}
		<p class="note">This module drives nothing.</p>
	{/if}
</section>

<style>
	section {
		padding: 16px 20px;
		border-bottom: 1px solid var(--line);
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.relay,
	.reading {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 12px;
		background: #20261f;
		border: 1px solid var(--line);
		border-radius: 5px;
		color: var(--dim);
	}

	.relay.on {
		border-color: var(--warn);
		color: var(--text);
	}

	.name {
		font-size: 0.8rem;
	}

	.lamp {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		background: #3a423d;
	}

	.relay.on .lamp {
		background: var(--warn);
		box-shadow: 0 0 10px var(--warn);
	}

	.port {
		font-weight: 600;
	}

	.state,
	.value {
		margin-left: auto;
		font-size: 0.7rem;
		letter-spacing: 0.05em;
		text-transform: uppercase;
	}

	.strip {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		height: 56px;
		padding: 0 14px;
		border: 1px solid var(--line);
		border-radius: 5px;
		mix-blend-mode: normal;
		color: #000;
		text-shadow: 0 0 6px rgb(255 255 255 / 0.6);
		transition: background 0.15s;
	}

	.oled {
		min-height: 56px;
		padding: 12px 14px;
		background: #05140a;
		border: 1px solid var(--line);
		border-radius: 5px;
		color: #7dffa4;
		white-space: pre-wrap;
	}

	.note,
	.raw {
		font-size: 0.8rem;
		color: var(--dim);
	}

	.raw {
		padding: 10px 12px;
		background: #12160f;
		border: 1px solid var(--line);
		border-radius: 5px;
		overflow-x: auto;
	}
</style>
