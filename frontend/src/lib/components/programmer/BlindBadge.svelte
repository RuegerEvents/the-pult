<script lang="ts">
	/**
	 * The strip a view carries while the show is blind.
	 *
	 * The rig, the plan and the Fixtures sheet go on showing the programmer while blind,
	 * which is the point of programming blind with a picture in front of you — and also
	 * the one way these views can show something the room cannot see. So they say so,
	 * on the view itself rather than only on the button that switched it.
	 */
	import { blind } from '$lib/stores/programmer.js';
</script>

{#if $blind}
	<div class="blind" role="status">
		<b>BLIND</b> — this shows the programmer; the stage is showing playback
	</div>
{/if}

<style>
	.blind {
		flex: none;
		padding: 3px 10px;
		background: color-mix(in srgb, var(--bad, #ef4444) 22%, transparent);
		border-bottom: 1px solid color-mix(in srgb, var(--bad, #ef4444) 55%, transparent);
		color: var(--text-bright, #fff);
		font-size: var(--font-xs, 11px);
		text-align: center;
	}
	b {
		letter-spacing: 0.08em;
	}
</style>
