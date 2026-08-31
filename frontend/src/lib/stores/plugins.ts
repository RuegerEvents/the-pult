/**
 * What the station's plugin runtime is running, and the panels that follow.
 *
 * `plugins` is LOCAL state — a fact about the station this browser is talking
 * to, not about the show — so it arrives the way `session` and `devices` do:
 * subscribed straight off the socket rather than through a collection.
 *
 * From it falls the second half of the panel registry: a plugin declaring a
 * surface (a built-in component this frontend supplies) or a web-component
 * panel (a script the plugin ships) becomes an entry beside the built-in
 * panels, under an id no built-in will ever collide with:
 * `plugin:<plugin-id>:<surface-or-panel-id>`.
 */

import { derived, readable, type Readable } from 'svelte/store';

import type { PluginsState } from '$lib/generated/index.js';
import { PANELS, type PanelMeta } from '$lib/layout/panels.js';
import { showClient } from './show.js';

import BarSurface from '$lib/components/plugins/BarSurface.svelte';
import ConsoleSurface from '$lib/components/plugins/ConsoleSurface.svelte';
import WebComponentPanel from '$lib/components/plugins/WebComponentPanel.svelte';

const EMPTY: PluginsState = { plugins: [] };

/** The runtime's report, live. */
export const pluginsState: Readable<PluginsState> = readable(EMPTY, (set) => {
	// First subscription happens from a component, by which time +layout.svelte
	// has long since pointed the show stores at the socket.
	const client = showClient();
	const apply = (value: unknown) => {
		if (value && typeof value === 'object') set(value as PluginsState);
	};
	const unsub = client.subscribe('plugins', apply);
	const refetch = () => void client.get(['plugins']).then(apply);
	refetch();
	const forget = client.addConnectListener(refetch);
	return () => {
		unsub();
		forget();
	};
});

/** The workspace panel id a plugin surface or panel opens under. */
export const pluginPanelId = (plugin: string, id: string): string => `plugin:${plugin}:${id}`;

/**
 * The panels plugins contribute right now. Failed plugins keep their entries —
 * the tile renders the failure instead of the surface, which is where an
 * operator actually looks for it.
 */
export const pluginPanels: Readable<Record<string, PanelMeta>> = derived(
	pluginsState,
	($state) => {
		const panels: Record<string, PanelMeta> = {};
		for (const plugin of $state.plugins) {
			for (const surface of plugin.surfaces) {
				panels[pluginPanelId(plugin.id, surface.id)] = {
					title: surface.title,
					component: surface.kind === 'bar' ? BarSurface : ConsoleSurface,
					fills: true,
					props: { pluginId: plugin.id, surfaceId: surface.id, status: plugin.status }
				};
			}
			for (const panel of plugin.panels) {
				panels[pluginPanelId(plugin.id, panel.id)] = {
					title: panel.title,
					component: WebComponentPanel,
					fills: panel.fills,
					props: {
						pluginId: plugin.id,
						element: panel.element,
						script: panel.script,
						status: plugin.status
					}
				};
			}
		}
		return panels;
	}
);

/** Every panel this console can draw right now: the built-ins, then plugins. */
export const allPanels: Readable<Record<string, PanelMeta>> = derived(
	pluginPanels,
	($plugins) => ({ ...PANELS, ...$plugins }) as Record<string, PanelMeta>
);

// ── The console focus key ─────────────────────────────────────────────────────

/**
 * Wherever a console surface is open, Ctrl/Cmd+K should land in its prompt.
 * The surface registers its focus function while mounted; the keymap calls
 * whoever registered last, which is the console most recently opened.
 */
let focusConsoleInput: (() => void) | null = null;

export function registerConsoleFocus(focus: () => void): () => void {
	focusConsoleInput = focus;
	return () => {
		if (focusConsoleInput === focus) focusConsoleInput = null;
	};
}

/** Focus an open console surface. False when none is mounted. */
export function focusConsole(): boolean {
	if (!focusConsoleInput) return false;
	focusConsoleInput();
	return true;
}
