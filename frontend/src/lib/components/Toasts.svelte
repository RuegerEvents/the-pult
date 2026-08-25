<script lang="ts">
	import { toasts, dismissToast } from '$lib/toasts.js';
</script>

<div class="toast-stack" aria-live="polite">
	{#each $toasts as toast (toast.id)}
		<div class="toast toast--{toast.level}" role="alert">
			<span class="toast-msg">{toast.message}</span>
			<button class="toast-close" onclick={() => dismissToast(toast.id)} aria-label="Dismiss">✕</button>
		</div>
	{/each}
</div>

<style>
	.toast-stack {
		position: fixed;
		bottom: 16px;
		right: 16px;
		display: flex;
		flex-direction: column;
		gap: 8px;
		z-index: 9999;
		max-width: 380px;
		pointer-events: none;
	}

	.toast {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		padding: 10px 12px;
		border-radius: 6px;
		border: 1px solid;
		font-size: 0.82rem;
		line-height: 1.4;
		backdrop-filter: blur(4px);
		pointer-events: all;
		animation: slide-in 0.15s ease-out;
	}

	@keyframes slide-in {
		from { opacity: 0; transform: translateY(8px); }
		to   { opacity: 1; transform: translateY(0); }
	}

	.toast--error {
		background: #2a1010;
		border-color: #7f1d1d;
		color: #fca5a5;
	}

	.toast--warning {
		background: #1f1a08;
		border-color: #7c5c10;
		color: #fcd34d;
	}

	.toast--info {
		background: #0f1f30;
		border-color: #1e40af;
		color: #93c5fd;
	}

	.toast-msg {
		flex: 1;
		word-break: break-word;
	}

	.toast-close {
		background: none;
		border: none;
		color: inherit;
		opacity: 0.6;
		cursor: pointer;
		font-size: 0.72rem;
		padding: 0;
		flex-shrink: 0;
		line-height: 1;
	}

	.toast-close:hover {
		opacity: 1;
	}
</style>
