<script lang="ts">
	/**
	 * A shape this build has never heard of: an older console reading a newer
	 * station, or a connector that arrived with a plugin.
	 *
	 * Drawn as itself rather than left out. A panel that quietly omitted the one
	 * section it did not recognise would be describing the wire wrongly with no sign
	 * that it was doing so, which is worse than a block of JSON.
	 */

	const { shape, of }: { shape: string; of: unknown } = $props();
</script>

<p class="note">
	This console does not know how to draw <code>{shape}</code>, so here it is as it arrived.
</p>
<pre>{JSON.stringify(of, null, 2)}</pre>

<style>
	.note { color: #c9a227; font-size: 12px; margin: 0 0 6px; }
	code { color: #ddd; }
	pre {
		font-size: 11px; color: #999; background: #141414; border: 1px solid #2a2a2a;
		border-radius: 3px; padding: 8px; max-height: 300px; overflow: auto; margin: 0;
	}
</style>
