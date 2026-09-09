<script lang="ts">
	/**
	 * Everything that is done once.
	 *
	 * Patching a rig, teaching the console a fixture type, naming which cable a
	 * service goes out on, installing a plugin: each of those is an errand, and an
	 * errand that costs a tile costs the picture somebody is programming against. So
	 * they are sections of one full-screen dialog rather than panels competing for
	 * the workspace, which is what every other desk does with the word *Setup*.
	 *
	 * Each section is the panel component **unchanged**. That is the whole trick and
	 * the reason this is cheap: the Patch panel is the same component whether it is
	 * in a tile or in here, so nothing is written twice and nothing can drift.
	 *
	 * A panel with `home: 'both'` — patch, devices, I/O, network, MVR-xchange — is
	 * here *and* still available to the workspace, because each is a thing somebody
	 * sometimes keeps open: a patch beside the rig while a rig is being built, an
	 * I/O panel beside the wire viewer while a cable is being chased.
	 */

	import Dialog from '$lib/components/Dialog.svelte';
	import { PANELS, panelHome, type PanelId } from '$lib/layout/panels.js';
	import { closeSetup, openSetup, setupSection } from '$lib/stores/setup.js';

	/**
	 * The sections, in the order somebody sets a console up in: what the rig is,
	 * then what it is plugged into, then who else is on it, then the show itself.
	 */
	const SECTIONS: PanelId[] = (
		['patch', 'fixturetypes', 'devices', 'outputs', 'network', 'session', 'plugins', 'xchange', 'show', 'settings'] as PanelId[]
	).filter((id) => panelHome(PANELS[id]) !== 'workspace');

	const current = $derived(
		($setupSection && SECTIONS.includes($setupSection as PanelId)
			? ($setupSection as PanelId)
			: SECTIONS[0]) as PanelId
	);
	const Section = $derived(PANELS[current].component);
</script>

<Dialog title="Setup" size="full" onclose={closeSetup}>
	<div class="setup">
		<nav>
			{#each SECTIONS as id (id)}
				<button class="section" class:on={id === current} onclick={() => openSetup(id)}>
					{PANELS[id].title}
				</button>
			{/each}
		</nav>
		<div class="pane">
			<!-- Keyed on the section, so leaving one and coming back mounts it fresh
			     rather than showing a half-finished form from ten minutes ago. -->
			{#key current}
				<Section />
			{/key}
		</div>
	</div>
</Dialog>

<style>
	.setup {
		flex: 1;
		min-height: 0;
		display: grid;
		grid-template-columns: 180px 1fr;
	}

	nav {
		display: flex;
		flex-direction: column;
		gap: 1px;
		padding: 8px;
		border-right: 1px solid var(--line);
		background: var(--bg-sunken);
		overflow-y: auto;
	}

	.section {
		text-align: left;
		background: none;
		border: none;
		border-radius: var(--radius);
		color: var(--text-dim);
		font: inherit;
		font-size: var(--font-sm);
		padding: 8px 10px;
		cursor: pointer;
	}
	.section:hover {
		background: var(--bg-hover);
		color: var(--text);
	}
	.section.on {
		background: var(--bg-raised);
		color: var(--text-bright);
	}

	.pane {
		min-width: 0;
		overflow: auto;
		padding: 12px;
	}
</style>
