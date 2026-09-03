<script lang="ts">
	/**
	 * The console's own log.
	 *
	 * Deliberately not the History panel. That one is the oplog — who changed what,
	 * per person, undoable, replicated. This is diagnostics: per station, nobody's
	 * to undo, and hundreds of lines a second at `debug`. A peer lost mid-show, an
	 * output whose socket would not bind, a node that stopped answering, a plugin
	 * saying something about itself — all of them used to go to a stdout that does
	 * not exist under the desktop app, under a packaged `.app`, or in a browser.
	 *
	 * Two streams become one list: `log.tail` answers the backlog when the panel
	 * opens, and live batches arrive on the `logs` path. `$lib/logs.ts` merges them
	 * by `(node_id, seq)` and is where every rule about that lives — this file is
	 * the view over it.
	 *
	 * **A chip lights a peer.** Every station's warnings arrive whether or not
	 * anybody is looking; clicking a peer's chip asks it for more, up to what that
	 * peer is keeping for itself, and un-clicking it — or closing this panel, or
	 * closing this tab — puts it back. Nothing expires, because the ask is
	 * recomputed from who is actually watching.
	 */

	import { onMount } from 'svelte';

	import type { LogLevel, LogLine, LogSource } from '$lib/generated/index.js';
	import { getClientContext } from '$lib/ws/context.js';
	import { LEVELS, LogBuffer, type Entry } from '$lib/logs.js';

	const client = getClientContext();

	type Tail = {
		lines: LogLine[];
		nodeId: string;
		captureLevel: LogLevel;
		publishLevel: LogLevel;
		file: string | null;
		raised: Record<string, LogLevel>;
	};

	let buffer = $state(new LogBuffer());
	/** Bumped whenever the buffer changes, since the buffer itself is not reactive. */
	let version = $state(0);
	let thisStation = $state<string | null>(null);
	let captureLevel = $state<LogLevel>('info');
	let file = $state<string | null>(null);
	let unavailable = $state<string | null>(null);

	/** What this panel shows, which is not what the station keeps. */
	let showing = $state<LogLevel>('debug');
	let search = $state('');
	let follow = $state(true);
	/** Peers this browser has raised, and to what. */
	let raised = $state<Record<string, LogLevel>>({});
	/** Which sources are hidden, keyed as the chips are. */
	let hidden = $state<Set<string>>(new Set());

	let list = $state<HTMLElement | null>(null);

	const sourceKey = (s: LogSource) => (s.kind === 'station' ? 'station' : `${s.kind}:${s.id}`);
	const sourceLabel = (s: LogSource) => (s.kind === 'station' ? 'station' : s.id);

	const entries = $derived.by((): Entry[] => {
		version; // read, so this recomputes when the buffer is written to
		const needle = search.trim().toLowerCase();
		return buffer
			.entries(showing, (s) => !hidden.has(sourceKey(s)))
			.filter(
				(e) =>
					e.kind === 'gap' ||
					!needle ||
					e.line.message.toLowerCase().includes(needle) ||
					e.line.target.toLowerCase().includes(needle)
			);
	});

	/** Every source that has said anything, so the chips are what exists. */
	const sources = $derived.by(() => {
		version;
		const seen = new Map<string, LogSource>();
		for (const e of buffer.entries()) {
			if (e.kind === 'line') seen.set(sourceKey(e.line.source), e.line.source);
		}
		return [...seen.entries()].sort(([a], [b]) => a.localeCompare(b));
	});

	/** Stations other than this one, which are the ones a chip can raise. */
	const peers = $derived.by(() => {
		version;
		return buffer.stations().filter((id) => id !== thisStation);
	});

	const short = (id: string) => id.replace(/-/g, '').slice(0, 8);
	const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour12: false });

	function take(lines: LogLine[]) {
		if (buffer.add(lines) > 0) version++;
	}

	async function fetchBacklog() {
		try {
			const answer = (await client.call('log.tail', { limit: 2000 })) as Tail;
			thisStation = answer.nodeId;
			captureLevel = answer.captureLevel;
			file = answer.file;
			raised = answer.raised ?? {};
			unavailable = null;
			take(answer.lines ?? []);
		} catch (e) {
			// A station started without a log is a real configuration, not a fault.
			unavailable = e instanceof Error ? e.message : String(e);
		}
	}

	async function setCaptureLevel(level: LogLevel) {
		const answer = (await client.call('log.setLevel', { level })) as {
			captureLevel: LogLevel;
		};
		captureLevel = answer.captureLevel;
	}

	async function toggleRaise(nodeId: string) {
		if (raised[nodeId]) {
			await client.call('log.unwatch', { nodeId });
			const { [nodeId]: _gone, ...rest } = raised;
			raised = rest;
		} else {
			await client.call('log.watch', { nodeId, level: 'debug' });
			raised = { ...raised, [nodeId]: 'debug' };
		}
	}

	$effect(() => {
		// Read so the effect re-runs on every new line, then scroll.
		version;
		if (follow && list) list.scrollTop = list.scrollHeight;
	});

	onMount(() => {
		const stop = client.subscribe('logs', (v: unknown) => {
			if (Array.isArray(v)) take(v as LogLine[]);
		});
		fetchBacklog();
		const stopConnect = client.addConnectListener(fetchBacklog);

		return () => {
			stop();
			stopConnect();
			// Closing the panel is letting go of every peer it had raised. The
			// station recomputes and tells them; if this browser dies instead, the
			// socket closing does the same thing.
			for (const nodeId of Object.keys(raised)) {
				client.call('log.unwatch', { nodeId }).catch(() => {});
			}
		};
	});
</script>

<div class="log">
	<header>
		<h2>System log</h2>
		<div class="controls">
			<input class="search" type="search" placeholder="filter…" bind:value={search} />
			<label class="showing">
				show
				<select bind:value={showing}>
					{#each LEVELS as level (level)}
						<option value={level}>{level}</option>
					{/each}
				</select>
			</label>
			<label class="follow">
				<input type="checkbox" bind:checked={follow} /> follow
			</label>
		</div>
	</header>

	{#if unavailable}
		<p class="empty">
			This station has no log to show — it was started without one. {unavailable}
		</p>
	{:else}
		<div class="chips">
			{#each sources as [key, source] (key)}
				<button
					class="chip"
					class:off={hidden.has(key)}
					onclick={() => {
						const next = new Set(hidden);
						if (!next.delete(key)) next.add(key);
						hidden = next;
					}}
					title={source.kind === 'plugin' ? `only what ${source.id} said` : source.kind}
				>
					{sourceLabel(source)}
				</button>
			{/each}
			{#each peers as nodeId (nodeId)}
				<!-- A peer's chip is not a filter but an ask: lit, that station sends
				     its debug down the link while somebody is here to read it. -->
				<button
					class="chip peer"
					class:lit={!!raised[nodeId]}
					onclick={() => toggleRaise(nodeId)}
					title="ask {short(nodeId)} for more, as far as it keeps for itself"
				>
					{short(nodeId)}{raised[nodeId] ? ' ✦' : ''}
				</button>
			{/each}
		</div>

		<div class="lines" bind:this={list}>
			{#if entries.length === 0}
				<p class="empty">Nothing yet.</p>
			{/if}
			{#each entries as entry (entry.kind === 'line' ? `${entry.line.node_id}:${entry.line.seq}` : `gap:${entry.nodeId}:${entry.seq}`)}
				{#if entry.kind === 'gap'}
					<!-- Said out loud. The station's broadcast drops for a listener that
					     fell behind rather than slowing the console down to keep it, and a
					     log that quietly skipped a thousand lines would be worse than one
					     that admits it. -->
					<p class="gap">
						{entry.missing.toLocaleString()} lines from {short(entry.nodeId)} did not arrive
					</p>
				{:else}
					{@const line = entry.line}
					<p class="line {line.level}">
						<span class="at">{clock(line.at_ms)}</span>
						{#if line.node_id !== thisStation}
							<span class="station">{short(line.node_id)}</span>
						{/if}
						<span class="level">{line.level}</span>
						{#if line.source.kind !== 'station'}
							<span class="src {line.source.kind}">{line.source.id}</span>
						{/if}
						<span class="target">{line.target}</span>
						<span class="message">{line.message}</span>
					</p>
				{/if}
			{/each}
		</div>

		<footer>
			<span>
				keeping
				<select
					class="level-select"
					value={captureLevel}
					onchange={(e) => setCaptureLevel(e.currentTarget.value as LogLevel)}
				>
					{#each LEVELS as level (level)}
						<option value={level}>{level}</option>
					{/each}
				</select>
			</span>
			{#if file}
				<!-- The ring holds a few thousand lines. Anything older is here, which
				     is also what survives the crash somebody is usually looking for. -->
				<span class="file" title={file}>this run is also written to {file}</span>
			{/if}
		</footer>
	{/if}
</div>

<style>
	.log {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		padding: 12px 14px;
		gap: 8px;
	}

	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
	}

	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}

	.controls {
		display: flex;
		align-items: center;
		gap: 10px;
		font-size: var(--font-xs);
		color: var(--text-faint);
	}

	.search {
		background: var(--bg-input, transparent);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-xs);
		padding: 3px 8px;
		width: 16ch;
	}

	select {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 4px;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}

	.chip {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: 999px;
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 2px 9px;
		cursor: pointer;
	}
	.chip.off {
		color: var(--text-faint);
		opacity: 0.45;
	}
	.chip.peer {
		border-style: dashed;
	}
	.chip.peer.lit {
		border-style: solid;
		border-color: var(--live);
		color: var(--live);
	}

	.lines {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overflow-x: auto;
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: var(--font-xs);
		line-height: 1.5;
	}

	.line {
		display: flex;
		gap: 8px;
		white-space: pre;
		color: var(--text-dim);
	}
	.line .at,
	.line .target {
		color: var(--text-faint);
	}
	.line .level {
		text-transform: uppercase;
		width: 5ch;
	}
	.line.warn .level,
	.line.warn .message {
		color: var(--warn, #d8a657);
	}
	.line.error .level,
	.line.error .message {
		color: var(--danger, #e06c75);
	}
	.line.info .message {
		color: var(--text);
	}
	.line .station {
		color: var(--live);
	}
	.line .src {
		color: var(--accent, var(--live));
	}
	.line .message {
		white-space: pre-wrap;
	}

	.gap {
		color: var(--text-faint);
		font-style: italic;
		text-align: center;
		padding: 2px 0;
	}

	.empty {
		color: var(--text-dim);
		font-size: var(--font-sm);
		max-width: 48ch;
		line-height: 1.5;
	}

	footer {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		color: var(--text-faint);
		font-size: var(--font-xs);
	}

	.file {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
