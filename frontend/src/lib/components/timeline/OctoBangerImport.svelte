<script lang="ts">
	/**
	 * An OctoBanger prop, moved onto OpenHaunt nodes.
	 *
	 * Two requests, because somebody has to say where each channel goes before it can go
	 * anywhere: the station reads the upload and describes it, the operator answers, and
	 * the same bytes go back with the answer. Nothing waits on the station in between, so
	 * closing this half way leaves nothing behind.
	 *
	 * An archive can hold several props, and they are imported one at a time — each is
	 * a timeline, a flow and its own mapping, and the outputs one prop takes are the ones
	 * the next must not. What the prop needed that a node does not (which Arduino pins,
	 * an active-low relay board) is shown beside the channel as a note, never carried.
	 */

	import type { AudioDestination, Fixture } from '$lib/generated/index.js';
	import { collection } from '$lib/stores/show.js';
	import { userId } from '$lib/stores/user.js';
	import { zones } from '$lib/stores/zones.js';
	import { getClientContext } from '$lib/ws/context.js';
	import { clock } from '$lib/waveform.js';
	import {
		clashes,
		defaultEdge,
		fillFrom,
		portId,
		switchOutputs,
		triggerInputs,
		triggerText,
		type Described
	} from '$lib/octobanger.js';
	import Dialog from '../Dialog.svelte';

	let { onclose }: { onclose: () => void } = $props();

	const client = getClientContext();
	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	type Report = { created: number; updated: number; missing: string[]; warnings: string[] };

	/** One file the operator picked, and what the station said is in it. */
	type Upload = { file: File; controllers: Described[] };

	let uploads = $state<Upload[]>([]);
	let reading = $state(false);
	let failed = $state<string | null>(null);
	let fileInput = $state<HTMLInputElement | null>(null);

	/** Which prop is open, as `upload index / controller path`. */
	let current = $state<string | null>(null);
	/** What each prop came to, by the same key. */
	let done = $state<Record<string, Report>>({});

	// The answer for the open prop.
	let assigned = $state<Record<number, string | null>>({});
	let trigger = $state<string>('');
	let edge = $state<'RisingEdge' | 'FallingEdge'>('RisingEdge');
	let destination = $state<string>('');
	let ambience = $state(false);
	let supplied = $state<Record<string, File>>({});
	let importing = $state(false);

	const entries = $derived(
		uploads.flatMap((u, index) => u.controllers.map((c) => ({ key: `${index}/${c.path}`, upload: u, described: c })))
	);
	const open = $derived(entries.find((e) => e.key === current) ?? null);
	const outputs = $derived(switchOutputs($fixtures, $types));
	const inputs = $derived(triggerInputs($fixtures, $types));
	const nodes = $derived(
		$fixtures.filter(
			(f: Fixture) => 'OpenHaunt' in f.address && !f.address.OpenHaunt.head && !f.address.OpenHaunt.strip
		)
	);
	/** Fixtures with an output a channel can take, for "fill from". */
	const fillable = $derived([...new Map(outputs.map((o) => [o.fixtureId, o.fixture])).entries()]);
	const doubled = $derived(clashes(assigned));

	async function read(files: FileList) {
		reading = true;
		failed = null;
		try {
			for (const file of Array.from(files)) {
				const response = await fetch(client.httpUrl('/api/import/obc/read'), { method: 'POST', body: file });
				if (!response.ok) {
					failed = `${file.name}: ${(await response.text()) || `the console answered ${response.status}`}`;
					continue;
				}
				const { controllers } = (await response.json()) as { controllers: Described[] };
				uploads = [...uploads, { file, controllers }];
			}
			const first = entries.find((e) => !done[e.key]);
			if (first && !current) choose(first.key);
		} catch (error) {
			failed = String(error);
		} finally {
			reading = false;
		}
	}

	/** Open a prop, with the answer it starts from. */
	function choose(key: string) {
		current = key;
		const entry = entries.find((e) => e.key === key);
		if (!entry) return;
		const d = entry.described;
		assigned = Object.fromEntries(d.channels.map((c) => [c.number, null]));
		trigger = '';
		edge = defaultEdge(d.controller.trigger);
		destination = $zones[0] ? `zone:${$zones[0].id}` : '';
		const ambient = d.sounds.find((s) => s.role === 'ambient');
		// A loop the upload carried and that has something in it. A silent one is a file
		// made to fill the MP3 board's slot on a prop with no ambience.
		ambience = !!ambient?.found && ambient.silent === false;
		supplied = {};
	}

	function fill(fixtureId: string) {
		if (!open) return;
		const used = open.described.channels.filter((c) => c.used).map((c) => c.number);
		assigned = fillFrom(assigned, used, outputs, fixtureId);
	}

	function destinationOf(value: string): AudioDestination | null {
		if (value.startsWith('zone:')) return { Zone: { zone: value.slice(5) } };
		if (value.startsWith('node:')) return { Node: { serial: value.slice(5), mode: 'Stream' } };
		return null;
	}

	/** A file the upload did not carry, into the asset store, by the name the prop uses. */
	async function store(file: File): Promise<string> {
		const response = await fetch(client.httpUrl('/assets'), {
			method: 'POST',
			headers: { 'content-type': file.type || 'audio/mpeg', 'x-pult-user': $userId },
			body: file
		});
		if (!response.ok) throw new Error(`${file.name}: ${(await response.text()) || response.status}`);
		return ((await response.json()) as { sha256: string }).sha256;
	}

	async function importOpen() {
		if (!open || importing || doubled.size > 0) return;
		importing = true;
		failed = null;
		try {
			const audio: Record<string, string> = {};
			for (const [name, file] of Object.entries(supplied)) audio[name] = await store(file);
			const mapping = {
				controller: open.described.path,
				channels: Object.entries(assigned).map(([channel, port]) => {
					const [fixtureId, key] = port ? port.split('|') : [null, null];
					return { channel: Number(channel), fixtureId, key };
				}),
				trigger: trigger ? { fixtureId: trigger.split('|')[0], key: trigger.split('|')[1] } : null,
				edge,
				destination: destinationOf(destination),
				ambience,
				audio
			};
			const url = client.httpUrl(`/api/import/obc?mapping=${encodeURIComponent(JSON.stringify(mapping))}`);
			const response = await fetch(url, { method: 'POST', headers: { 'x-pult-user': $userId }, body: open.upload.file });
			if (!response.ok) {
				failed = (await response.text()) || `the console answered ${response.status}`;
				return;
			}
			done = { ...done, [open.key]: await response.json() };
			const next = entries.find((e) => !done[e.key]);
			if (next) choose(next.key);
		} catch (error) {
			failed = String(error);
		} finally {
			importing = false;
		}
	}

	function soundState(s: Described['sounds'][number]): string {
		if (supplied[s.name]) return `will be uploaded (${supplied[s.name].name})`;
		if (!s.found) return 'not in the upload';
		if (s.error) return s.error;
		return `${clock(s.durationMs ?? 0)}${s.silent ? ' · silent' : ''}`;
	}
</script>

<input
	type="file"
	accept=".obc,.OBC,.zip"
	multiple
	bind:this={fileInput}
	onchange={(e) => {
		const files = e.currentTarget.files;
		if (files?.length) void read(files);
		e.currentTarget.value = '';
	}}
	hidden
/>

<Dialog title="Import OctoBanger" size="large" {onclose}>
	{#if uploads.length === 0}
		<div class="empty">
			<p>
				An <b>.OBC</b> controller, or a project archive with the controller and its MP3s in it.
				Each prop becomes a timeline, an ambience loop if it had one, and a flow from its trigger.
			</p>
			<button class="btn btn-primary" disabled={reading} onclick={() => fileInput?.click()}>
				{reading ? 'Reading…' : 'Choose files…'}
			</button>
		</div>
	{:else}
		<div class="props">
			{#each entries as e (e.key)}
				<button class="prop" class:on={e.key === current} onclick={() => choose(e.key)}>
					{e.described.controller.name}
					{#if done[e.key]}<span class="tick">imported</span>{/if}
				</button>
			{/each}
			<button class="btn btn-ghost" disabled={reading} onclick={() => fileInput?.click()}>Add…</button>
		</div>
	{/if}

	{#if failed}<p class="failed" role="alert">{failed}</p>{/if}

	{#if open}
		{@const d = open.described}
		{@const c = d.controller}
		{#if done[open.key]}
			{@const r = done[open.key]}
			<section>
				<p>{r.created} new, {r.updated} updated.</p>
				<ul class="notes">
					{#each r.missing as gone (gone)}<li>{gone}</li>{/each}
					{#each r.warnings as warning (warning)}<li>{warning}</li>{/each}
				</ul>
			</section>
		{:else}
			<section class="head">
				<div>
					<b>{c.name}</b>
					<span class="dim">{c.comment}</span>
				</div>
				<div class="dim">
					{clock(c.endMs)} · then cold for {c.resetDelayS} s · trigger {triggerText(c.trigger)}{d.triggerPin ? ` on ${d.triggerPin}` : ''}
				</div>
			</section>

			<section>
				<div class="row-head">
					<h4>Channels</h4>
					{#if fillable.length > 0}
						<select
							class="select"
							value=""
							onchange={(e) => {
								if (e.currentTarget.value) fill(e.currentTarget.value);
								e.currentTarget.value = '';
							}}
						>
							<option value="">Fill from node…</option>
							{#each fillable as [id, name] (id)}<option value={id}>{name}</option>{/each}
						</select>
					{/if}
				</div>
				<table>
					<tbody>
						{#each d.channels.filter((ch) => ch.used) as ch (ch.number)}
							{@const named = c.channels[ch.number - 1]}
							<tr class:clash={doubled.has(ch.number)}>
								<td class="num">{ch.number}</td>
								<td>
									<div>{named?.name}</div>
									<div class="dim small">{named?.comment}{ch.pin ? ` · was ${ch.pin}` : ''}{named?.activeLow ? ' · Active Low' : ''}</div>
								</td>
								<td class="strip" title="When it is on">
									{#each ch.spans as [from, to], i (i)}
										<span style:left="{(from / c.endMs) * 100}%" style:width="{Math.max(0.5, ((to - from) / c.endMs) * 100)}%"></span>
									{/each}
								</td>
								<td>
									<select class="select" bind:value={assigned[ch.number]}>
										<option value={null}>Placeholder (assign later)</option>
										{#each outputs as o (portId(o))}<option value={portId(o)}>{o.label}</option>{/each}
									</select>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
				{#if d.channels.some((ch) => !ch.used)}
					<details>
						<summary class="dim small">Never on ({d.channels.filter((ch) => !ch.used).length})</summary>
						<table>
							<tbody>
								{#each d.channels.filter((ch) => !ch.used) as ch (ch.number)}
									<tr class:clash={doubled.has(ch.number)}>
										<td class="num">{ch.number}</td>
										<td>{c.channels[ch.number - 1]?.name}</td>
										<td>
											<select class="select" bind:value={assigned[ch.number]}>
												<option value={null}>Not imported</option>
												{#each outputs as o (portId(o))}<option value={portId(o)}>{o.label}</option>{/each}
											</select>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</details>
				{/if}
				{#if doubled.size > 0}<p class="failed">Two channels go to one output.</p>{/if}
			</section>

			<section>
				<h4>Trigger</h4>
				<div class="line">
					<select class="select" bind:value={trigger}>
						<option value="">None: the flow's button only</option>
						{#each inputs as i (portId(i))}<option value={portId(i)}>{i.label}</option>{/each}
					</select>
					<select class="select" bind:value={edge} disabled={!trigger}>
						<option value="RisingEdge">fires going on</option>
						<option value="FallingEdge">fires going off</option>
					</select>
				</div>
				<p class="hint">
					The OctoBanger's trigger-out pin, which set off the next prop in a chain, is not imported.
					To chain props, add a <i>Play timeline</i> action to the flow, or an event to the scare.
				</p>
			</section>

			{#if d.sounds.length > 0}
				<section>
					<h4>Sound</h4>
					<div class="line">
						<select class="select" bind:value={destination}>
							<option value="">No sound</option>
							{#each $zones as z (z.id)}<option value={`zone:${z.id}`}>{z.name}</option>{/each}
							{#each nodes as n (n.id)}
								{#if 'OpenHaunt' in n.address}<option value={`node:${n.address.OpenHaunt.serial}`}>{n.name} (node speaker)</option>{/if}
							{/each}
						</select>
					</div>
					<ul class="sounds">
						{#each d.sounds as s (s.role)}
							<li>
								<span class="role">{s.role === 'scare' ? 'Scare' : 'Ambience'}</span>
								<span>{s.name}</span>
								<span class="dim">{soundState(s)}</span>
								{#if !s.found}
									<label class="btn btn-ghost small">
										Supply…
										<input
											type="file"
											accept="audio/*,.mp3,.wav"
											hidden
											onchange={(e) => {
												const file = e.currentTarget.files?.[0];
												if (file) {
													supplied = { ...supplied, [s.name]: file };
													if (s.role === 'ambient') ambience = true;
												}
											}}
										/>
									</label>
								{/if}
								{#if s.role === 'ambient'}
									<label class="small"><input type="checkbox" bind:checked={ambience} /> loop it between scares</label>
								{/if}
							</li>
						{/each}
					</ul>
					{#if ambience}
						<p class="hint">The ambience starts when the first scare ends. Play it once at doors to start it sooner.</p>
					{/if}
				</section>
			{/if}

			{#if d.notes.length > 0}
				<section>
					<h4>Not carried over</h4>
					<ul class="notes">{#each d.notes as note (note)}<li>{note}</li>{/each}</ul>
				</section>
			{/if}
		{/if}
	{/if}

	{#snippet footer()}
		<button class="btn btn-ghost" onclick={onclose}>{Object.keys(done).length > 0 ? 'Done' : 'Cancel'}</button>
		{#if open && !done[open.key]}
			<button class="btn btn-primary" disabled={importing || doubled.size > 0} onclick={importOpen}>
				{importing ? 'Importing…' : `Import ${open.described.controller.name}`}
			</button>
		{/if}
	{/snippet}
</Dialog>

<style>
	.empty {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 10px;
	}
	.props {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin-bottom: 10px;
	}
	.prop {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		color: var(--text);
		padding: 4px 10px;
		font: inherit;
		cursor: pointer;
	}
	.prop.on {
		border-color: var(--accent);
	}
	.tick {
		color: var(--text-dim);
		font-size: var(--font-xs);
		margin-left: 4px;
	}
	section {
		margin: 12px 0;
	}
	h4 {
		margin: 0 0 6px;
		font-size: var(--font-xs);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.row-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	table {
		width: 100%;
		border-collapse: collapse;
	}
	td {
		padding: 4px 6px;
		vertical-align: middle;
		border-bottom: 1px solid var(--line);
	}
	td.num {
		width: 2ch;
		color: var(--text-dim);
	}
	td.strip {
		position: relative;
		width: 30%;
		min-width: 120px;
		height: 18px;
	}
	td.strip span {
		position: absolute;
		top: 6px;
		height: 8px;
		background: var(--accent);
		border-radius: 1px;
	}
	tr.clash td {
		background: color-mix(in srgb, var(--danger, #ef4444) 15%, transparent);
	}
	.line {
		display: flex;
		gap: 6px;
		flex-wrap: wrap;
	}
	.sounds {
		list-style: none;
		padding: 0;
		margin: 8px 0 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.sounds li {
		display: flex;
		gap: 10px;
		align-items: center;
		flex-wrap: wrap;
	}
	.role {
		width: 7ch;
		color: var(--text-dim);
	}
	.notes {
		margin: 0;
		padding-left: 18px;
	}
	.dim {
		color: var(--text-dim);
	}
	.small {
		font-size: var(--font-xs);
	}
	.hint {
		color: var(--text-dim);
		font-size: var(--font-xs);
		margin: 6px 0 0;
	}
	.failed {
		color: var(--danger, #ef4444);
	}
</style>
