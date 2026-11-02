<script lang="ts">
	/**
	 * The right control for whatever kind of value a parameter holds.
	 *
	 * One place decides this, so the values panel, the quicksheet and anything after
	 * them cannot end up offering a fader for a relay.
	 */

	import type { ParameterValue, PhysicalRange } from '$lib/generated/index.js';
	import ColorControl from './ColorControl.svelte';
	import Fader from './Fader.svelte';
	import IntControl from './IntControl.svelte';
	import TextControl from './TextControl.svelte';
	import ToggleControl from './ToggleControl.svelte';
	import ChoiceControl from './ChoiceControl.svelte';

	let {
		value,
		label,
		tint = 'var(--accent)',
		choices = [],
		range = null,
		flash = false,
		oninput
	}: {
		value: ParameterValue;
		label: string;
		tint?: string;
		/** The words a text value may be, where its type names them. */
		choices?: string[];
		/** What a number's 0..1 spans, so its readout is in its own unit. */
		range?: PhysicalRange | null;
		/** A held-only button beside an on/off switch, for trying a relay while programming. */
		flash?: boolean;
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
		{range}
		value={value.value}
		oninput={(v) => oninput({ type: 'Float', value: v })}
	/>
{:else if value.type === 'Int'}
	<IntControl value={value.value} oninput={(v) => oninput({ type: 'Int', value: v })} />
{:else if value.type === 'Bool'}
	<ToggleControl
		{label}
		{flash}
		value={value.value}
		oninput={(v) => oninput({ type: 'Bool', value: v })}
	/>
{:else if choices.length > 0}
	<ChoiceControl {choices} value={value.value} oninput={(v) => oninput({ type: 'Text', value: v })} />
{:else}
	<TextControl value={value.value} oninput={(v) => oninput({ type: 'Text', value: v })} />
{/if}
