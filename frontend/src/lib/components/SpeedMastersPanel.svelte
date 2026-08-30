<script lang="ts">
	/**
	 * The tempos effects follow.
	 *
	 * A master is a number several effects are locked to, and the way an operator sets
	 * one is by tapping along with the band. So Tap is the big control and everything
	 * else is small.
	 *
	 * Tap and Run/Stop stay live when the panel is locked. Both are things done during
	 * a show, at speed, and a lock that stopped an operator following a tempo change
	 * would be a lock nobody left on. What the lock covers is the rest: renaming a
	 * master, changing its multiplier, deleting one.
	 */

	import { focusOnMount } from '$lib/actions.js';
	import type { Fixture, SpeedMaster } from '$lib/generated/index.js';
	import {
		beatPhase,
		bpmFromTaps,
		effectiveHz,
		MULTIPLIERS,
		sinceLastGap,
		tidyBpm
	} from '$lib/speedmasters.js';
	import { collection } from '$lib/stores/show.js';
	import { editing } from '$lib/stores/editing.js';
	import { getDataContext } from '$lib/ws/context.js';

	const data = getDataContext();
	const masters = collection('speed_masters');
	const fixtures = collection('fixtures');
	const unlocked = editing('speedmasters');

	let creating = $state(false);
	let newName = $state('');
	/** Tap times per master, kept here because a half-finished tempo is not show data. */
	let taps = $state<Record<string, number[]>>({});

	/**
	 * A clock the beat dots follow.
	 *
	 * One timer for the panel rather than one per master: a dozen masters would be a
	 * dozen animation frames doing the same arithmetic, and they all read the same
	 * millisecond anyway.
	 */
	let now = $state(Date.now());
	$effect(() => {
		let frame = 0;
		const tick = () => {
			now = Date.now();
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	async function createMaster() {
		const name = newName.trim();
		if (!name) return;
		await data.speed_masters.create({
			id: crypto.randomUUID(),
			name,
			bpm: 120,
			multiplier: 1,
			running: true,
			t0: Date.now()
		});
		newName = '';
		creating = false;
	}

	/**
	 * One tap.
	 *
	 * Writes the tempo *and* the anchor together. That pairing is the whole reason a
	 * tempo change is a bounded step in phase rather than a slide: every station
	 * re-resolves from the new bpm measured from the instant of this tap, so they all
	 * land in the same place rather than each drifting from wherever it happened to be.
	 *
	 * The anchor moves on the first tap too, so the beat starts where the operator's
	 * finger came down rather than wherever the old cycle had got to.
	 */
	async function tap(master: SpeedMaster) {
		const at = Date.now();
		const run = [...(taps[master.id] ?? []), at];
		taps = { ...taps, [master.id]: run };

		const bpm = bpmFromTaps(run);
		const entity = data.speed_masters.byId(master.id);
		if (bpm !== null) await entity.bpm.set(tidyBpm(bpm));
		await entity.t0.set(at);
		if (!master.running) await entity.running.set(true);
	}

	/** Editing the tempo by hand re-anchors it too, for the same reason a tap does. */
	async function setBpm(master: SpeedMaster, bpm: number) {
		if (!(bpm > 0)) return;
		const entity = data.speed_masters.byId(master.id);
		await entity.bpm.set(bpm);
		await entity.t0.set(Date.now());
		taps = { ...taps, [master.id]: [] };
	}

	/**
	 * Starting a stopped master starts its beat here, rather than resuming a cycle
	 * that has notionally been running the whole time it was stopped.
	 */
	async function setRunning(master: SpeedMaster, running: boolean) {
		const entity = data.speed_masters.byId(master.id);
		if (running) await entity.t0.set(Date.now());
		await entity.running.set(running);
	}

	/** The effects following this master, from what each station is rendering. */
	function following(master: SpeedMaster): { fixture: Fixture; key: string }[] {
		const out: { fixture: Fixture; key: string }[] = [];
		for (const fixture of $fixtures) {
			for (const [key, effect] of Object.entries(fixture.live_effects ?? {})) {
				// `live_effects` is already resolved, so the master's id is not in it.
				// What it does carry is the rate the master produced, which is enough
				// to say "these are moving at this tempo" — and if it later stops
				// matching, that is the honest answer too.
				if (effect && Math.abs(effect.rate_hz - effectiveHz(master)) < 1e-3) {
					out.push({ fixture, key });
				}
			}
		}
		return out;
	}
</script>

<div class="masters">
	<header class="head">
		<h2>Speed masters</h2>
		{#if $unlocked}
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Master'}
			</button>
		{/if}
	</header>

	{#if creating && $unlocked}
		<form class="new" onsubmit={(e) => { e.preventDefault(); createMaster(); }}>
			<input class="input" placeholder="Name, e.g. Chases" bind:value={newName} use:focusOnMount />
			<button class="btn btn-primary" type="submit">Create</button>
		</form>
	{/if}

	{#if $masters.length === 0}
		<p class="empty">
			No speed masters. An effect can carry its own rate in hertz; a master is for when
			a whole show's worth of them should move together.
		</p>
	{/if}

	{#each $masters as master (master.id)}
		{@const hz = effectiveHz(master)}
		{@const phase = beatPhase(master, now)}
		{@const on = master.running && phase < 0.5}
		<section class="master" class:stopped={!master.running}>
			<div class="top">
				<!-- The dot is the beat, and it is the first thing to look at: an operator
				     checking a tempo is checking whether this is flashing with the band. -->
				<span class="beat" class:lit={on} aria-hidden="true"></span>
				{#if $unlocked}
					<input
						class="input name"
						value={master.name}
						onchange={(e) => data.speed_masters.byId(master.id).name.set(e.currentTarget.value)}
					/>
				{:else}
					<span class="name">{master.name}</span>
				{/if}
				<span class="hz">{hz.toFixed(2)} Hz</span>
				{#if $unlocked}
					<button
						class="btn btn-danger btn-icon"
						title="Delete {master.name}"
						onclick={() => data.speed_masters.byId(master.id).delete()}
					>×</button>
				{/if}
			</div>

			<div class="controls">
				<!-- Big, because it is tapped in time with music by somebody who is not
				     looking at the screen. -->
				<button class="tap" onclick={() => tap(master)}>
					TAP
					{#if sinceLastGap(taps[master.id] ?? []).length > 1}
						<span class="count">{sinceLastGap(taps[master.id] ?? []).length}</span>
					{/if}
				</button>

				<label class="bpm">
					<input
						class="input"
						type="number"
						min="1"
						max="600"
						step="0.1"
						value={master.bpm}
						onchange={(e) => setBpm(master, Number(e.currentTarget.value))}
					/>
					<span>bpm</span>
				</label>

				<div class="mult" role="group" aria-label="Multiplier">
					{#each MULTIPLIERS as m (m)}
						<button
							class="btn"
							class:on={master.multiplier === m}
							disabled={!$unlocked}
							onclick={() => data.speed_masters.byId(master.id).multiplier.set(m)}
						>{m === 1 ? '×1' : `×${m}`}</button>
					{/each}
				</div>

				<button
					class="btn"
					class:running={master.running}
					onclick={() => setRunning(master, !master.running)}
				>{master.running ? '■ Stop' : '▶ Run'}</button>
			</div>

			{#if following(master).length > 0}
				<div class="riders">
					<span class="riders-label">Moving with this:</span>
					{#each following(master) as rider (rider.fixture.id + rider.key)}
						<span class="rider">{rider.fixture.name} · {rider.key}</span>
					{/each}
				</div>
			{/if}
		</section>
	{/each}
</div>

<style>
	.masters {
		padding: 16px 20px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.head {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}

	.new {
		display: flex;
		gap: 8px;
	}

	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 46ch;
		line-height: 1.5;
	}

	.master {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-panel);
		padding: var(--pad);
		display: flex;
		flex-direction: column;
		gap: var(--pad);
	}

	.master.stopped {
		opacity: 0.7;
	}

	.top {
		display: flex;
		align-items: center;
		gap: var(--pad);
	}

	.beat {
		width: 14px;
		height: 14px;
		flex: none;
		border-radius: 50%;
		border: 1px solid var(--line-input);
		background: transparent;
	}

	.beat.lit {
		background: var(--live);
		border-color: var(--live);
	}

	.name {
		font-weight: 500;
		flex: 1;
		min-width: 0;
	}

	input.name {
		flex: 1;
		min-width: 0;
	}

	.hz {
		color: var(--text-dim);
		font-variant-numeric: tabular-nums;
		font-size: var(--font-sm);
	}

	.controls {
		display: flex;
		align-items: center;
		gap: var(--pad);
		flex-wrap: wrap;
	}

	/* Tapped in time with music, by somebody not looking at the screen. */
	.tap {
		min-width: 96px;
		min-height: var(--hit);
		border: 1px solid var(--live);
		border-radius: var(--radius);
		background: transparent;
		color: var(--live);
		font: inherit;
		font-weight: 600;
		letter-spacing: 0.08em;
		cursor: pointer;
	}
	.tap:active {
		background: var(--live);
		color: var(--bg);
	}
	.count {
		margin-left: 6px;
		font-weight: 400;
		opacity: 0.7;
	}

	.bpm {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--text-dim);
		font-size: var(--font-sm);
	}
	.bpm .input {
		width: 6rem;
		font-variant-numeric: tabular-nums;
	}

	.mult {
		display: flex;
		gap: 2px;
	}
	.mult .btn {
		padding: 0 10px;
		min-width: 44px;
	}
	.mult .btn.on {
		border-color: var(--accent);
		color: var(--accent);
	}

	.btn.running {
		border-color: var(--good);
		color: var(--good);
	}

	.riders {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		border-top: 1px solid var(--line);
		padding-top: 8px;
	}
	.riders-label {
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.rider {
		font-size: var(--font-xs);
		padding: 2px 8px;
		border-radius: 999px;
		border: 1px solid var(--live);
		color: var(--live);
	}
</style>
