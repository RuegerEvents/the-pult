<script lang="ts">
	/**
	 * The right control for whatever kind of value a parameter holds.
	 *
	 * One place decides this, so the values panel, the quicksheet and anything after
	 * them cannot end up offering a fader for a relay.
	 */

	import type { ParameterValue } from '$lib/generated/index.js';
	import ColorControl from './ColorControl.svelte';
	import Fader from './Fader.svelte';
	import IntControl from './IntControl.svelte';
	import TextControl from './TextControl.svelte';
	import ToggleControl from './ToggleControl.svelte';

	let {
		value,
		label,
		tint = 'var(--accent)',
		oninput
	}: {
		value: ParameterValue;
		label: string;
		tint?: string;
		oninput: (value: ParameterValue) => void;
	} = $props();
</script>

{#if value.type === 'Color'}
	<ColorControl
		value={value.value}
		oninput={(rgb) =>
			oninput({ type: 'Color', value: { ...rgb, overrides: value.value.overrides } })}
	/>
{:else if value.type === 'Float'}
	<Fader
		{label}
		{tint}
		value={value.value}
		oninput={(v) => oninput({ type: 'Float', value: v })}
	/>
{:else if value.type === 'Int'}
	<IntControl value={value.value} oninput={(v) => oninput({ type: 'Int', value: v })} />
{:else if value.type === 'Bool'}
	<ToggleControl
		{label}
		value={value.value}
		oninput={(v) => oninput({ type: 'Bool', value: v })}
	/>
{:else}
	<TextControl value={value.value} oninput={(v) => oninput({ type: 'Text', value: v })} />
{/if}
