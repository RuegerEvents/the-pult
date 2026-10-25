<script lang="ts">
	/**
	 * On or off: a relay, a dry contact the console closes.
	 *
	 * A switch says which it is by where its knob is — right and lit is on — so it
	 * carries no word beside it repeating that.
	 */

	let {
		value,
		oninput,
		label = 'On'
	}: { value: boolean; oninput: (value: boolean) => void; label?: string } = $props();
</script>

<button
	type="button"
	class="toggle hit"
	class:on={value}
	role="switch"
	aria-checked={value}
	aria-label={label}
	title={value ? 'On' : 'Off'}
	onclick={() => oninput(!value)}
>
	<span class="knob"></span>
</button>

<style>
	.toggle {
		position: relative;
		flex: none;
		width: 40px;
		height: 22px;
		padding: 0;
		border: 1px solid var(--line-strong);
		border-radius: 11px;
		background: var(--bg-sunken);
		cursor: pointer;
		transition:
			background 0.12s,
			border-color 0.12s;
	}
	.toggle.on {
		border-color: var(--live);
		background: color-mix(in srgb, var(--live) 35%, var(--bg-sunken));
	}

	.knob {
		position: absolute;
		top: 2px;
		left: 2px;
		width: 16px;
		height: 16px;
		border-radius: 50%;
		background: var(--line-input);
		transition:
			transform 0.12s,
			background 0.12s;
	}
	.toggle.on .knob {
		transform: translateX(18px);
		background: var(--live);
	}
</style>
