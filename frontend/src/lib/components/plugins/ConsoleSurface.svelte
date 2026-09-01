<script lang="ts" module>
	type Entry = { kind: string; text: string; caret?: string };

	/**
	 * Scrollback and history outlive the component: switching tabs unmounts a
	 * panel, and a console that forgot the evening every time the operator
	 * glanced at the patch would be useless. Module scope, because the plain
	 * script runs once per mounted instance; keyed per surface.
	 */
	const sessions = new Map<string, { entries: Entry[]; history: string[] }>();
</script>

<script lang="ts">
	/**
	 * The console surface: a prompt, its scrollback, and a completion popup.
	 *
	 * This component knows nothing about any grammar. Every keystroke's
	 * completions and every submitted line go to the plugin named in its props,
	 * over the surface protocol (`surface.exec` / `surface.complete` /
	 * `surface.help`); what comes back is lines to print, an error with a span
	 * to underline, and optionally effects — a selection to apply in this
	 * browser, because the selection is the operator's, not the show's.
	 */

	import { onMount } from 'svelte';
	import { get } from 'svelte/store';

	import type { PluginStatus } from '$lib/generated/index.js';
	import type { SelectionQuery } from '$lib/selection.js';
	import { registerConsoleFocus } from '$lib/stores/plugins.js';
	import { applySelectionEffect, selection } from '$lib/stores/selection.js';
	import { showClient } from '$lib/stores/show.js';
	import { userId } from '$lib/stores/user.js';

	let {
		pluginId,
		surfaceId = 'console',
		status
	}: { pluginId: string; surfaceId?: string; status: PluginStatus } = $props();

	type Item = { text: string; detail?: string };

	// Resolved once, when the panel mounts: a plain object, not reactive state,
	// so the unmount cleanup below can still reach it to write the scrollback
	// back. The props that name it do not change over an instance's life.
	const session = (() => {
		const key = `${pluginId}:${surfaceId}`;
		let held = sessions.get(key);
		if (!held) {
			held = { entries: [], history: [] };
			sessions.set(key, held);
		}
		return held;
	})();

	let entries = $state<Entry[]>([]);
	let line = $state('');
	let busy = $state(false);
	let items = $state<Item[]>([]);
	let replaceFrom = $state(0);
	let picked = $state(0);
	let recalling = $state(-1);
	let input = $state<HTMLInputElement | null>(null);
	let scroll = $state<HTMLDivElement | null>(null);

	const failed = $derived(status.state === 'Failed' ? status.reason : null);
	// Only items that insert something count as choosable; a bare hint row
	// (empty text) is advice, not an option.
	const choosable = $derived(items.filter((i) => i.text !== ''));

	function ctx() {
		return { selection: get(selection), userId: get(userId) };
	}

	function call(method: string, payload: unknown): Promise<unknown> {
		return showClient().call(`plugin.${pluginId}.surface.${method}`, { payload, ctx: ctx() });
	}

	async function refreshCompletions() {
		const at = input?.selectionStart ?? line.length;
		try {
			const result = (await call('complete', { line, cursor: at })) as {
				items?: Item[];
				replaceFrom?: number;
			} | null;
			items = result?.items ?? [];
			replaceFrom = result?.replaceFrom ?? at;
			picked = 0;
		} catch {
			items = [];
		}
	}

	function accept(item: Item) {
		const at = input?.selectionStart ?? line.length;
		line = line.slice(0, replaceFrom) + item.text + ' ' + line.slice(at);
		items = [];
		queueMicrotask(() => {
			input?.focus();
			const end = replaceFrom + item.text.length + 1;
			input?.setSelectionRange(end, end);
			void refreshCompletions();
		});
	}

	async function submit() {
		const typed = line.trim();
		if (!typed || busy) return;
		busy = true;
		items = [];
		entries.push({ kind: 'input', text: typed });
		session.history.push(typed);
		recalling = -1;
		line = '';
		try {
			const result = (await call('exec', { line: typed })) as {
				lines?: Entry[];
				error?: { message: string; span?: { start: number; end: number }; expected?: string[] };
				effects?: { selection?: { query?: SelectionQuery; fixtureIds?: string[] } };
			} | null;
			for (const out of result?.lines ?? []) entries.push({ kind: out.kind, text: out.text });
			if (result?.error) {
				const { message, span, expected } = result.error;
				// The caret line sits under the echoed input, pointing at the
				// span the parser blamed — the plugin measured it in bytes of
				// what was sent, which is the trimmed line echoed above.
				if (span && span.end >= span.start) {
					const width = Math.max(1, Math.min(span.end, typed.length) - span.start);
					entries.push({
						kind: 'error',
						text: message,
						caret: ' '.repeat(span.start) + '^'.repeat(width)
					});
				} else {
					entries.push({ kind: 'error', text: message });
				}
				if (expected?.length) {
					entries.push({ kind: 'info', text: `expected: ${expected.join(', ')}` });
				}
			}
			applySelectionEffect(result?.effects);
		} catch (e) {
			entries.push({ kind: 'error', text: e instanceof Error ? e.message : String(e) });
		} finally {
			busy = false;
			// The input was disabled while the command ran, which took the
			// focus with it; a command line that has to be clicked between
			// commands is not a command line.
			queueMicrotask(() => {
				scroll?.scrollTo({ top: scroll.scrollHeight });
				input?.focus();
			});
		}
	}

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter') {
			event.preventDefault();
			// Enter accepts a completion only when the operator has visibly
			// picked one with the arrows; otherwise it always runs the line.
			// Tab is the accept key. A prompt where Enter sometimes types
			// instead of doing is maddening.
			if (picked > 0 && choosable.length > 0) {
				accept(choosable[picked]);
			} else {
				void submit();
			}
			return;
		}
		if (event.key === 'Tab') {
			event.preventDefault();
			if (choosable.length > 0) accept(choosable[picked]);
			return;
		}
		if (event.key === 'Escape') {
			items = [];
			return;
		}
		if (event.key === 'ArrowDown') {
			if (choosable.length > 0) {
				event.preventDefault();
				picked = (picked + 1) % choosable.length;
			}
			return;
		}
		if (event.key === 'ArrowUp') {
			if (choosable.length > 0) {
				event.preventDefault();
				picked = (picked + choosable.length - 1) % choosable.length;
			} else if (session.history.length > 0) {
				event.preventDefault();
				recalling = recalling < 0 ? session.history.length - 1 : Math.max(0, recalling - 1);
				line = session.history[recalling];
			}
		}
	}

	onMount(() => {
		// Copied in on mount, copied out on unmount — plain data both ways, so
		// nothing subtle about proxies decides whether the evening survives a
		// glance at another tab.
		entries = [...session.entries];
		const unregister = registerConsoleFocus(() => input?.focus());
		return () => {
			session.entries = $state.snapshot(entries) as Entry[];
			unregister();
		};
	});
</script>

<div class="console">
	{#if failed !== null}
		<p class="dead">
			This panel's plugin ({pluginId}) is not running: {failed}
		</p>
	{:else}
		<div class="scrollback" bind:this={scroll} data-surface={surfaceId}>
			{#if entries.length === 0}
				<p class="hello">Type <code>help</code> for how this works. Tab completes.</p>
			{/if}
			{#each entries as entry, i (i)}
				<div class="entry {entry.kind}">
					{#if entry.kind === 'input'}<span class="prompt">›</span>{/if}
					<pre>{entry.text}</pre>
					{#if entry.caret}<pre class="caret">  {entry.caret}</pre>{/if}
				</div>
			{/each}
		</div>
		<div class="promptrow">
			<span class="prompt">›</span>
			<input
				bind:this={input}
				bind:value={line}
				oninput={() => void refreshCompletions()}
				onkeydown={onKeydown}
				onblur={() => setTimeout(() => (items = []), 150)}
				placeholder={busy ? '…' : 'fixture 1 thru 5 @ 80'}
				disabled={busy}
				spellcheck="false"
				autocomplete="off"
			/>
			{#if items.length > 0}
				<div class="popup">
					{#each items as item, i (i)}
						{#if item.text === ''}
							<span class="ghost">{item.detail}</span>
						{:else}
							<button
								class:on={choosable.indexOf(item) === picked}
								onpointerdown={(e) => {
									e.preventDefault();
									accept(item);
								}}
							>
								<span class="word">{item.text}</span>
								{#if item.detail}<span class="detail">{item.detail}</span>{/if}
							</button>
						{/if}
					{/each}
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.console {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: var(--font-sm);
	}

	.scrollback {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 8px 10px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.hello {
		color: var(--text-faint);
		margin: 0;
	}
	.hello code {
		color: var(--text-dim);
	}

	.entry {
		display: flex;
		gap: 6px;
		align-items: baseline;
	}
	.entry pre {
		margin: 0;
		white-space: pre-wrap;
		word-break: break-word;
		font: inherit;
	}
	.entry.input {
		color: var(--text-bright);
	}
	.entry.result pre {
		color: var(--text);
	}
	.entry.info pre {
		color: var(--text-dim);
	}
	.entry.error {
		flex-direction: column;
		gap: 0;
	}
	.entry.error pre {
		color: var(--bad);
	}
	.entry.error pre.caret {
		color: var(--accent-solid, var(--text-bright));
	}

	.prompt {
		color: var(--text-faint);
		flex: none;
	}

	.promptrow {
		position: relative;
		display: flex;
		align-items: center;
		gap: 6px;
		flex: none;
		padding: 6px 10px;
		border-top: 1px solid var(--line);
		background: var(--bg-chrome);
	}
	.promptrow input {
		flex: 1;
		min-width: 0;
		background: none;
		border: none;
		outline: none;
		color: var(--text-bright);
		font: inherit;
	}

	.popup {
		position: absolute;
		bottom: 100%;
		left: 18px;
		z-index: 40;
		display: flex;
		flex-direction: column;
		max-height: 240px;
		min-width: 200px;
		overflow-y: auto;
		margin-bottom: 4px;
		padding: 3px;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		box-shadow: 0 6px 18px #0008;
	}
	.popup button {
		display: flex;
		justify-content: space-between;
		gap: 14px;
		text-align: left;
		background: none;
		border: none;
		border-radius: 3px;
		color: var(--text);
		font: inherit;
		padding: 3px 8px;
		cursor: pointer;
	}
	.popup button:hover,
	.popup button.on {
		background: var(--bg-hover);
	}
	.popup .word {
		color: var(--text-bright);
	}
	.popup .detail {
		color: var(--text-faint);
	}
	.popup .ghost {
		color: var(--text-faint);
		font-style: italic;
		padding: 3px 8px;
	}

	.dead {
		color: var(--text-faint);
		font-style: italic;
		padding: 14px;
	}
</style>
