<script lang="ts">
	/**
	 * Who is at this console, and the two buttons that depend on knowing.
	 *
	 * In the top bar rather than a panel because undo is not any one panel's, and
	 * because "who am I" is the sort of thing that should be visible without being
	 * looked for — an operator taking back a change wants to be sure it is *their*
	 * change first.
	 */

	import { addUser, beUser, currentUser, hasChosen, signOut, users, userId } from '$lib/stores/user.js';
	import { redo, undo } from '$lib/stores/undo.js';
	import { focusOnMount } from '$lib/actions.js';
	import Undo2 from '@lucide/svelte/icons/undo-2';
	import Redo2 from '@lucide/svelte/icons/redo-2';

	/** One or two letters for the badge: the initials of a name, or its first two. */
	function initials(name: string): string {
		const words = name.trim().split(/\s+/).filter(Boolean);
		if (words.length === 0) return '?';
		if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
		return (words[0][0] + words[words.length - 1][0]).toUpperCase();
	}

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
	<!-- Never disabled: a show always has an operator, so there is always somebody
	     whose last change this takes back. -->
	<button class="tb-icon" title="Take back your last change (Ctrl-Z)" aria-label="Undo" onclick={undo}>
		<Undo2 size={16} />
	</button>
	<button class="tb-icon" title="Put back your last undo (Ctrl-Shift-Z)" aria-label="Redo" onclick={redo}>
		<Redo2 size={16} />
	</button>

	<!-- Marked while nobody has said who they are. Not because anything is broken —
	     changes are attributed and undoable either way — but because until somebody
	     says, every unsaid console is sharing one operator's undo history, and that
	     is worth finding out before two people discover it by pressing Ctrl-Z. -->
	<button
		class="tb-icon who"
		class:unsaid={!$hasChosen}
		aria-label="Who is at this console"
		aria-expanded={open}
		title={$hasChosen
			? 'Working as ' + (($currentUser && $currentUser.name) || 'somebody who is no longer in the show')
			: 'Working as the shared operator — say who you are to get your own undo history'}
		onclick={() => (open = !open)}
	>
		<span class="avatar" style:background={$currentUser?.color ?? undefined}>
			{$currentUser ? initials($currentUser.name) : '?'}
		</span>
	</button>

	{#if open}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="menu" onpointerleave={() => { if (!adding) open = false; }}>
			{#each $users as user (user.id)}
				<button class:on={user.id === $userId} onclick={() => { beUser(user.id); open = false; }}>
					<span class="dot" style:background={user.color}></span>
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

			{#if $hasChosen}
				<!-- Signing out is a real thing to want on a shared desk at the end of a
				     session. It lands on the show's operator rather than on nobody:
				     nobody is a state in which nothing can be taken back. -->
				<button class="out" onclick={() => { signOut(); open = false; }}>Sign out</button>
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

	.avatar {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border-radius: 50%;
		background: var(--bg-raised);
		color: #fff;
		font-size: 10px;
		font-weight: 700;
		letter-spacing: 0.02em;
		text-shadow: 0 1px 1px rgb(0 0 0 / 0.5);
	}
	/* Nobody having said is not an error — changes are attributed and undoable
	   either way — but it does mean this console shares an undo history with every
	   other one that has not said, which is worth noticing before two operators
	   find out by pressing Ctrl-Z. So the badge wears a ring until somebody says. */
	.who.unsaid .avatar {
		box-shadow: 0 0 0 2px var(--live);
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
