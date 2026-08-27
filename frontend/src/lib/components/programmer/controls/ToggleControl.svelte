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
		gap: 7px;
		padding: 2px 8px 2px 2px;
		border: 1px solid var(--line-strong);
		border-radius: 12px;
		background: var(--bg-sunken);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.toggle.on {
		border-color: var(--live);
		color: var(--live);
	}

	.knob {
		width: 16px;
		height: 16px;
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
