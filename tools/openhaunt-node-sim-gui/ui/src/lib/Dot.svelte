<script lang="ts">
	let { on, label, tone = 'live' }: { on: boolean; label: string; tone?: 'live' | 'warn' } =
		$props();
</script>

<span class="dot-label" class:on class:warn={tone === 'warn'}>
	<span class="dot"></span>{label}
</span>

<style>
	.dot-label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 0.75rem;
		color: var(--dim);
		white-space: nowrap;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: #3a423d;
		transition: background 0.25s;
	}

	.on .dot {
		background: var(--live);
	}

	.on.warn .dot {
		background: var(--warn);
	}

	.on {
		color: var(--text);
	}
</style>
