<script lang="ts">
	/**
	 * The bar surface: one line in, a transcript out.
	 *
	 * Same protocol as the console surface, different posture: whatever the
	 * plugin behind it does with the text takes a moment (the reference use is
	 * a language model round trip), so the input shows a busy state and the
	 * answer arrives as a transcript below. If the browser can hear, a mic
	 * button types for you — feature-detected, frontend-only, nothing about it
	 * reaches the plugin.
	 */

	import { get } from 'svelte/store';

	import type { PluginStatus } from '$lib/generated/index.js';
	import { idsQuery } from '$lib/selection.js';
	import { selection, setQuery } from '$lib/stores/selection.js';
	import { showClient } from '$lib/stores/show.js';
	import { userId } from '$lib/stores/user.js';

	let {
		pluginId,
		surfaceId = 'bar',
		status
	}: { pluginId: string; surfaceId?: string; status: PluginStatus } = $props();

	type Entry = { kind: string; text: string };

	let text = $state('');
	let busy = $state(false);
	let listening = $state(false);
	let transcript = $state<Entry[]>([]);

	const failed = $derived(status.state === 'Failed' ? status.reason : null);

	// The Web Speech API where the browser has it; quietly absent elsewhere.
	type SpeechCtor = new () => {
		lang: string;
		onresult: (e: { results: ArrayLike<ArrayLike<{ transcript: string }>> }) => void;
		onend: () => void;
		onerror: () => void;
		start(): void;
		stop(): void;
	};
	const Speech: SpeechCtor | undefined =
		(globalThis as Record<string, unknown>).SpeechRecognition as SpeechCtor | undefined ??
		((globalThis as Record<string, unknown>).webkitSpeechRecognition as SpeechCtor | undefined);
	let recognizer: InstanceType<SpeechCtor> | null = null;

	function listen() {
		if (!Speech) return;
		if (listening) {
			recognizer?.stop();
			return;
		}
		recognizer = new Speech();
		recognizer.lang = navigator.language || 'en-US';
		recognizer.onresult = (e) => {
			const heard = Array.from({ length: e.results.length }, (_, i) => e.results[i][0].transcript)
				.join(' ')
				.trim();
			if (heard) {
				text = heard;
				void submit();
			}
		};
		recognizer.onend = () => (listening = false);
		recognizer.onerror = () => (listening = false);
		listening = true;
		recognizer.start();
	}

	async function submit() {
		const asked = text.trim();
		if (!asked || busy) return;
		busy = true;
		transcript = [{ kind: 'input', text: asked }];
		try {
			const result = (await showClient().call(`plugin.${pluginId}.surface.exec`, {
				payload: { line: asked },
				ctx: { selection: get(selection), userId: get(userId) }
			})) as {
				lines?: Entry[];
				error?: { message: string };
				effects?: { selection?: { fixtureIds?: string[] } };
			} | null;
			transcript = [
				{ kind: 'input', text: asked },
				...(result?.lines ?? []),
				...(result?.error ? [{ kind: 'error', text: result.error.message }] : [])
			];
			const ids = result?.effects?.selection?.fixtureIds;
			if (ids) setQuery(idsQuery(ids));
			if (!result?.error) text = '';
		} catch (e) {
			transcript = [
				{ kind: 'input', text: asked },
				{ kind: 'error', text: e instanceof Error ? e.message : String(e) }
			];
		} finally {
			busy = false;
		}
	}
</script>

<div class="bar" data-surface={surfaceId}>
	{#if failed !== null}
		<p class="dead">This panel's plugin ({pluginId}) is not running: {failed}</p>
	{:else}
		<form
			class="row"
			onsubmit={(e) => {
				e.preventDefault();
				void submit();
			}}
		>
			<input
				bind:value={text}
				placeholder={busy ? 'thinking…' : 'say what should happen'}
				disabled={busy}
				spellcheck="false"
			/>
			{#if Speech}
				<button
					type="button"
					class="mic"
					class:live={listening}
					title={listening ? 'Stop listening' : 'Speak instead of typing'}
					onclick={listen}
				>{listening ? '◉' : '🎙'}</button>
			{/if}
			<button type="submit" disabled={busy || !text.trim()}>{busy ? '…' : 'Do it'}</button>
		</form>
		<div class="transcript">
			{#if transcript.length === 0}
				<p class="hello">
					Plain language in, command lines out — “first five fixtures to 80 percent”.
					Every command it runs shows up here, and undo takes it back.
				</p>
			{/if}
			{#each transcript as entry, i (i)}
				<pre class="entry {entry.kind}">{entry.text}</pre>
			{/each}
		</div>
	{/if}
</div>

<style>
	.bar {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	.row {
		display: flex;
		gap: 8px;
		padding: 8px 10px;
		border-bottom: 1px solid var(--line);
		background: var(--bg-chrome);
		flex: none;
	}
	.row input {
		flex: 1;
		min-width: 0;
		background: var(--bg);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-bright);
		font: inherit;
		font-size: var(--font-sm);
		padding: 5px 9px;
		outline: none;
	}
	.row input:focus {
		border-color: var(--accent-solid, var(--line-strong));
	}
	.row button {
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		padding: 4px 10px;
		cursor: pointer;
	}
	.row button:disabled {
		color: var(--text-faint);
		cursor: default;
	}
	.mic.live {
		color: var(--bad);
	}

	.transcript {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 8px 10px;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: var(--font-sm);
	}
	.hello {
		color: var(--text-faint);
		font-family: system-ui, sans-serif;
		margin: 0;
	}
	.entry {
		margin: 0 0 2px;
		white-space: pre-wrap;
		word-break: break-word;
	}
	.entry.input {
		color: var(--text-bright);
	}
	.entry.info {
		color: var(--text-dim);
	}
	.entry.result {
		color: var(--text);
	}
	.entry.error {
		color: var(--bad);
	}

	.dead {
		color: var(--text-faint);
		font-style: italic;
		padding: 14px;
	}
</style>
