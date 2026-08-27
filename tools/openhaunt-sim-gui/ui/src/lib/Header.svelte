<script lang="ts">
	import Dot from './Dot.svelte';
	import { uptime, type Snapshot } from './node.js';

	let { node }: { node: Snapshot } = $props();

	// Uptime is worked out here from the moment the node started rather than sent
	// with every update, so it keeps counting on a node nothing is happening to.
	let now = $state(Date.now());
	$effect(() => {
		const tick = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(tick);
	});
</script>

<header>
	<div class="identity">
		<h1>{node.moduleName}</h1>
		<span class="mono serial">{node.serial}</span>
		{#if node.switchesMains}
			<span class="mains" title="Descriptor bit 6: this module switches mains">mains</span>
		{/if}
	</div>

	<dl>
		<div><dt>Control</dt><dd class="mono">{node.httpAddr || '—'}</dd></div>
		<div><dt>Module</dt><dd class="mono">0x{node.typeId.toString(16).padStart(4, '0')}</dd></div>
		{#if node.caps}
			<div><dt>Caps</dt><dd class="mono">{node.caps}</dd></div>
		{/if}
		{#if node.sacnAddr}
			<div><dt>sACN</dt><dd class="mono">{node.sacnAddr}</dd></div>
		{/if}
		<div><dt>Up</dt><dd>{uptime(node.startedMs, now)}</dd></div>
	</dl>

	<div class="lamps">
		<Dot on={node.advertising} label="mDNS" />
		<!-- Adoption is the whole handshake in one flag: a node is discovered, not
		     configured, so being told where the broker is is the only setup it gets. -->
		<Dot on={node.adopted} label={node.broker ? `adopted · ${node.broker}` : 'not adopted'} />
		<Dot on={node.mqttConnected} label="MQTT" />
		{#if node.identified > 0}
			<Dot on={true} tone="warn" label={`identify ×${node.identified}`} />
		{/if}
	</div>
</header>

<style>
	header {
		padding: 16px 20px;
		background: var(--panel);
		border-bottom: 1px solid var(--line);
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.identity {
		display: flex;
		align-items: baseline;
		gap: 10px;
		flex-wrap: wrap;
	}

	h1 {
		font-size: 1rem;
		font-weight: 600;
	}

	.serial {
		color: var(--dim);
	}

	.mains {
		font-size: 0.68rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: #1a1a1a;
		background: var(--warn);
		border-radius: 3px;
		padding: 2px 6px;
	}

	dl {
		display: flex;
		flex-wrap: wrap;
		gap: 6px 24px;
	}

	dl div {
		display: flex;
		gap: 8px;
		align-items: baseline;
	}

	dt {
		font-size: 0.7rem;
		letter-spacing: 0.07em;
		text-transform: uppercase;
		color: var(--dim);
	}

	dd {
		font-size: 0.8rem;
	}

	.lamps {
		display: flex;
		flex-wrap: wrap;
		gap: 8px 20px;
	}
</style>
