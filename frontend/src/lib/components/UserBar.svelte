<script lang="ts">
	/**
	 * Who is at this console, and the two buttons that depend on knowing.
	 *
	 * In the top bar rather than a panel because undo is not any one panel's, and
	 * because "who am I" is the sort of thing that should be visible without being
	 * looked for — an operator taking back a change wants to be sure it is *their*
	 * change first.
	 */

	import { addUser, beUser, currentUser, users, userId } from '$lib/stores/user.js';
	import { redo, undo } from '$lib/stores/undo.js';
	import { focusOnMount } from '$lib/actions.js';

	let open = $state(false);
	let adding = $state(false);
	let name = $state('');

	async function create() {
		await addUser(name);
		name = '';
		adding = false;
		open = false;
	}
</script>

<div class="userbar">
	<button
		class="chip undo"
		title="Take back your last change (Ctrl-Z)"
		aria-label="Undo"
		disabled={!$userId}
		onclick={undo}
	>↶</button>
	<button
		class="chip undo"
		title="Put back your last undo (Ctrl-Shift-Z)"
		aria-label="Redo"
		disabled={!$userId}
		onclick={redo}
	>↷</button>

	<button
		class="chip who"
		class:unknown={!$currentUser}
		title={$currentUser ? 'Working as ' + $currentUser.name : 'Nobody is signed in — changes cannot be taken back'}
		onclick={() => (open = !open)}
	>
		{#if $currentUser}
			<span class="dot" style:background={$currentUser.colour}></span>
			{$currentUser.name}
		{:else}
			Who are you?
		{/if}
	</button>

	{#if open}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="menu" onpointerleave={() => { if (!adding) open = false; }}>
			{#each $users as user (user.id)}
				<button class:on={user.id === $userId} onclick={() => { beUser(user.id); open = false; }}>
					<span class="dot" style:background={user.colour}></span>
					{user.name}
				</button>
			{/each}

			{#if adding}
				<form onsubmit={(e) => { e.preventDefault(); create(); }}>
					<input class="input" placeholder="Your name" bind:value={name} use:focusOnMount />
				</form>
			{:else}
				<button class="add" onclick={() => (adding = true)}>+ Somebody else</button>
			{/if}

			{#if $userId}
				<!-- Signing out is a real thing to want on a shared desk at the end of a
				     session, and it is not the same as being nobody by accident. -->
				<button class="out" onclick={() => { beUser(null); open = false; }}>Sign out</button>
			{/if}
		</div>
	{/if}
</div>

<style>
	.userbar {
		position: relative;
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.chip {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-xs);
		padding: 3px 8px;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.chip:disabled {
		opacity: 0.4;
		cursor: default;
	}

	.undo {
		font-size: 13px;
		line-height: 1;
		padding: 2px 7px;
	}

	.who {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	/* Nobody signed in is not an error, but it does mean nothing can be taken back,
	   and that is worth saying before somebody finds out by pressing Ctrl-Z. */
	.who.unknown {
		border-color: var(--live);
		color: var(--live);
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		flex: none;
	}

	.menu {
		position: absolute;
		top: 100%;
		right: 0;
		margin-top: 4px;
		z-index: 40;
		min-width: 180px;
		display: flex;
		flex-direction: column;
		background: var(--bg-panel);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		padding: 4px;
		box-shadow: 0 6px 18px #0008;
	}

	.menu button {
		display: flex;
		align-items: center;
		gap: 8px;
		background: none;
		border: none;
		color: var(--text);
		font: inherit;
		font-size: var(--font-sm);
		text-align: left;
		padding: 6px 8px;
		border-radius: 3px;
		cursor: pointer;
	}
	.menu button:hover {
		background: var(--bg-hover);
	}
	.menu button.on {
		color: var(--accent);
	}
	.menu .add,
	.menu .out {
		border-top: 1px solid var(--line);
		margin-top: 4px;
		padding-top: 8px;
		color: var(--text-dim);
	}

	.menu form {
		padding: 4px;
	}
	.menu .input {
		width: 100%;
	}
</style>
