<script lang="ts">
	/** On or off: a relay, a dry contact the console closes. */

	let {
		value,
		oninput,
		label = 'On'
	}: { value: boolean; oninput: (value: boolean) => void; label?: string } = $props();
</script>

<button
	type="button"
	class="toggle"
	class:on={value}
	role="switch"
	aria-checked={value}
	aria-label={label}
	onclick={() => oninput(!value)}
>
	<span class="knob"></span>
	<span class="text">{value ? 'on' : 'off'}</span>
</button>

<style>
	.toggle {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		/* Pressed, not dragged, so the height is padding around a small knob rather
		   than a 44px pill. */
		min-height: var(--control-h);
		padding: 2px 14px 2px 4px;
		border: 1px solid var(--line-strong);
		border-radius: 12px;
		background: var(--bg-sunken);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-sm);
		cursor: pointer;
	}
	.toggle.on {
		border-color: var(--live);
		color: var(--live);
	}

	.knob {
		width: 20px;
		height: 20px;
		border-radius: 50%;
		background: var(--line-input);
		transition: background 0.12s;
	}
	.toggle.on .knob {
		background: var(--live);
	}

	.text {
		font-family: monospace;
	}
</style>
