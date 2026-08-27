<script lang="ts">
	import {
		readBrightness,
		readColor,
		readState,
		readText,
		switches,
		type Snapshot
	} from './node.js';

	let { node }: { node: Snapshot } = $props();

	const relays = $derived(switches(node.module));
	const colour = $derived(readColor(node.outputs['0']));
	const brightness = $derived(readBrightness(node.outputs['1']) ?? readBrightness(node.outputs['0']));
	const text = $derived(readText(node.outputs['0']));

	/** Anything the console sent that this panel has no picture for. */
	const unrecognised = $derived(
		Object.entries(node.outputs).filter(
			([port, value]) =>
				!(node.module === 'relay' || node.module === 'contact'
					? relays.includes(Number(port))
					: node.module === 'led'
						? port === '0' || port === '1'
						: node.module === 'oled'
							? port === '0'
							: false) || value === null
		)
	);
</script>

<section>
	<h2>Outputs</h2>

	{#if relays.length > 0}
		<!-- Read-only on purpose: these are what the console drives. A panel that
		     could flip them would be inventing state the console does not know about. -->
		<div class="relays">
			{#each relays as port (port)}
				{@const on = readState(node.outputs[String(port)])}
				<div class="relay" class:on>
					<span class="port mono">{port}</span>
					<span class="lamp"></span>
					<span class="state">{on ? 'closed' : 'open'}</span>
				</div>
			{/each}
		</div>
	{/if}

	{#if node.module === 'led'}
		<div class="strip" style:background={colour ?? '#000'}>
			<span class="mono">{colour ?? 'unlit'}</span>
			{#if brightness !== null}
				<span class="mono dim">{Math.round(brightness * 100)}%</span>
			{/if}
		</div>
	{/if}

	{#if node.module === 'oled'}
		<div class="oled mono">{text ?? ''}</div>
	{/if}

	{#if node.module === 'dmx'}
		<p class="note">
			A gateway has no ports of its own — what it is sent arrives as sACN, below.
		</p>
	{/if}

	{#if unrecognised.length > 0}
		<pre class="mono raw">{JSON.stringify(Object.fromEntries(unrecognised), null, 2)}</pre>
	{:else if relays.length === 0 && node.module !== 'led' && node.module !== 'oled' && node.module !== 'dmx'}
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

	.relays {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(110px, 1fr));
		gap: 8px;
	}

	.relay {
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

	.state {
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

	.strip .dim {
		opacity: 0.7;
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
