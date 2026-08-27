<script lang="ts">
	import type { Frame } from './node.js';

	let { frames }: { frames: Frame[] } = $props();
</script>

<section>
	<h2>sACN</h2>
	{#if frames.length === 0}
		<p class="note">Nothing on the wire yet.</p>
	{/if}
	{#each frames as frame (frame.universe)}
		{@const lit = frame.channels.filter((c) => c > 0).length}
		<div class="universe">
			<div class="label">
				<span class="mono">Universe {frame.universe}</span>
				<span class="lit">{lit} of 512 above zero</span>
			</div>
			<!-- All 512, as one bar each. A gateway forwards a whole universe, so the
			     honest picture is the whole universe rather than the first few. -->
			<div class="channels">
				{#each frame.channels as level, channel (channel)}
					<i style:height="{(level / 255) * 100}%" title="{channel + 1}: {level}"></i>
				{/each}
			</div>
		</div>
	{/each}
</section>

<style>
	section {
		padding: 16px 20px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.universe {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.label {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
	}

	.lit,
	.note {
		font-size: 0.75rem;
		color: var(--dim);
	}

	.channels {
		display: flex;
		align-items: flex-end;
		gap: 1px;
		height: 72px;
		padding: 4px;
		background: #12160f;
		border: 1px solid var(--line);
		border-radius: 4px;
		overflow-x: auto;
	}

	i {
		flex: 1 0 2px;
		min-height: 1px;
		background: var(--live);
		opacity: 0.85;
	}
</style>
