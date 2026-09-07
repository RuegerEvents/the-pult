<script lang="ts">
	/**
	 * A position, and what is written against it.
	 *
	 * The transport is four numbers on a SYNCED row — `running`, `anchor_ms`,
	 * `position_at_anchor_ms`, `rate` — so the playhead here is worked out from the
	 * console's clock exactly as the station works it out, and **nothing ticks on
	 * either side**. The readout below is a `requestAnimationFrame` loop over
	 * arithmetic, not a subscription to a moving number.
	 *
	 * Which is why it shows a gap rather than a figure until {@link consoleNow} has an
	 * offset. A page evaluating an anchor against an unadjusted `Date.now()` shows a
	 * playhead that is out by however wrong this machine's clock is, silently — the
	 * same failure `ws/clock.ts` exists to remove, and the same answer.
	 *
	 * Every transport act carries `at`, the way a Go does: every station runs the same
	 * command from the same arguments and so anchors the same millisecond.
	 *
	 * **The waveform is drawn from what the station handed over**, never decoded here:
	 * `Timeline::peaks` is a small asset the playing station reduced once, and a tablet
	 * that decoded a fifty-megabyte file to draw four hundred columns would be doing a
	 * console's work to produce a thumbnail.
	 *
	 * **And the detector proposes; this panel confirms.** *Detect beats* draws its
	 * answer over the waveform in its own colour and changes nothing; *Use this grid*
	 * is the write. A console that silently rewrote the grid of a show somebody had
	 * already programmed against would be worse than one that could not detect
	 * anything.
	 */

	import { onMount } from 'svelte';

	import { focusOnMount } from '$lib/actions.js';
	import type {
		AudioStatus,
		Cue,
		Detected,
		GridSegment,
		InputConfig,
		LtcRate,
		Sequence,
		SpeedMaster,
		Station,
		Timeline,
		TimelineAction,
		TimelineEvent
	} from '$lib/generated/index.js';
	import { selection } from '$lib/stores/selection.js';
	import { clock, parsePosition } from '$lib/waveform.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';
	import { consoleNow } from '$lib/ws/clock.js';
	import Waveform from './Waveform.svelte';

	const client = getClientContext();
	const data = getDataContext();

	let timelines = $state<Timeline[]>([]);
	let sequences = $state<Sequence[]>([]);
	let cues = $state<Cue[]>([]);
	let inputs = $state<InputConfig[]>([]);
	let masters = $state<SpeedMaster[]>([]);
	let stations = $state<Station[]>([]);
	let audioStatus = $state<Record<string, AudioStatus>>({});
	let chosen = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let locateTo = $state('');
	let problem = $state<string | null>(null);
	let busy = $state<string | null>(null);
	/** The detector's answer, held here until somebody accepts or discards it. */
	let proposal = $state<Detected | null>(null);
	let snapping = $state(true);
	let modelState = $state<{ present: boolean; fetching: boolean; bytes: number } | null>(null);
	let uploading = $state(false);

	/** Redrawn every animation frame, which is what makes the readout move. */
	let now = $state<number | null>(null);

	const timeline = $derived(timelines.find((t) => t.id === chosen) ?? timelines[0] ?? null);
	const position = $derived.by(() => {
		if (!timeline || now === null) return null;
		if (!timeline.running) return timeline.position_at_anchor_ms;
		if (now <= timeline.anchor_ms) return timeline.position_at_anchor_ms;
		return timeline.position_at_anchor_ms + (now - timeline.anchor_ms) * timeline.rate;
	});

	// `clock` and `parsePosition` are `$lib/waveform.ts`'s: the waveform beside this
	// panel reads and writes the same positions, and two spellings of "1:04.20" would
	// disagree exactly where somebody typed one and dragged the other.

	const sequenceName = (id: string) => sequences.find((s) => s.id === id)?.name ?? 'a gone sequence';
	const cueName = (id: string) => cues.find((c) => c.id === id)?.name ?? 'a gone cue';
	const cuesOf = (sequenceId: string): Cue[] => {
		const sequence = sequences.find((s) => s.id === sequenceId);
		if (!sequence) return [];
		return sequence.cue_ids.map((id) => cues.find((c) => c.id === id)).filter((c): c is Cue => !!c);
	};

	function describe(action: TimelineAction): string {
		if ('GoToCue' in action) {
			return `${sequenceName(action.GoToCue.sequence_id)} → ${cueName(action.GoToCue.cue_id)}`;
		}
		if ('GoNext' in action) return `${sequenceName(action.GoNext.sequence_id)} → next`;
		return `${sequenceName(action.Off.sequence_id)} off`;
	}

	const sequenceOf = (action: TimelineAction): string =>
		'GoToCue' in action
			? action.GoToCue.sequence_id
			: 'GoNext' in action
				? action.GoNext.sequence_id
				: action.Off.sequence_id;

	// ── Acts ──────────────────────────────────────────────────────────────────

	/**
	 * Every transport act carries the moment it was taken, so each station anchors
	 * the same millisecond rather than whenever its own actor got round to it. A
	 * browser with no clock offset yet is not allowed to name one, and says so.
	 */
	function at(): number | null {
		const stamp = consoleNow();
		if (stamp === null) problem = 'This browser has not worked out the station clock yet.';
		return stamp;
	}

	async function play() {
		const stamp = at();
		if (stamp === null || !timeline) return;
		await data.timelines.byId(timeline.id).play({ at: stamp });
	}

	async function stop() {
		const stamp = at();
		if (stamp === null || !timeline) return;
		await data.timelines.byId(timeline.id).stop({ at: stamp });
	}

	async function locate(positionMs: number) {
		const stamp = at();
		if (stamp === null || !timeline) return;
		await data.timelines.byId(timeline.id).locate({ positionMs, at: stamp });
	}

	async function arm(inputId: string | null) {
		if (!timeline) return;
		await data.timelines.byId(timeline.id).record({ inputId });
	}

	async function createTimeline() {
		const name = newName.trim();
		if (!name) return;
		const id = crypto.randomUUID();
		await data.timelines.create({
			id,
			name,
			audio: null,
			peaks: null,
			detected: null,
			source: 'Internal',
			grid: [],
			markers: [],
			events: [],
			tracks: [],
			speed_master: null,
			// The leader, which is the rule outputs follow. Nothing plays audio yet, so
			// this is only what it will mean.
			node_id: null,
			running: false,
			anchor_ms: 0,
			position_at_anchor_ms: 0,
			rate: 1,
			recording: null
		});
		// Shown at once. Without this the panel goes on showing `timelines[0]`, and the
		// next thing somebody does — attaching a file, say — happens to a different
		// timeline from the one they just made.
		chosen = id;
		newName = '';
		creating = false;
	}

	/** `events` is one column, so every edit rewrites the whole list. */
	async function setEvents(next: TimelineEvent[]) {
		if (!timeline) return;
		next.sort((a, b) => a.at_ms - b.at_ms);
		await data.timelines.byId(timeline.id).events.set(next);
	}

	async function addEvent() {
		if (!timeline || sequences.length === 0) return;
		await setEvents([
			...timeline.events,
			{
				id: crypto.randomUUID(),
				// Where the playhead is, which is where somebody watching a song wants
				// the Go they are adding.
				at_ms: Math.round(position ?? 0),
				action: { GoNext: { sequence_id: sequences[0].id } }
			}
		]);
	}

	async function addMarker() {
		if (!timeline) return;
		await data.timelines.byId(timeline.id).markers.set([
			...timeline.markers,
			{ id: crypto.randomUUID(), at_ms: Math.round(position ?? 0), name: 'Marker' }
		]);
	}

	// ── The sound ─────────────────────────────────────────────────────────────

	/** What this station says about this timeline's audio, if anything. */
	const sound = $derived(timeline ? (audioStatus[timeline.id] ?? null) : null);

	/** Which station plays it, in words. */
	const playedBy = $derived.by(() => {
		if (!timeline) return '';
		if (!timeline.node_id) return 'the leader';
		return stations.find((s) => s.id === timeline.node_id)?.hostname ?? 'a station that is not here';
	});

	/**
	 * Put a file in the show.
	 *
	 * By extension, because that is what a file picker gives — the station accepts the
	 * four kinds a show actually arrives as and refuses everything else by name.
	 */
	async function uploadAudio(file: File) {
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
			problem = `This console plays wav, mp3, flac and m4a. ${file.name} is none of them.`;
			return;
		}
		problem = null;
		uploading = true;
		try {
			const response = await fetch('/assets', {
				method: 'POST',
				headers: { 'content-type': mime },
				body: await file.arrayBuffer()
			});
			if (!response.ok) throw new Error(await response.text());
			const { sha256 } = (await response.json()) as { sha256: string };
			if (!timeline) return;
			// The waveform is the station's to compute, so it is cleared here rather
			// than kept: a new file with the old file's picture is worse than no
			// picture at all.
			await data.timelines.byId(timeline.id).peaks.set(null);
			await data.timelines.byId(timeline.id).detected.set(null);
			await data.timelines.byId(timeline.id).audio.set(sha256);
			proposal = null;
		} catch (e) {
			problem = `That file could not be stored: ${e}`;
		} finally {
			uploading = false;
		}
	}

	/** Find the beats. The station does the work; this waits for the answer. */
	async function detect() {
		if (!timeline) return;
		problem = null;
		busy = 'Listening to the whole song…';
		try {
			proposal = (await client.call('timeline.detect', { timelineId: timeline.id })) as Detected;
			if (proposal.beats_ms.length === 0) {
				problem = 'Nothing in that file reads as a pulse.';
				proposal = null;
			}
		} catch (e) {
			problem = String(e);
		} finally {
			busy = null;
		}
	}

	/**
	 * Turn the proposal into a grid.
	 *
	 * The arithmetic is `pult_audio::grid_from_beats`'s and is deliberately not
	 * repeated here — a segment per tempo change, the bpm from the median inter-beat
	 * interval, `at_ms` on a downbeat — so this asks the station for it by writing what
	 * it was given. The panel's own job is only to decide *when*.
	 */
	async function useProposal() {
		if (!timeline || !proposal) return;
		const segments = gridFrom(proposal);
		if (segments.length === 0) {
			problem = 'There is not enough of a pulse there to make a grid from.';
			return;
		}
		await data.timelines.byId(timeline.id).grid.set(segments);
		proposal = null;
	}

	/**
	 * The proposal as one segment.
	 *
	 * Deliberately simpler than `pult_audio::grid_from_beats`, which splits a song at
	 * every tempo change: what a panel is for here is *showing* the answer, and an
	 * operator who wants the rallentando split out adds a segment, which is two numbers
	 * in a row below.
	 *
	 * What it does **not** simplify is the tempo, because the obvious reading is wrong.
	 * The model works at 50 frames a second, so every beat is quantised to 20 ms — at
	 * 128 bpm a 468.75 ms beat lands on 460 or 480, whose median is 460, and
	 * `60000 / median` is 130.4. Over a minute that walks a whole bar out of step. So
	 * the tempo comes from the **span** divided by the number of beats the span holds,
	 * with each interval asked how many beats it covers so that a beat the detector
	 * missed counts as the two it stands for. The station's own function does exactly
	 * this, for the same reason and in the same words.
	 */
	function gridFrom(found: Detected): GridSegment[] {
		const beats = found.beats_ms;
		if (beats.length < 2) return [];
		const gaps = beats.slice(1).map((at, i) => at - beats[i]);
		const median = [...gaps].sort((a, b) => a - b)[Math.floor(gaps.length / 2)];
		if (!median) return [];
		const span = beats[beats.length - 1] - beats[0];
		const held = gaps.reduce((sum, gap) => sum + Math.max(1, Math.round(gap / median)), 0);
		const bpm = span > 0 ? (60_000 * held) / span : 60_000 / median;
		const first = found.downbeats_ms[0] ?? beats[0];
		const bar =
			found.downbeats_ms.length > 1
				? Math.max(1, Math.round((found.downbeats_ms[1] - found.downbeats_ms[0]) / median))
				: 4;
		return [{ at_ms: first, bpm: Math.round(bpm * 10) / 10, beats_per_bar: bar }];
	}

	/** `grid` is one column, so every edit rewrites the whole list. */
	async function setGrid(next: GridSegment[]) {
		if (!timeline) return;
		next.sort((a, b) => a.at_ms - b.at_ms);
		await data.timelines.byId(timeline.id).grid.set(next);
	}

	async function askAboutTheModel(fetchIt: boolean) {
		try {
			modelState = (await client.call('timeline.model', { full: fetchIt })) as typeof modelState;
		} catch (e) {
			problem = String(e);
		}
	}

	/**
	 * Put what is on the wire into the programmer, for the selection.
	 *
	 * Offered here rather than in the programmer because the input is a timeline's
	 * business: a grab and a recording are the same decoding, taken once and taken
	 * continuously. Refused by name where this station is not the one listening.
	 */
	async function grab() {
		problem = null;
		const fixtureIds = $selection;
		if (fixtureIds.length === 0) {
			problem = 'Nothing is selected.';
			return;
		}
		try {
			const answer = (await client.call('input.grab', { fixtureIds })) as { written: number };
			problem = answer.written === 0 ? 'Nothing on the wire reaches those fixtures.' : null;
		} catch (e) {
			problem = String(e);
		}
	}

	onMount(() => {
		const stops = [
			data.timelines.subscribeDeep((v) => {
				timelines = v;
				if (chosen && !v.some((t) => t.id === chosen)) chosen = null;
			}),
			data.sequences.subscribeDeep((v) => { sequences = v; }),
			data.cues.subscribeDeep((v) => { cues = v; }),
			data.inputs.subscribeDeep((v) => { inputs = v; }),
			data.speed_masters.subscribeDeep((v) => { masters = v; }),
			data.stations.subscribeDeep((v) => { stations = v; })
		];

		// `audio_status` is LOCAL and subscribed by path, the way `output_status` is.
		// This station's alone: it describes a device on this machine, and the station
		// beside it running the same show has its own answer.
		const applyAudio = (v: unknown) => {
			if (v && typeof v === 'object') audioStatus = v as Record<string, AudioStatus>;
		};
		stops.push(client.subscribe('audio_status', applyAudio));
		const fetchLocal = () => {
			void client.get(['audio_status']).then(applyAudio);
		};
		fetchLocal();
		stops.push(client.addConnectListener(fetchLocal));
		void askAboutTheModel(false);

		// The playhead, drawn rather than subscribed to. Nothing on the wire carries a
		// moving position: it is arithmetic over an anchor, so this loop is the only
		// thing that has to happen at all.
		let frame = requestAnimationFrame(function tick() {
			now = consoleNow();
			frame = requestAnimationFrame(tick);
		});

		return () => {
			cancelAnimationFrame(frame);
			for (const stop of stops) stop();
		};
	});
</script>

<div class="timelines">
	<header class="block-head">
		<h2>Timelines</h2>
		<div class="head-controls">
			{#if timelines.length > 1}
				<select class="text-input" value={timeline?.id ?? ''} onchange={(e) => (chosen = e.currentTarget.value)}>
					{#each timelines as t (t.id)}
						<option value={t.id}>{t.name}</option>
					{/each}
				</select>
			{/if}
			<button class="ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Timeline'}
			</button>
		</div>
	</header>

	{#if creating}
		<form class="new-row" onsubmit={(e) => { e.preventDefault(); createTimeline(); }}>
			<input class="text-input" placeholder="What is it for?" bind:value={newName} use:focusOnMount />
			<button class="primary" type="submit">Add</button>
		</form>
	{/if}

	{#if !timeline}
		<p class="empty">
			No timelines. One is a position and a list of Gos written against it — a song, an act,
			or anything else that has to happen at a time rather than on a button.
		</p>
	{:else}
		<section class="block transport">
			<input
				class="text-input name"
				value={timeline.name}
				onchange={(e) => data.timelines.byId(timeline.id).name.set(e.currentTarget.value)}
			/>
			<span class="readout" class:rolling={timeline.running}>
				{#if position === null}
					<span class="gap" title="This browser has not worked out the station's clock yet">—:—.—</span>
				{:else}
					{clock(position)}
				{/if}
			</span>
			{#if timeline.running}
				<button class="primary" onclick={stop}>Stop</button>
			{:else}
				<button class="primary" onclick={play}>Play</button>
			{/if}
			<button class="ghost" onclick={() => locate(0)}>To top</button>
			<form class="locate" onsubmit={(e) => {
				e.preventDefault();
				const ms = parsePosition(locateTo);
				if (ms !== null) locate(ms);
				locateTo = '';
			}}>
				<input class="text-input narrow" placeholder="1:04" bind:value={locateTo} />
				<button class="ghost" type="submit">Locate</button>
			</form>
			<button
				class="danger"
				title="Delete timeline"
				onclick={() => data.timelines.byId(timeline.id).delete()}>×</button
			>
		</section>

		{#if problem}<p class="problem">{problem}</p>{/if}
		{#if busy}<p class="hint">{busy}</p>{/if}

		<section class="block">
			<Waveform {timeline} {proposal} {snapping} onLocate={locate} />
		</section>

		<section class="block">
			<header class="block-head">
				<h3>Sound</h3>
				<div class="head-controls">
					<label class="checkbox">
						<input type="checkbox" bind:checked={snapping} />
						snap to the grid
					</label>
				</div>
			</header>

			<div class="rows-grid">
				<span class="label">Audio file</span>
				<div class="row">
					{#if timeline.audio}
						<span class="what">{timeline.audio.slice(0, 8)}…</span>
						<button
							class="ghost"
							onclick={async () => {
								await data.timelines.byId(timeline.id).audio.set(null);
								await data.timelines.byId(timeline.id).peaks.set(null);
								await data.timelines.byId(timeline.id).detected.set(null);
								proposal = null;
							}}>Remove</button
						>
					{:else}
						<span class="what">none</span>
					{/if}
					<label class="ghost file">
						{uploading ? 'Storing…' : timeline.audio ? 'Replace…' : 'Add a file…'}
						<input
							type="file"
							accept=".wav,.mp3,.flac,.m4a,.mp4,.aac,audio/*"
							disabled={uploading}
							onchange={(e) => {
								const file = e.currentTarget.files?.[0];
								e.currentTarget.value = '';
								if (file) uploadAudio(file);
							}}
						/>
					</label>
				</div>

				<span class="label">Played by</span>
				<div class="row">
					<select
						class="text-input"
						value={timeline.node_id ?? ''}
						onchange={(e) =>
							data.timelines.byId(timeline.id).node_id.set(e.currentTarget.value || null)}
					>
						<option value="">the leader</option>
						{#each stations as station (station.id)}
							<option value={station.id}>{station.hostname}</option>
						{/each}
					</select>
					{#if sound}
						{#if sound.playing}
							<span class="good">playing through {sound.device ?? 'a device'}</span>
							<span class="what">{sound.drift_ms} ms from the anchor</span>
						{:else if sound.fault}
							<span class="bad">{sound.fault}</span>
						{/if}
					{:else if timeline.audio}
						<span class="what">nothing on this station is playing it</span>
					{/if}
				</div>

				<span class="label">Position from</span>
				<div class="row">
					<select
						class="text-input"
						value={typeof timeline.source === 'string' ? 'Internal' : 'Ltc'}
						onchange={(e) =>
							data.timelines
								.byId(timeline.id)
								.source.set(
									e.currentTarget.value === 'Ltc'
										? { Ltc: { fps: 'F25', offset_frames: 0 } }
										: 'Internal'
								)}
					>
						<option value="Internal">this console</option>
						<option value="Ltc">timecode (LTC)</option>
					</select>
					{#if typeof timeline.source !== 'string'}
						<select
							class="text-input"
							value={timeline.source.Ltc.fps}
							onchange={(e) =>
								data.timelines.byId(timeline.id).source.set({
									Ltc: {
										fps: e.currentTarget.value as LtcRate,
										offset_frames: (timeline.source as { Ltc: { offset_frames: number } }).Ltc
											.offset_frames
									}
								})}
						>
							<option value="F24">24 fps</option>
							<option value="F25">25 fps</option>
							<option value="F30">30 fps</option>
							<option value="F2997">29.97 non-drop</option>
							<option value="F2997Df">29.97 drop-frame</option>
						</select>
						<input
							class="text-input narrow"
							title="Offset in frames, subtracted from what arrives"
							value={timeline.source.Ltc.offset_frames}
							onchange={(e) => {
								const frames = Number(e.currentTarget.value);
								if (!Number.isFinite(frames)) return;
								data.timelines.byId(timeline.id).source.set({
									Ltc: {
										fps: (timeline.source as { Ltc: { fps: LtcRate } }).Ltc.fps,
										offset_frames: Math.round(frames)
									}
								});
							}}
						/>
						<span class="what">frames offset</span>
					{/if}
				</div>

				{#if sound?.ltc}
					<span class="label">Timecode</span>
					<div class="row">
						{#if sound.ltc.lock === 'Locked'}
							<span class="good">locked</span>
						{:else if sound.ltc.lock === 'Lost'}
							<span class="bad">LTC lost</span>
						{:else}
							<span class="what">waiting for timecode</span>
						{/if}
						{#if sound.ltc.position_ms !== null && sound.ltc.position_ms !== undefined}
							<span class="what">{clock(sound.ltc.position_ms)}</span>
						{/if}
						{#if sound.ltc.device}<span class="what">{sound.ltc.device}</span>{/if}
						{#if sound.ltc.fault}<span class="bad">{sound.ltc.fault}</span>{/if}
					</div>
				{/if}

				<span class="label">Speed master</span>
				<div class="row">
					<select
						class="text-input"
						value={timeline.speed_master ?? ''}
						onchange={(e) =>
							data.timelines.byId(timeline.id).speed_master.set(e.currentTarget.value || null)}
					>
						<option value="">drives none</option>
						{#each masters as master (master.id)}
							<option value={master.id}>{master.name}</option>
						{/each}
					</select>
					<span class="what">
						While this runs, the leader sets that master's tempo at each grid segment.
					</span>
				</div>
			</div>
		</section>

		<section class="block">
			<header class="block-head">
				<h3>Beat grid</h3>
				<div class="head-controls">
					{#if proposal}
						<span class="what">
							{proposal.beats_ms.length} beats, {proposal.downbeats_ms.length} bars
						</span>
						<button class="primary" onclick={useProposal}>Use this grid</button>
						<button class="ghost" onclick={() => (proposal = null)}>Discard</button>
					{:else}
						<button class="ghost" onclick={detect} disabled={!timeline.audio || busy !== null}>
							Detect beats
						</button>
					{/if}
					<button
						class="ghost"
						onclick={() =>
							setGrid([
								...timeline.grid,
								{ at_ms: Math.round(position ?? 0), bpm: 120, beats_per_bar: 4 }
							])}>+ Segment</button
					>
				</div>
			</header>

			{#if timeline.grid.length === 0}
				<p class="empty">
					No grid. One is a tempo from a position — a segment per tempo change, because a song has
					a rallentando in it and a show has an interval.
				</p>
			{:else}
				<table class="rows">
					<tbody>
						{#each timeline.grid as segment, i (i)}
							<tr>
								<td>
									<input
										class="text-input narrow"
										value={clock(segment.at_ms)}
										onchange={(e) => {
											const ms = parsePosition(e.currentTarget.value);
											if (ms === null) return;
											setGrid(
												timeline.grid.map((each, at) => (at === i ? { ...each, at_ms: ms } : each))
											);
										}}
									/>
								</td>
								<td>
									<input
										class="text-input narrow"
										value={Math.round(segment.bpm * 100) / 100}
										onchange={(e) => {
											const bpm = Number(e.currentTarget.value);
											if (!Number.isFinite(bpm) || bpm <= 0) return;
											setGrid(timeline.grid.map((each, at) => (at === i ? { ...each, bpm } : each)));
										}}
									/>
								</td>
								<td class="what">bpm</td>
								<td>
									<input
										class="text-input narrow"
										value={segment.beats_per_bar}
										onchange={(e) => {
											const beats = Number(e.currentTarget.value);
											if (!Number.isFinite(beats) || beats < 1) return;
											setGrid(
												timeline.grid.map((each, at) =>
													at === i ? { ...each, beats_per_bar: Math.round(beats) } : each
												)
											);
										}}
									/>
								</td>
								<td class="what">beats to the bar</td>
								<td>
									<button
										class="danger"
										title="Remove"
										onclick={() => setGrid(timeline.grid.filter((_, at) => at !== i))}>×</button
									>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}

			<p class="note">
				{#if modelState?.present}
					Detecting with the full-accuracy model on this station.
				{:else if modelState?.fetching}
					The full-accuracy model is downloading. Detection uses the small one until it lands.
				{:else}
					Detecting with the small model this console ships with.
					<button class="link" onclick={() => askAboutTheModel(true)}>
						Fetch the full one (83 MB)
					</button>
				{/if}
			</p>
		</section>

		<section class="block">
			<header class="block-head">
				<h3>Events</h3>
				<button class="ghost" onclick={addEvent} disabled={sequences.length === 0}>+ Event</button>
			</header>
			{#if sequences.length === 0}
				<p class="empty">There are no sequences to Go. A timeline says <em>when</em>; a sequence is the stack.</p>
			{:else if timeline.events.length === 0}
				<p class="empty">Nothing happens on this timeline yet.</p>
			{:else}
				<table class="rows">
					<tbody>
						{#each timeline.events as event (event.id)}
							<tr>
								<td>
									<input
										class="text-input narrow"
										value={clock(event.at_ms)}
										onchange={(e) => {
											const ms = parsePosition(e.currentTarget.value);
											if (ms === null) return;
											setEvents(
												timeline.events.map((each) =>
													each.id === event.id ? { ...each, at_ms: ms } : each
												)
											);
										}}
									/>
								</td>
								<td>
									<select
										class="text-input"
										value={sequenceOf(event.action)}
										onchange={(e) => {
											const sequence_id = e.currentTarget.value;
											setEvents(
												timeline.events.map((each) =>
													each.id === event.id ? { ...each, action: { GoNext: { sequence_id } } } : each
												)
											);
										}}
									>
										{#each sequences as sequence (sequence.id)}
											<option value={sequence.id}>{sequence.name}</option>
										{/each}
									</select>
								</td>
								<td>
									<select
										class="text-input"
										value={'GoToCue' in event.action ? event.action.GoToCue.cue_id : 'GoNext' in event.action ? '' : 'off'}
										onchange={(e) => {
											const sequence_id = sequenceOf(event.action);
											const picked = e.currentTarget.value;
											const action: TimelineAction =
												picked === ''
													? { GoNext: { sequence_id } }
													: picked === 'off'
														? { Off: { sequence_id } }
														: { GoToCue: { sequence_id, cue_id: picked } };
											setEvents(
												timeline.events.map((each) =>
													each.id === event.id ? { ...each, action } : each
												)
											);
										}}
									>
										<option value="">Go next</option>
										<option value="off">Off</option>
										{#each cuesOf(sequenceOf(event.action)) as cue (cue.id)}
											<option value={cue.id}>{cue.name}</option>
										{/each}
									</select>
								</td>
								<td class="what">{describe(event.action)}</td>
								<td>
									<button
										class="danger"
										title="Remove"
										onclick={() => setEvents(timeline.events.filter((each) => each.id !== event.id))}
										>×</button
									>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section class="block">
			<header class="block-head">
				<h3>Markers</h3>
				<button class="ghost" onclick={addMarker}>+ Marker</button>
			</header>
			{#if timeline.markers.length === 0}
				<p class="empty">No markers.</p>
			{:else}
				<table class="rows">
					<tbody>
						{#each timeline.markers as marker (marker.id)}
							<tr>
								<td>
									<input
										class="text-input narrow"
										value={clock(marker.at_ms)}
										onchange={(e) => {
											const ms = parsePosition(e.currentTarget.value);
											if (ms === null) return;
											data.timelines.byId(timeline.id).markers.set(
												timeline.markers.map((each) =>
													each.id === marker.id ? { ...each, at_ms: ms } : each
												)
											);
										}}
									/>
								</td>
								<td>
									<input
										class="text-input"
										value={marker.name}
										onchange={(e) =>
											data.timelines.byId(timeline.id).markers.set(
												timeline.markers.map((each) =>
													each.id === marker.id
														? { ...each, name: e.currentTarget.value }
														: each
												)
											)}
									/>
								</td>
								<td>
									<button class="ghost" onclick={() => locate(marker.at_ms)}>Go to</button>
								</td>
								<td>
									<button
										class="danger"
										title="Remove"
										onclick={() =>
											data.timelines
												.byId(timeline.id)
												.markers.set(timeline.markers.filter((each) => each.id !== marker.id))}
										>×</button
									>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</section>

		<section class="block">
			<header class="block-head">
				<h3>Recordings</h3>
				<div class="head-controls">
					<select
						class="text-input"
						value={timeline.recording ?? ''}
						onchange={(e) => arm(e.currentTarget.value || null)}
					>
						<option value="">not recording</option>
						{#each inputs.filter((i) => i.enabled) as input (input.id)}
							<option value={input.id}>record from {input.name}</option>
						{/each}
					</select>
					<button class="ghost" onclick={grab} title="Decode what is on the wire now into the programmer">
						Grab selection
					</button>
				</div>
			</header>

			{#if timeline.recording}
				<p class="hint">
					Armed. The station holding that input records from the next Play to the next Stop,
					and writes the take here.
				</p>
			{/if}

			{#if timeline.tracks.length === 0}
				<p class="empty">No takes yet.</p>
			{:else}
				<table class="rows">
					<tbody>
						{#each timeline.tracks as track (track.id)}
							<tr>
								<td>
									<input
										class="text-input"
										value={track.name}
										onchange={(e) =>
											data.timelines.byId(timeline.id).tracks.set(
												timeline.tracks.map((each) =>
													each.id === track.id ? { ...each, name: e.currentTarget.value } : each
												)
											)}
									/>
								</td>
								<td>
									<input
										type="checkbox"
										title="Play this take with the timeline"
										checked={track.enabled}
										onchange={(e) =>
											data.timelines.byId(timeline.id).tracks.set(
												timeline.tracks.map((each) =>
													each.id === track.id
														? { ...each, enabled: e.currentTarget.checked }
														: each
												)
											)}
									/>
								</td>
								<td class="what">{track.asset.slice(0, 8)}…</td>
								<td>
									<button
										class="danger"
										title="Remove"
										onclick={() =>
											data.timelines
												.byId(timeline.id)
												.tracks.set(timeline.tracks.filter((each) => each.id !== track.id))}
										>×</button
									>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
			<p class="note">
				A take holds every key it recorded over playback and under the programmer. Stopping the
				timeline lets those keys go, over the show's home fade, into whatever playback holds
				beneath them.
			</p>
		</section>
	{/if}
</div>

<style>
	.timelines { padding: 16px 20px; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
	.head-controls { display: flex; align-items: center; gap: 6px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	h3 { font-size: 12px; font-weight: 600; color: #ccc; margin: 0; }
	.transport { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
	.locate { display: flex; gap: 4px; }
	.readout { font-variant-numeric: tabular-nums; font-size: 20px; color: #bbb; min-width: 96px; }
	.readout.rolling { color: #4ade80; }
	.gap { color: #777; }
	.rows { width: 100%; border-collapse: collapse; font-size: 13px; }
	.rows td { padding: 3px 6px 3px 0; vertical-align: middle; }
	.what { color: #888; font-size: 12px; }
	.hint { color: #777; font-size: 12px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.problem { color: #e05555; font-size: 12px; margin: 0 0 8px; }
	.note { color: #666; font-size: 12px; margin-top: 10px; font-style: italic; }
	.new-row { display: flex; gap: 6px; margin-bottom: 12px; }
	.rows-grid { display: grid; grid-template-columns: max-content 1fr; gap: 6px 12px; align-items: center; font-size: 13px; }
	.label { color: #888; font-size: 12px; }
	.row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
	.checkbox { display: flex; align-items: center; gap: 4px; color: #999; font-size: 12px; }
	.good { color: #4ade80; font-size: 12px; }
	.bad { color: #e05555; font-size: 12px; }
	.file { position: relative; overflow: hidden; display: inline-block; }
	.file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
	.link { background: none; border: none; color: #2f6fd0; font: inherit; padding: 0; cursor: pointer; text-decoration: underline; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.text-input.narrow { width: 84px; }
	.text-input.name { width: 160px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost:hover { border-color: #555; color: #fff; }
	.ghost:disabled { opacity: 0.4; cursor: default; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
