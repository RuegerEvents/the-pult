<script lang="ts">
	/**
	 * A flash: one press of a button recorded onto a timeline — a value on one parameter
	 * from when it was pressed to when it was let go. Played over every take and cutting
	 * none, so where it starts, how long it lasts and what it puts there are the whole of
	 * it. A level is set as a percentage; any other value is the one the key put there.
	 */

	import type { ParameterValue, TimelineFlash } from '$lib/generated/index.js';
	import type { FlashRow } from '$lib/kinds/timeline.kind.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { current, locate, writeColumns } from '$lib/stores/timelines.js';
	import { clock, parsePosition } from '$lib/waveform.js';
	import Help from '../../Help.svelte';

	let { rows }: { rows: FlashRow[] } = $props();

	async function change(row: FlashRow, patch: Partial<TimelineFlash>) {
		const timeline = current(row.timelineId);
		if (!timeline) return;
		const flashes = (timeline.flashes ?? []).map((f) => (f.id === row.id ? { ...f, ...patch } : f));
		await writeColumns(row.timelineId, { flashes });
	}

	/** A level as a percentage, where the value is one. */
	function percent(value: ParameterValue): number | null {
		if (value.type === 'Float') return Math.round(value.value * 1000) / 10;
		return null;
	}

	function described(value: ParameterValue): string {
		switch (value.type) {
			case 'Int':
				return String(value.value);
			case 'Bool':
				return value.value ? 'on' : 'off';
			case 'Color':
				return `rgb ${[value.value.r, value.value.g, value.value.b].map((c) => Math.round(c * 255)).join(' ')}`;
			case 'Text':
				return value.value;
			default:
				return '';
		}
	}
</script>

{#each rows as row (row.id)}
	{@const level = percent(row.flash.value)}
	<div class="fields">
		{#if rows.length > 1}<span class="wide label">{row.name}</span>{/if}
		<span class="label"
			>At <Help label="What a flash does"
				>A flash key pressed while the timeline recorded. It holds its value for as long as the key was
				held, over every take on its parameter, and then lets the take carry on. Recording again adds
				flashes and never removes one. Drag it to move it, drag an end to make it longer or shorter, and
				delete it to take it out.</Help
			></span
		>
		<div class="inline">
			<input
				class="input narrow"
				disabled={!$canEdit}
				value={clock(row.flash.at_ms)}
				onchange={(e) => {
					const ms = parsePosition(e.currentTarget.value);
					if (ms !== null) void change(row, { at_ms: Math.max(0, Math.round(ms)) });
				}}
			/>
			<button class="btn btn-ghost" onclick={() => locate(row.timelineId, row.flash.at_ms)}>Go to</button>
		</div>
		<span class="label">For</span>
		<div class="inline">
			<input
				class="input narrow"
				type="number"
				min="1"
				step="10"
				disabled={!$canEdit}
				value={row.flash.duration_ms}
				onchange={(e) => {
					const ms = Math.round(Number(e.currentTarget.value));
					if (Number.isFinite(ms) && ms >= 1) void change(row, { duration_ms: ms });
				}}
			/>
			<span class="note">ms</span>
		</div>
		<span class="label">Value</span>
		{#if level !== null}
			<div class="inline">
				<input
					class="input narrow"
					type="number"
					min="0"
					max="100"
					step="1"
					disabled={!$canEdit}
					value={level}
					onchange={(e) => {
						const v = Number(e.currentTarget.value);
						if (Number.isFinite(v)) void change(row, { value: { type: 'Float', value: Math.max(0, Math.min(100, v)) / 100 } });
					}}
				/>
				<span class="note">%</span>
			</div>
		{:else}
			<span class="note">{described(row.flash.value)}</span>
		{/if}
	</div>
{/each}
