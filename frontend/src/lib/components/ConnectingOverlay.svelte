<script lang="ts">
	/**
	 * The screen while there is no console to talk to.
	 *
	 * Everything in this app is a view of state that lives on the backend, so without
	 * one there is nothing to look at: panels sit empty, a fader moves and springs
	 * back, and none of it says why. This covers the lot and says the one thing that
	 * matters — which console is not answering, and that it is still being asked.
	 */

	let {
		everConnected,
		address,
		onretry
	}: { everConnected: boolean; address: string; onretry: () => void } = $props();
</script>

<div class="cover" role="status" aria-live="polite">
	<div class="panel">
		<div class="spinner" aria-hidden="true"></div>
		<h1>{everConnected ? 'The console stopped answering' : 'Connecting to the console'}</h1>
		<p class="where">{address}</p>
		<p class="hint">
			{everConnected
				? 'Still trying. The show is on the console rather than in this browser, so nothing has been lost.'
				: 'Nothing can be read or changed until it answers.'}
		</p>
		<button onclick={onretry}>Try now</button>
	</div>
</div>

<style>
	.cover {
		position: fixed;
		inset: 0;
		z-index: 100;
		display: grid;
		place-items: center;
		/* Opaque, not a veil: a half-hidden workspace showing state nobody can change
		   is the confusion this exists to end. */
		background: var(--bg, #1a1a1a);
	}

	.panel {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
		padding: 0 24px;
		text-align: center;
	}

	.spinner {
		width: 34px;
		height: 34px;
		margin-bottom: 6px;
		border: 3px solid var(--line-strong, #3a3a3a);
		border-top-color: var(--accent, #4a9eff);
		border-radius: 50%;
		animation: spin 0.9s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(1turn);
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.spinner {
			animation: breathe 1.8s ease-in-out infinite;
		}

		@keyframes breathe {
			50% {
				opacity: 0.25;
			}
		}
	}

	h1 {
		font-size: 0.95rem;
		font-weight: 500;
		color: var(--text-bright, #fff);
	}

	.where {
		font-family: monospace;
		font-size: var(--font-sm, 12px);
		color: var(--text-dim, #888);
	}

	.hint {
		max-width: 34ch;
		font-size: var(--font-sm, 12px);
		line-height: 1.5;
		color: var(--text-faint, #555);
	}

	button {
		margin-top: 6px;
		background: none;
		border: 1px solid var(--line-strong, #3a3a3a);
		border-radius: var(--radius, 4px);
		color: #bbb;
		font: inherit;
		font-size: var(--font-sm, 12px);
		padding: 5px 14px;
		cursor: pointer;
	}
	button:hover {
		border-color: var(--line-input, #555);
		color: var(--text-bright, #fff);
	}
</style>
