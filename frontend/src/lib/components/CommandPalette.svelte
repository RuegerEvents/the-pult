<script lang="ts">
	/**
	 * ⌘K: find anything by name and do it.
	 *
	 * Panels, every object of every kind, and the console's own verbs, from one
	 * registry (`$lib/commands.ts`) asked each time it opens, so what was created a
	 * second ago is here. Keyboard first: type, arrows, Enter; Escape closes. An object
	 * is listed twice — once to act (recall, apply, Go) and once to inspect — because
	 * those are the two things the grammar says a person does to one.
	 */

	import { focusOnMount } from '$lib/actions.js';
	import { allCommands, searchCommands, type Command } from '$lib/commands.js';
	import { categoryColor } from '$lib/layout/categories.js';
	import { addToast } from '$lib/toasts.js';

	let { onclose }: { onclose: () => void } = $props();

	let query = $state('');
	let cursor = $state(0);
	const everything = allCommands();
	/** Twelve at most: a palette is for knowing what you want, not for browsing. */
	const shown = $derived(searchCommands(query, everything).slice(0, query ? 12 : 10));

	$effect(() => {
		void query;
		cursor = 0;
	});

	async function run(command: Command) {
		onclose();
		try {
			await command.run();
		} catch (e) {
			addToast(`${command.title}: ${e}`);
		}
	}

	function onkey(event: KeyboardEvent) {
		if (event.key === 'ArrowDown') {
			event.preventDefault();
			cursor = Math.min(shown.length - 1, cursor + 1);
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			cursor = Math.max(0, cursor - 1);
		} else if (event.key === 'Enter') {
			event.preventDefault();
			const command = shown[cursor];
			if (command) void run(command);
		} else if (event.key === 'Escape') {
			event.preventDefault();
			event.stopPropagation();
			onclose();
		}
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={(e) => e.target === e.currentTarget && onclose()}>
	<div class="palette" role="dialog" aria-modal="true" aria-label="Command palette">
		<input
			class="search"
			placeholder="Find a panel, a preset, a cue… or a verb"
			aria-label="Command"
			bind:value={query}
			use:focusOnMount
			onkeydown={onkey}
		/>
		<ul role="listbox">
			{#each shown as command, i (command.id)}
				{@const Icon = command.icon}
				<li role="option" aria-selected={i === cursor}>
					<button
						class:on={i === cursor}
						onclick={() => run(command)}
						onpointerenter={() => (cursor = i)}
						style:--cat={command.category ? categoryColor(command.category) : 'var(--text-dim)'}
					>
						<span class="mark">{#if Icon}<Icon size={15} />{/if}</span>
						<span class="title">{command.title}</span>
						<span class="section">{command.section}</span>
						{#if command.hint}<span class="hint">{command.hint}</span>{/if}
					</button>
				</li>
			{:else}
				<li class="none">Nothing called anything like “{query}”.</li>
			{/each}
		</ul>
		<footer>↑↓ to choose · Enter to do it · Esc to close</footer>
	</div>
</div>

<style>
	.scrim {
		position: fixed;
		inset: 0;
		z-index: 50;
		display: flex;
		justify-content: center;
		align-items: flex-start;
		padding-top: 12vh;
		background: rgb(0 0 0 / 45%);
	}
	.palette {
		width: min(620px, calc(100% - 24px));
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: 8px;
		box-shadow: 0 16px 48px rgb(0 0 0 / 60%);
		overflow: hidden;
	}
	.search {
		width: 100%;
		padding: 14px 16px;
		background: none;
		border: none;
		border-bottom: 1px solid var(--line);
		color: var(--text-bright);
		font: inherit;
		font-size: var(--font-md);
		outline: none;
	}
	ul {
		list-style: none;
		max-height: 50vh;
		overflow-y: auto;
		padding: 4px;
	}
	button {
		display: flex;
		align-items: center;
		gap: 10px;
		width: 100%;
		min-height: 38px;
		padding: 0 10px;
		background: none;
		border: none;
		border-radius: var(--radius);
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		cursor: pointer;
	}
	button.on {
		background: var(--bg-hover);
		color: var(--text-bright);
	}
	.mark {
		display: inline-grid;
		place-items: center;
		width: 24px;
		height: 24px;
		flex: none;
		border-radius: 5px;
		color: var(--cat);
		background: color-mix(in srgb, var(--cat) 14%, transparent);
	}
	.title {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.section {
		font-size: 10px;
		color: var(--text-faint);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.hint {
		font-size: var(--font-xs);
		color: var(--text-dim);
		min-width: 4.5rem;
		text-align: right;
	}
	.none {
		padding: 14px;
		color: var(--text-faint);
		font-style: italic;
		font-size: var(--font-sm);
	}
	footer {
		padding: 7px 14px;
		border-top: 1px solid var(--line);
		font-size: 10px;
		color: var(--text-faint);
	}
</style>
