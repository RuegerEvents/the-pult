<script lang="ts">
	/**
	 * "This bar has six lights on it."
	 *
	 * A real question rather than a confirmation, which is why it exists at all and why
	 * it has three answers. Deleting a bar because it was the wrong length should not
	 * take six lanterns with it; deleting a truss because the whole thing has gone
	 * should. Nobody can guess which, and a console that guessed would be wrong half
	 * the time in the way that costs an hour.
	 *
	 * **Either answer is one gesture**, so either is one Ctrl-Z — including *Keep them
	 * where they are*, which writes a placement to every child before anything is
	 * deleted.
	 *
	 * A bare object, with nothing hanging off it, never gets here: it goes.
	 */
	import type { Fixture, SceneObject } from '$lib/generated/index.js';

	let {
		name,
		objects,
		fixtures,
		onanswer
	}: {
		name: string;
		objects: SceneObject[];
		fixtures: Fixture[];
		onanswer: (keepChildren: boolean | null) => void;
	} = $props();

	/** "6 fixtures and 2 objects", with neither half said when there is none of it. */
	const what = $derived(
		[
			fixtures.length > 0 ? `${fixtures.length} ${fixtures.length === 1 ? 'fixture' : 'fixtures'}` : null,
			objects.length > 0 ? `${objects.length} ${objects.length === 1 ? 'object' : 'objects'}` : null
		]
			.filter(Boolean)
			.join(' and ')
	);
</script>

<!-- Modal, because the answer changes what the next click does. `Toasts` is not, which
     is why this is its own component rather than a line in one. `fixed` rather than
     `absolute`: it is mounted at the root of the app and belongs to the window, since
     the verb can be reached from three different panels. -->
<div class="cover" role="presentation" onclick={() => onanswer(null)}>
	<div
		class="prompt"
		role="dialog"
		aria-modal="true"
		aria-label="Delete"
		onclick={(e) => e.stopPropagation()}
		onkeydown={(e) => e.key === 'Escape' && onanswer(null)}
		tabindex="-1"
	>
		<p><strong>{name}</strong> has {what} on it.</p>
		<div class="answers">
			<button class="primary" onclick={() => onanswer(false)}>Delete them too</button>
			<button onclick={() => onanswer(true)}>Keep them where they are</button>
			<button class="quiet" onclick={() => onanswer(null)}>Cancel</button>
		</div>
		<p class="note">
			Keeping them leaves each one exactly where it is in the room, hanging off
			nothing and clamped to nothing. Either way it is one act, and one Ctrl-Z.
		</p>
	</div>
</div>

<style>
	.cover {
		position: fixed;
		inset: 0;
		z-index: 40;
		display: grid;
		place-items: center;
		background: rgb(0 0 0 / 45%);
	}
	.prompt {
		min-width: 320px;
		max-width: 46ch;
		padding: 16px;
		border: 1px solid var(--line-strong, #333);
		border-radius: 5px;
		background: #1a1a1a;
		box-shadow: 0 12px 40px rgb(0 0 0 / 60%);
	}
	p { margin: 0 0 12px; color: #ccc; font-size: 13px; line-height: 1.5; }
	strong { color: #fff; }
	.answers { display: flex; flex-wrap: wrap; gap: 8px; }
	button {
		background: none;
		border: 1px solid var(--line-strong, #333);
		border-radius: 3px;
		color: #ccc;
		padding: 5px 12px;
		font: inherit;
		font-size: 12px;
		cursor: pointer;
	}
	button:hover { border-color: var(--line-input, #555); color: #fff; }
	.primary { background: #2a2f3a; border-color: var(--line-input, #555); color: #fff; }
	.quiet { color: #888; }
	.note { margin: 12px 0 0; color: #666; font-size: 11px; }
</style>
