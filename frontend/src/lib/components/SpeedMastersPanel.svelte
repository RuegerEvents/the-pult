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
	import { canEdit } from '$lib/stores/showLock.js';
	import { getDataContext } from '$lib/ws/context.js';
	import NameDialog from './NameDialog.svelte';
	import { uuid } from '$lib/uuid.js';
	import { consoleNow } from '$lib/ws/clock.js';

	/**
	 * The show clock, never this machine's own: a `t0` is an anchor every station
	 * resolves the beat from, and a laptop 600 ms out stamped it 600 ms out (HHG).
	 */
	const showNow = () => consoleNow() ?? Date.now();

	const data = getDataContext();
	const masters = collection('speed_masters');
	const fixtures = collection('fixtures');

	let creating = $state(false);
	/** Tap times per master, kept here because a half-finished tempo is not show data. */
	let taps = $state<Record<string, number[]>>({});

	/**
	 * A clock the beat dots follow.
	 *
	 * One timer for the panel rather than one per master: a dozen masters would be a
	 * dozen animation frames doing the same arithmetic, and they all read the same
	 * millisecond anyway.
	 */
	let now = $state(showNow());
	$effect(() => {
		let frame = 0;
		const tick = () => {
			now = showNow();
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	async function createMaster(name: string) {
		await data.speed_masters.create({
			id: uuid(),
			name,
			bpm: 120,
			multiplier: 1,
			running: true,
			t0: showNow()
		});
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
		const at = showNow();
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
		await entity.t0.set(showNow());
		taps = { ...taps, [master.id]: [] };
	}

	/**
	 * Starting a stopped master starts its beat here, rather than resuming a cycle
	 * that has notionally been running the whole time it was stopped.
	 */
	async function setRunning(master: SpeedMaster, running: boolean) {
		const entity = data.speed_masters.byId(master.id);
		if (running) await entity.t0.set(showNow());
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
		{#if $canEdit}
			<button class="btn btn-ghost" onclick={() => (creating = true)}>+ Master</button>
		{/if}
	</header>

	{#if creating}
		<NameDialog
			title="New speed master"
			placeholder="Name, e.g. Chases"
			onsubmit={createMaster}
			onclose={() => (creating = false)}
		/>
	{/if}

	{#if $masters.length === 0}
		<p class="empty">
			No speed masters. An effect can carry its own rate in hertz; a master is for when
			a whole show's worth of them should move together.
		</p>
	{/if}

	{#if $masters.length > 0}
		<!-- A table, a master a row: the beat first, because an operator checking a
		     tempo is checking whether this is flashing with the band. -->
		<table class="rack">
			<thead>
				<tr>
					<th></th><th>Master</th><th>BPM</th><th>Hz</th><th>Multiplier</th><th>Tap</th><th></th>
					<th>Riders</th><th></th>
				</tr>
			</thead>
			<tbody>
				{#each $masters as master (master.id)}
					{@const hz = effectiveHz(master)}
					{@const phase = beatPhase(master, now)}
					{@const on = master.running && phase < 0.5}
					{@const riders = following(master)}
					<tr class:stopped={!master.running}>
						<td><span class="beat" class:lit={on} aria-hidden="true"></span></td>
						<td class="name-cell">
							{#if $canEdit}
								<input
									class="input name"
									value={master.name}
									onchange={(e) => data.speed_masters.byId(master.id).name.set(e.currentTarget.value)}
								/>
							{:else}
								<span class="name">{master.name}</span>
							{/if}
						</td>
						<td>
							<!-- Rounded for display only. `bpm` is an f32 in the schema, so 56.1
							     comes back as 56.099998474121094, and a number input shows every
							     digit of it — which reads as a console that cannot count. -->
							<input
								class="input bpm"
								type="number"
								min="1"
								max="600"
								step="0.1"
								aria-label="{master.name} BPM"
								value={tidyBpm(master.bpm)}
								onchange={(e) => setBpm(master, Number(e.currentTarget.value))}
							/>
						</td>
						<td class="hz">{hz.toFixed(2)}</td>
						<td>
							<div class="mult" role="group" aria-label="Multiplier">
								{#each MULTIPLIERS as m (m)}
									<button
										class:on={master.multiplier === m}
										disabled={!$canEdit}
										onclick={() => data.speed_masters.byId(master.id).multiplier.set(m)}
									>×{m}</button>
								{/each}
							</div>
						</td>
						<td>
							<!-- Big enough to hit in time with music without looking. -->
							<button class="tap" onclick={() => tap(master)}>
								TAP
								{#if sinceLastGap(taps[master.id] ?? []).length > 1}
									<span class="count">{sinceLastGap(taps[master.id] ?? []).length}</span>
								{/if}
							</button>
						</td>
						<td>
							<button
								class="run"
								class:running={master.running}
								onclick={() => setRunning(master, !master.running)}
							>{master.running ? '■ Stop' : '▶ Run'}</button>
						</td>
						<td class="riders" title={riders.map((r) => `${r.fixture.name} · ${r.key}`).join('\n')}>
							{riders.length === 0 ? '—' : riders.length}
						</td>
						<td class="acts">
							{#if $canEdit}
								<button
									class="delete"
									title="Delete {master.name}"
									aria-label="Delete {master.name}"
									onclick={() => data.speed_masters.byId(master.id).delete()}
								>✕</button>
							{/if}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
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


	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 46ch;
		line-height: 1.5;
	}

	.rack {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	.rack th {
		text-align: left;
		font-weight: 500;
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint);
		padding: 0 10px 6px 0;
	}
	.rack td {
		padding: 6px 10px 6px 0;
		border-top: 1px solid var(--line);
		vertical-align: middle;
	}
	.rack tr.stopped td {
		color: var(--text-dim);
	}

	.beat {
		display: block;
		width: 14px;
		height: 14px;
		border-radius: 50%;
		border: 1px solid var(--line-input);
	}
	.beat.lit {
		background: var(--live);
		border-color: var(--live);
	}

	.name-cell {
		min-width: 8rem;
	}
	.name {
		font-weight: 500;
	}
	input.name {
		width: 100%;
	}
	.bpm {
		width: 5.5rem;
		font-variant-numeric: tabular-nums;
	}
	.hz,
	.riders {
		color: var(--text-dim);
		font-variant-numeric: tabular-nums;
	}

	/* Either of four, as one control. */
	.mult {
		display: inline-flex;
		border: 1px solid var(--line-strong);
		border-radius: 6px;
		overflow: hidden;
	}
	.mult button {
		padding: 4px 9px;
		border: none;
		border-right: 1px solid var(--line-strong);
		background: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.mult button:last-child {
		border-right: none;
	}
	.mult button.on {
		background: color-mix(in srgb, var(--accent) 22%, transparent);
		color: var(--text-bright);
	}

	/* Tapped in time with music, by somebody not looking at the screen. */
	.tap {
		min-width: 72px;
		min-height: 32px;
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

	.run {
		min-height: 28px;
		padding: 0 10px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		background: none;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		white-space: nowrap;
		cursor: pointer;
	}
	.run.running {
		border-color: var(--good);
		color: var(--good);
	}

	.acts {
		text-align: right;
	}
	.delete {
		border: none;
		background: none;
		color: var(--text-faint);
		cursor: pointer;
		font-size: var(--font-xs);
	}
	.delete:hover {
		color: var(--bad);
	}
</style>
