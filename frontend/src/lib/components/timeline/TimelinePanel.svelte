<script lang="ts">
	import { untrack } from 'svelte';
	/**
	 * A timeline: its list on the left, its lanes in the middle, and what is picked up on
	 * them in the Inspector on the right — option A of the UI direction (task 80).
	 *
	 * Nothing is a form under the canvas any more. A clip, an event, a marker, a tempo or
	 * a take is an object (`$lib/kinds/timeline.kind.ts`), so clicking one puts it in the
	 * Inspector the way clicking a preset does; with nothing picked up, the docked
	 * Inspector shows the timeline's own settings.
	 *
	 * **The Inspector is docked here only while there is no Inspector panel** in the
	 * layout (or this panel is maximized and has the screen to itself). Two places
	 * editing one clip at once would be two answers to "where do I change this", which is
	 * the confusion the Inspector exists to end — so it is the same component, in one
	 * place at a time.
	 *
	 * The transport is four numbers on a SYNCED row, so the playhead is worked out from
	 * the console's clock exactly as the station works it out, and **nothing ticks on
	 * either side** — and every transport act carries `at`, the way a Go does.
	 *
	 * **The waveform is drawn from what the station handed over**, never decoded here:
	 * peaks are a small asset the playing station reduced once.
	 */

	import { onMount } from 'svelte';

	import PanelRight from '@lucide/svelte/icons/panel-right';
	import Upload from '@lucide/svelte/icons/upload';
	import Flag from '@lucide/svelte/icons/flag';
	import Help from '../Help.svelte';
	import { addToast } from '$lib/toasts.js';
	import { entries as programmerEntries } from '$lib/stores/programmer.js';
	import { collection } from '$lib/stores/show.js';
	import { displayLabel, formatValue } from '$lib/patch.js';
	import { takes } from '$lib/stores/timelines.js';

	import InspectorPanel from '$lib/components/inspector/InspectorPanel.svelte';
	import TimelineInspector from './inspectors/TimelineInspector.svelte';
	import Lanes from './Lanes.svelte';
	import TimelineList from './TimelineList.svelte';
	import Transport from './Transport.svelte';
	import { timelineOfItem } from '$lib/kinds/timeline.kind.js';
	import { panelsIn } from '$lib/layout.js';
	import { selectSubject, subject, subjectRows } from '$lib/objects.js';
	import { maximized, revealPanel, tree } from '$lib/stores/layout.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { firstDestination } from '$lib/stores/zones.js';
	import { getStationContext } from '$lib/ws/context.js';
	import { now, openTimeline, openTimelineId, positionOf, proposals, writeColumns, current } from '$lib/stores/timelines.js';
	import { uuid } from '$lib/uuid.js';

	const station = getStationContext();
	let width = $state(1200);
	let root = $state<HTMLDivElement | null>(null);
	let snapping = $state(true);
	let lanes = $state<{ markerAtPlayhead: () => Promise<void> } | null>(null);
	const fixtures = collection('fixtures');
	const nameOf = (id: string) => $fixtures.find((f) => f.id === id)?.name ?? id.slice(0, 6);
	let uploading = $state(false);

	const timeline = $derived($openTimeline);
	/** What a take from the programmer is recording right now: every key it holds. */
	const take = $derived(timeline ? ($takes?.[timeline.id] ?? null) : null);
	const recordingProgrammer = $derived(
		!!take && !!timeline?.recording && 'Programmer' in timeline.recording
	);
	/** Narrower than this, the list is a dropdown. */
	const narrow = $derived(width < 760);
	const inspectorElsewhere = $derived(panelsIn($tree).includes('inspector') && $maximized !== 'timeline');
	/** The docked Inspector needs room; below this it hands over to the panel. */
	const docked = $derived(!inspectorElsewhere && width >= 900);

	// Picking up anything that belongs to a timeline opens that timeline — when the
	// subject *changes*, and only then. Reading the open timeline in here made it a
	// dependency, so opening another timeline from its card re-ran this, found a clip of
	// the old one still the subject, and put the old one straight back: the card "did
	// nothing" whenever something of another timeline was selected (seen at the gig).
	$effect(() => {
		const refs = $subject;
		untrack(() => {
			for (let i = refs.length - 1; i >= 0; i--) {
				const id = timelineOfItem(refs[i]);
				if (id) {
					if (id !== $openTimelineId) openTimelineId.set(id);
					return;
				}
			}
		});
	});

	/**
	 * Put a file in the show, as a clip at the playhead going out of the first two
	 * channels until somebody says otherwise. By extension, because that is what a file
	 * picker gives — the station takes the four kinds a show arrives as and refuses the
	 * rest by name.
	 */
	async function upload(file: File, atMs?: number) {
		const kinds: Record<string, string> = {
			wav: 'audio/wav',
			wave: 'audio/wav',
			mp3: 'audio/mpeg',
			flac: 'audio/flac',
			m4a: 'audio/mp4',
			mp4: 'audio/mp4',
			aac: 'audio/mp4'
		};
		const mime = kinds[file.name.split('.').pop()?.toLowerCase() ?? ''];
		if (!mime) {
			addToast(`This console plays wav, mp3, flac and m4a. ${file.name} is none of them.`);
			return;
		}
		if (!timeline) return;
		uploading = true;
		try {
			const response = await fetch('/assets', { method: 'POST', headers: { 'content-type': mime }, body: await file.arrayBuffer() });
			if (!response.ok) throw new Error(await response.text());
			const { sha256 } = (await response.json()) as { sha256: string };
			const live = current(timeline.id);
			if (!live) return;
			const id = uuid();
			// Somewhere to go: the show's first zone, or a "Main" made for it on this
			// station's first two channels. One gesture with the clip, so one Ctrl-Z.
			beginGesture();
			try {
				const destination = await firstDestination($station?.nodeId ?? null);
				await writeColumns(live.id, {
					clips: [
						...live.clips,
						{
							id,
							name: file.name.replace(/\.[^.]+$/, ''),
							asset: sha256,
							peaks: null,
							start_ms: Math.round(atMs ?? positionOf(live, $now) ?? 0),
							offset_ms: 0,
							length_ms: null,
							gain_db: 0,
							fade_in_ms: 0,
							fade_out_ms: 0,
							looped: false,
							destination
						}
					]
				});
			} finally {
				endGesture();
			}
			selectSubject({ kind: 'timeline-clip', id });
		} catch (e) {
			addToast(`That file could not be stored: ${e}`);
		} finally {
			uploading = false;
		}
	}

	onMount(() => {
		const observer = new ResizeObserver(() => (width = root?.clientWidth ?? 1200));
		if (root) observer.observe(root);
		return () => observer.disconnect();
	});
</script>

<div class="timeline-panel" class:narrow bind:this={root}>
	{#if narrow}
		<TimelineList open={timeline} narrow={true} />
	{:else}
		<aside class="list">
			<TimelineList open={timeline} narrow={false} />
		</aside>
	{/if}

	<section class="main">
		{#if timeline}
			<div class="bar">
				<Transport {timeline} bind:snapping />
				<div class="actions">
					{#if $canEdit}
						<label class="btn file" title="Add a sound at the playhead">
							<Upload size={14} />{uploading ? 'Storing…' : 'Sound'}
							<input
								type="file"
								accept=".wav,.mp3,.flac,.m4a,.mp4,.aac,audio/*"
								disabled={uploading}
								onchange={(e) => {
									const file = e.currentTarget.files?.[0];
									e.currentTarget.value = '';
									if (file) void upload(file);
								}}
							/>
						</label>
					{/if}
					{#if $canEdit}
						<button class="btn" title="A marker at the playhead (M)" onclick={() => lanes?.markerAtPlayhead()}>
							<Flag size={14} />Marker
						</button>
					{/if}
					<Help label="How the lanes work">
						<p>
							Click to pick up and to locate · drag to move, an edge to trim, empty space to draw a
							band · double-click the event or marker lane to add one · wheel or the buttons to
							zoom.
						</p>
						<p>
							With the lanes focused: <b>Space</b> plays and stops, <b>M</b> drops a marker at the
							playhead, arrows step a second. Drag sound files in from the desktop to place them.
						</p>
					</Help>
					{#if !docked && !inspectorElsewhere}
						<button class="btn btn-icon" title="Open the Inspector" aria-label="Open the Inspector" onclick={() => revealPanel('inspector')}>
							<PanelRight size={14} />
						</button>
					{/if}
				</div>
			</div>
			{#if recordingProgrammer}
				<!-- What the take is holding, live: the keys the programmer has and where
				     each one is, so recording is watched rather than trusted. -->
				<div class="recording-strip" aria-live="polite">
					<span class="rec"><span class="dot"></span>Recording</span>
					{#if $programmerEntries.length === 0}
						<span class="dim">move something in the programmer</span>
					{/if}
					{#each $programmerEntries as entry (entry.id)}
						<span class="param">
							{nameOf(entry.fixture_id)} · {displayLabel(entry.parameter_kind)}
							<b>{entry.effect ? 'effect' : formatValue(entry.value)}</b>
						</span>
					{/each}
				</div>
			{/if}
			<Lanes
				bind:this={lanes}
				{timeline}
				proposal={$proposals[timeline.id] ?? null}
				{snapping}
				ondropfiles={(files, atMs) => {
					// One after another from where they were dropped, the way a row of
					// files from a folder would be laid down.
					void (async () => {
						for (const file of files) await upload(file, atMs);
					})();
				}}
			/>
		{:else}
			<p class="note empty">Make a timeline on the left to write scares, beds and songs against a position.</p>
		{/if}
	</section>

	{#if docked && timeline}
		<aside class="dock">
			{#if $subjectRows.length > 0}
				<InspectorPanel />
			{:else}
				<div class="own">
					<span class="kind">Timeline</span>
					<h3>{timeline.name}</h3>
					<TimelineInspector rows={[timeline]} />
				</div>
			{/if}
		</aside>
	{/if}
</div>

<style>
	.timeline-panel {
		display: grid;
		grid-template-columns: 200px minmax(0, 1fr) auto;
		gap: 12px;
		height: 100%;
		min-height: 0;
		padding: 12px;
		box-sizing: border-box;
	}
	.timeline-panel.narrow {
		grid-template-columns: minmax(0, 1fr);
		grid-template-rows: auto minmax(0, 1fr);
	}
	.list {
		overflow-y: auto;
		min-height: 0;
	}
	.main {
		display: flex;
		flex-direction: column;
		gap: 8px;
		min-width: 0;
		min-height: 0;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.bar :global(.transport) {
		flex: 1;
	}
	.actions {
		display: flex;
		gap: 6px;
	}
	.file {
		position: relative;
		overflow: hidden;
	}
	.file input {
		position: absolute;
		inset: 0;
		opacity: 0;
		cursor: pointer;
	}
	.recording-strip {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		padding: 6px 8px;
		border: 1px solid color-mix(in srgb, var(--bad) 50%, transparent);
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--bad) 8%, transparent);
		font-size: var(--font-xs);
	}
	.rec {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		color: var(--bad);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.rec .dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--bad);
	}
	.param {
		padding: 2px 8px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		color: var(--text-dim);
		white-space: nowrap;
	}
	.param b {
		color: var(--text);
		font-weight: 500;
		margin-left: 4px;
		font-variant-numeric: tabular-nums;
	}
	.dim {
		color: var(--text-faint);
	}
	.empty {
		padding: 24px 0;
		font-size: var(--font-sm);
	}
	.dock {
		width: 320px;
		overflow-y: auto;
		overflow-x: hidden;
		min-height: 0;
		border-left: 1px solid var(--line);
		padding-left: 12px;
	}
	.dock :global(.inspector) {
		padding: 0;
	}
	.own {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding-left: 10px;
		border-left: 3px solid var(--cat-playback);
	}
	.own .kind {
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--cat-playback);
		font-weight: 600;
	}
	.own h3 {
		font-size: var(--font-md);
		font-weight: 600;
		margin: 0;
	}
</style>
