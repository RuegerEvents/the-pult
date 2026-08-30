<script lang="ts">
	/** A whole number, 0–255: a gobo index, a macro slot, a raw channel. */

	let {
		value,
		min = 0,
		max = 255,
		oninput
	}: { value: number; min?: number; max?: number; oninput: (value: number) => void } = $props();

	const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v)));
	const step = (by: number) => oninput(clamp(value + by));
</script>

<div class="stepper">
	<button type="button" aria-label="Down" onclick={() => step(-1)}>−</button>
	<input
		type="number"
		{min}
		{max}
		{value}
		oninput={(e) => oninput(clamp(Number(e.currentTarget.value)))}
	/>
	<button type="button" aria-label="Up" onclick={() => step(1)}>+</button>
</div>

<style>
	.stepper {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	button {
		width: var(--control-h);
		height: var(--control-h);
		flex: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		background: none;
		color: var(--text-dim);
		font: inherit;
		line-height: 1;
		cursor: pointer;
	}
	button:hover {
		border-color: var(--line-input);
		color: var(--text-bright);
	}

	input {
		width: 72px;
		min-height: var(--control-h);
		background: var(--bg-sunken);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-family: monospace;
		font-size: var(--font-sm);
		padding: 3px 8px;
		text-align: center;
	}
	input:focus {
		outline: none;
		border-color: var(--accent);
	}
</style>
