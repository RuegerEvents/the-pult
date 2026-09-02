<script lang="ts">
	/**
	 * One parameter's home value, editable.
	 *
	 * A control per kind of value rather than a text box holding JSON: a level is a
	 * percentage, a relay is on or off, a colour is a colour. Small enough to sit in a
	 * patch row, because that is where an operator is when they decide that this
	 * house light comes up rather than going dark.
	 *
	 * It never asks what the value *should* be — the caller passes what is showing,
	 * override or type default, and the station is the only thing that resolves which.
	 */

	import type { ParameterValue } from '$lib/generated/index.js';

	let {
		label,
		value,
		onchange
	}: {
		label: string;
		value: ParameterValue;
		onchange: (next: ParameterValue) => void;
	} = $props();

	const hex = (n: number) =>
		Math.round(Math.min(1, Math.max(0, n)) * 255)
			.toString(16)
			.padStart(2, '0');

	const asHex = (v: Extract<ParameterValue, { type: 'Color' }>) =>
		`#${hex(v.value.r)}${hex(v.value.g)}${hex(v.value.b)}`;

	const fromHex = (text: string) => ({
		r: parseInt(text.slice(1, 3), 16) / 255,
		g: parseInt(text.slice(3, 5), 16) / 255,
		b: parseInt(text.slice(5, 7), 16) / 255
	});
</script>

{#if value.type === 'Float'}
	<!-- Percent, like every other level in the console; the wire is 0–1. -->
	<input
		class="num"
		type="number"
		min="0"
		max="100"
		step="1"
		aria-label="{label} rests at"
		value={Math.round(value.value * 100)}
		onchange={(e) =>
			onchange({ type: 'Float', value: Math.min(1, Math.max(0, e.currentTarget.valueAsNumber / 100)) })}
	/>
{:else if value.type === 'Int'}
	<input
		class="num"
		type="number"
		step="1"
		aria-label="{label} rests at"
		value={value.value}
		onchange={(e) => onchange({ type: 'Int', value: Math.round(e.currentTarget.valueAsNumber) })}
	/>
{:else if value.type === 'Bool'}
	<input
		type="checkbox"
		aria-label="{label} rests on"
		checked={value.value}
		onchange={(e) => onchange({ type: 'Bool', value: e.currentTarget.checked })}
	/>
{:else if value.type === 'Color'}
	<input
		class="colour"
		type="color"
		aria-label="{label} rests at"
		value={asHex(value)}
		onchange={(e) =>
			onchange({
				type: 'Color',
				// The pinned emitters survive an edit of the colour: somebody who set
				// the white by hand and then dragged the picker meant to change the
				// colour, not to let go of the white.
				value: { ...fromHex(e.currentTarget.value), overrides: value.value.overrides }
			})}
	/>
{:else}
	<input
		class="text"
		type="text"
		aria-label="{label} rests at"
		value={value.value}
		onchange={(e) => onchange({ type: 'Text', value: e.currentTarget.value })}
	/>
{/if}

<style>
	.num {
		width: 3.5em;
	}
	.text {
		width: 7em;
	}
	.colour {
		width: 2em;
		height: 1.5em;
		padding: 0;
	}
	input {
		font: inherit;
		border: 1px solid var(--border, #444);
		border-radius: 3px;
		background: var(--input-bg, transparent);
		color: inherit;
	}
</style>
