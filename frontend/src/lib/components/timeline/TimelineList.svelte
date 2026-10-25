<script lang="ts">
	/**
	 * Every timeline, as cards: what each is doing, and which one is open.
	 *
	 * **A tap opens it for editing**, and does not play or stop it. It used to run it —
	 * the console's grammar, a card acts — and at the gig (2026-10-25) that was a timeline
	 * started or stopped on stage every time somebody only meant to look at another one.
	 * Playing is the transport's, above the lanes; firing a scare from a tablet is the
	 * surface's, which has a timeline tile for exactly that.
	 *
	 * Narrower than a card, the list is a dropdown, which is all a phone-width tile has
	 * room for.
	 */

	import Plus from '@lucide/svelte/icons/plus';

	import type { Timeline } from '$lib/generated/index.js';
	import Card from '$lib/components/objects/Card.svelte';
	import { canEdit } from '$lib/stores/showLock.js';
	import { createTimeline, lapOf, now, openTimelineId, positionOf, timelines } from '$lib/stores/timelines.js';
	import { clock } from '$lib/waveform.js';
	import NameDialog from '../NameDialog.svelte';

	let { open, narrow }: { open: Timeline | null; narrow: boolean } = $props();

	let creating = $state(false);

	function stateOf(timeline: Timeline, at: number | null): { text: string; progress: number | null } {
		if (!timeline.running) {
			return { text: timeline.end ? `stopped · ${clock(timeline.end.at_ms)}` : 'stopped', progress: null };
		}
		const position = positionOf(timeline, at);
		if (position === null) return { text: 'running', progress: null };
		const { position: inLap, lap } = lapOf(timeline, position);
		if (timeline.end?.then === 'Loop') {
			return { text: `going round · lap ${lap + 1} · ${clock(inLap)}`, progress: inLap / timeline.end.at_ms };
		}
		return {
			text: timeline.end ? `${clock(position)} / ${clock(timeline.end.at_ms)}` : clock(position),
			progress: timeline.end ? Math.min(1, position / timeline.end.at_ms) : null
		};
	}
</script>

{#if narrow}
	<div class="narrow">
		<select class="select" value={open?.id ?? ''} onchange={(e) => openTimelineId.set(e.currentTarget.value)} aria-label="Timeline">
			{#each $timelines as t (t.id)}
				<option value={t.id}>{t.running ? '▶ ' : ''}{t.name}</option>
			{/each}
		</select>
		{#if $canEdit}
			<button class="btn btn-icon" title="New timeline" aria-label="New timeline" onclick={() => (creating = true)}><Plus size={14} /></button>
		{/if}
	</div>
{:else}
	<div class="list">
		<div class="head">
			<span class="title">Timelines</span>
			{#if $canEdit}
				<button class="btn btn-icon" title="New timeline" aria-label="New timeline" onclick={() => (creating = true)}><Plus size={14} /></button>
			{/if}
		</div>
		{#each $timelines as t (t.id)}
			{@const s = stateOf(t, $now)}
			<Card
				ref={{ kind: 'timeline', id: t.id }}
				name={t.name}
				state={s.text}
				progress={s.progress}
				on={t.id === open?.id}
				onact={() => openTimelineId.set(t.id)}
			/>
		{:else}
			<p class="note">
				No timelines. One is a position and what is written against it — a scare, a song, an
				ambient bed, anything that has to happen at a time rather than on a button.
			</p>
		{/each}
	</div>
{/if}

{#if creating}
	<NameDialog
		title="New timeline"
		placeholder="What is it for?"
		onsubmit={(name) => createTimeline(name)}
		onclose={() => (creating = false)}
	/>
{/if}

<style>
	.list {
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-width: 0;
	}
	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		min-height: 30px;
	}
	.title {
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
		font-weight: 600;
	}
	.head .btn {
		min-height: 28px;
	}
	.narrow {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.narrow .select {
		width: 100%;
	}
</style>
