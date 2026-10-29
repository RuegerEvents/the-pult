<script lang="ts">
	/**
	 * On or off: a relay, a dry contact the console closes.
	 *
	 * A switch says which it is by where its knob is — right and lit is on — so it
	 * carries no word beside it repeating that.
	 *
	 * With `flash`, a button beside it is on for exactly as long as it is held and off
	 * when it is let go — for trying a relay while programming, where a switch means
	 * two touches and a forgotten second one leaves the smoke machine running.
	 */

	let {
		value,
		oninput,
		label = 'On',
		flash = false
	}: {
		value: boolean;
		oninput: (value: boolean) => void;
		label?: string;
		flash?: boolean;
	} = $props();

	let held = $state(false);

	function press() {
		if (held) return;
		held = true;
		oninput(true);
	}

	// Called from every way a press can end — up, cancel, a lost capture, the focus
	// going — so it has to be safe to call twice and only the first one writes.
	function letGo() {
		if (!held) return;
		held = false;
		oninput(false);
	}
</script>

<span class="pair">

<button
	type="button"
	class="toggle hit"
	class:on={value}
	role="switch"
	aria-checked={value}
	aria-label={label}
	title={value ? 'On' : 'Off'}
	onclick={() => oninput(!value)}
>
	<span class="knob"></span>
</button>
{#if flash}
	<button
		type="button"
		class="flash hit"
		class:held
		aria-label="Flash {label}"
		title="On while held"
		onpointerdown={(e) => {
			if (e.button !== 0) return;
			e.currentTarget.setPointerCapture(e.pointerId);
			press();
		}}
		onpointerup={letGo}
		onpointercancel={letGo}
		onlostpointercapture={letGo}
		onblur={letGo}
		onkeydown={(e) => {
			if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) {
				e.preventDefault();
				press();
			}
		}}
		onkeyup={(e) => {
			if (e.key === ' ' || e.key === 'Enter') letGo();
		}}
		oncontextmenu={(e) => e.preventDefault()}
	>
		Flash
	</button>
{/if}
</span>

<style>
	.pair {
		display: inline-flex;
		align-items: center;
		gap: 8px;
	}

	.flash {
		flex: none;
		height: 22px;
		padding: 0 10px;
		border: 1px solid var(--line-strong);
		border-radius: 4px;
		background: var(--bg-sunken);
		color: inherit;
		font: inherit;
		font-size: 11px;
		cursor: pointer;
		/* A held finger must not scroll the panel, select the word or open a menu. */
		touch-action: none;
		user-select: none;
		-webkit-user-select: none;
		-webkit-touch-callout: none;
	}
	.flash.held {
		border-color: var(--live);
		background: color-mix(in srgb, var(--live) 35%, var(--bg-sunken));
	}

	.toggle {
		position: relative;
		flex: none;
		width: 40px;
		height: 22px;
		padding: 0;
		border: 1px solid var(--line-strong);
		border-radius: 11px;
		background: var(--bg-sunken);
		cursor: pointer;
		transition:
			background 0.12s,
			border-color 0.12s;
	}
	.toggle.on {
		border-color: var(--live);
		background: color-mix(in srgb, var(--live) 35%, var(--bg-sunken));
	}

	.knob {
		position: absolute;
		top: 2px;
		left: 2px;
		width: 16px;
		height: 16px;
		border-radius: 50%;
		background: var(--line-input);
		transition:
			transform 0.12s,
			background 0.12s;
	}
	.toggle.on .knob {
		transform: translateX(18px);
		background: var(--live);
	}
</style>
