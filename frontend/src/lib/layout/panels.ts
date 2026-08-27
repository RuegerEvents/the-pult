/**
 * What can go in a tile.
 *
 * The schema stores panel ids as plain strings and knows nothing about what they
 * draw, which is deliberate: which panels a console has is a frontend question, and
 * a layout saved by a newer build should open on an older one with the panel it does
 * not recognise simply missing rather than breaking the tree.
 *
 * This is the one place that turns an id into a component. Adding a panel is a line
 * here and nothing anywhere else.
 */

import type { Component } from 'svelte';

import DevicesPanel from '$lib/components/DevicesPanel.svelte';
import FlowEditor from '$lib/components/flow/FlowEditor.svelte';
import OutputsPanel from '$lib/components/OutputsPanel.svelte';
import PatchPanel from '$lib/components/PatchPanel.svelte';
import SelectionPanel from '$lib/components/SelectionPanel.svelte';
import SequenceRunner from '$lib/components/SequenceRunner.svelte';
import SessionPanel from '$lib/components/SessionPanel.svelte';
import ShowPanel from '$lib/components/ShowPanel.svelte';
import StationsPanel from '$lib/components/StationsPanel.svelte';
import ValuesPanel from '$lib/components/programmer/ValuesPanel.svelte';
import PlanPanel from '$lib/components/stage/PlanPanel.svelte';
import RigPanel from '$lib/components/stage/RigPanel.svelte';

export type PanelId = keyof typeof PANELS;

export type PanelMeta = {
	title: string;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	component: Component<any>;
	/**
	 * True when the panel does its own scrolling and wants exactly the height it is
	 * given — a canvas, a plan, a graph. Everything else scrolls inside its tile.
	 */
	fills: boolean;
};

export const PANELS = {
	playback: { title: 'Playback', component: SequenceRunner, fills: false },
	values: { title: 'Programmer', component: ValuesPanel, fills: true },
	selection: { title: 'Selection', component: SelectionPanel, fills: true },
	plan: { title: 'Plan', component: PlanPanel, fills: true },
	rig: { title: '3D Rig', component: RigPanel, fills: true },
	patch: { title: 'Patch', component: PatchPanel, fills: false },
	flows: { title: 'Flows', component: FlowEditor, fills: true },
	outputs: { title: 'Outputs', component: OutputsPanel, fills: false },
	stations: { title: 'Stations', component: StationsPanel, fills: false },
	show: { title: 'Show', component: ShowPanel, fills: false },
	session: { title: 'Session', component: SessionPanel, fills: false },
	devices: { title: 'Devices', component: DevicesPanel, fills: false }
} as const satisfies Record<string, PanelMeta>;

export const isPanel = (id: string): id is PanelId => id in PANELS;

/** The panels, in menu order. */
export const PANEL_IDS = Object.keys(PANELS) as PanelId[];

export const panelTitle = (id: string): string => (isPanel(id) ? PANELS[id].title : id);
