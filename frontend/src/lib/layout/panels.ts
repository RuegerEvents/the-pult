/**
 * What can go in a tile.
 *
 * The schema stores panel ids as plain strings and knows nothing about what they
 * draw, which is deliberate: which panels a console has is a frontend question, and
 * a layout saved by a newer build should open on an older one with the panel it does
 * not recognise simply missing rather than breaking the tree.
 *
 * This is the one place that turns a *built-in* id into a component. Adding a
 * panel is a line here and nothing anywhere else. Panels contributed by the
 * station's plugins merge in beside these at runtime — `$lib/stores/plugins.ts`
 * derives them from the LOCAL `plugins` state under `plugin:*` ids, and the
 * workspace reads the merged `allPanels` store rather than this table.
 */

import type { Component } from 'svelte';

import DevicesPanel from '$lib/components/DevicesPanel.svelte';
import EffectsPanel from '$lib/components/effects/EffectsPanel.svelte';
import FlowEditor from '$lib/components/flow/FlowEditor.svelte';
import HistoryPanel from '$lib/components/HistoryPanel.svelte';
import SystemLogPanel from '$lib/components/SystemLogPanel.svelte';
import OutputsPanel from '$lib/components/OutputsPanel.svelte';
import PatchPanel from '$lib/components/PatchPanel.svelte';
import FixtureTypeEditor from '$lib/components/FixtureTypeEditor.svelte';
import PluginsPanel from '$lib/components/PluginsPanel.svelte';
import SelectionPanel from '$lib/components/SelectionPanel.svelte';
import SequenceRunner from '$lib/components/SequenceRunner.svelte';
import SessionPanel from '$lib/components/SessionPanel.svelte';
import SettingsPanel from '$lib/components/SettingsPanel.svelte';
import SpeedMastersPanel from '$lib/components/SpeedMastersPanel.svelte';
import ShowPanel from '$lib/components/ShowPanel.svelte';
import NetworkPanel from '$lib/components/NetworkPanel.svelte';
import StationsPanel from '$lib/components/StationsPanel.svelte';
import SystemPanel from '$lib/components/SystemPanel.svelte';
import ValuesPanel from '$lib/components/programmer/ValuesPanel.svelte';
import FixtureSheet from '$lib/components/programmer/FixtureSheet.svelte';
import WirePanel from '$lib/components/wire/WirePanel.svelte';
import XchangePanel from '$lib/components/XchangePanel.svelte';
import PlanPanel from '$lib/components/stage/PlanPanel.svelte';
import RigPanel from '$lib/components/stage/RigPanel.svelte';
import LayersPanel from '$lib/components/stage/LayersPanel.svelte';
import ObjectsPanel from '$lib/components/stage/ObjectsPanel.svelte';
import ObjectPanel from '$lib/components/stage/ObjectPanel.svelte';
import PiecesPanel from '$lib/components/stage/PiecesPanel.svelte';
import ToolsPanel from '$lib/components/stage/ToolsPanel.svelte';
import PaperworkPanel from '$lib/components/paperwork/PaperworkPanel.svelte';
import TimelinePanel from '$lib/components/timeline/TimelinePanel.svelte';

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
	/**
	 * True when this panel can change the show and so opens read-only, with an
	 * Edit toggle in the tile chrome.
	 *
	 * Not every panel that writes wants one. The programmer writes constantly and is
	 * the whole point of being at the console; an effects editor is an editor. What
	 * this marks is the panels where a mis-hit is expensive and rare: unpatching a
	 * fixture, forgetting a device, renaming a flow mid-show.
	 */
	editable?: boolean;
	/**
	 * Handed to the component when the tile renders it. Built-in panels take no
	 * props; a plugin panel needs to know which plugin it fronts, and this is
	 * how the registry entry carries that without the tile knowing either way.
	 */
	props?: Record<string, unknown>;
	/**
	 * Where this panel belongs: in the workspace, in the setup dialog, or in both.
	 *
	 * The distinction is not "does it write the show" — the programmer writes
	 * constantly. It is whether somebody *keeps it open*. A fixture type is made
	 * once and then patched from for a season; a plugin is installed once. Those
	 * are errands, and an errand that costs a tile is an errand that costs the
	 * picture somebody is programming against. What stays a panel stays for a
	 * reason written beside it: a sparkline is only a record because the panel
	 * witnessed it, a log subscribes while it is mounted, a wire view *is* an
	 * `output.watch`.
	 *
	 * Missing means `workspace`, which is what nearly all of them are.
	 */
	home?: 'workspace' | 'setup' | 'both';
};

export const PANELS = {
	playback: { title: 'Playback', component: SequenceRunner, fills: false, editable: true },
	values: { title: 'Programmer', component: ValuesPanel, fills: true },
	// The rig as a table, and the one panel that says *where* a value came from. It
	// fills, because a sheet handed half its tile is a sheet with three rows in it.
	sheet: { title: 'Fixtures', component: FixtureSheet, fills: true },
	selection: { title: 'Selection', component: SelectionPanel, fills: true },
	plan: { title: 'Plan', component: PlanPanel, fills: true, editable: true },
	rig: { title: '3D Rig', component: RigPanel, fills: true },
	layers: { title: 'Layers', component: LayersPanel, fills: false, editable: true },
	// The drawing itself, which the Layers panel only ever counted. A tree by parent,
	// because a truss run is a handle with its sections under it — and the one way to
	// reach a `Group`, which has no geometry and so cannot be clicked in the rig.
	objects: { title: 'Objects', component: ObjectsPanel, fills: false, editable: true },
	// The three the editor was built out of, and they are panels rather than sheets on
	// the rig's own toolbar for one reason: a strip that opens above the canvas pushes
	// the picture down, and the picture is what somebody is aiming a pointer at. What
	// stayed on that toolbar is only what is about *looking* — where the camera stands
	// and how the rig is drawn.
	pieces: { title: 'Pieces', component: PiecesPanel, fills: false },
	tools: { title: 'Rig tools', component: ToolsPanel, fills: false },
	object: { title: 'Object', component: ObjectPanel, fills: false, editable: true },
	patch: { title: 'Patch', component: PatchPanel, fills: false, editable: true, home: 'both' },
	// The fixture types themselves, which the Patch panel used to carry above the
	// rig. Setup only: a type is made once and patched from all season, and it took
	// the top third of the one panel somebody actually patches in.
	fixturetypes: { title: 'Fixture types', component: FixtureTypeEditor, fills: false, home: 'setup' },
	flows: { title: 'Flows', component: FlowEditor, fills: true, editable: true },
	// Both directions in one panel: an input is an output read backwards and has the
	// same fields, so learning a second vocabulary for the same cable would be the
	// only thing separating them bought.
	outputs: { title: 'I/O', component: OutputsPanel, fills: false, home: 'both' },
	// A position, and what is written against it: events that Go cues, markers, and
	// the takes recorded off an input.
	timeline: { title: 'Timeline', component: TimelinePanel, fills: false },
	// Where an output is configured is the panel above; this is what it is actually
	// putting on the wire. Asked for while somebody is looking rather than published,
	// so a console with it shut costs the station nothing.
	wire: { title: 'On the wire', component: WirePanel, fills: false },
	// Other people's software on the network, and the rigs it is offering. Its own
	// panel rather than a section of Rig tools: that is a strip of buttons, and this is
	// a live list of who is there and what they have.
	xchange: { title: 'MVR-xchange', component: XchangePanel, fills: false, home: 'both' },
	stations: { title: 'Stations', component: StationsPanel, fills: false },
	// The other half of the pair: Stations is who is here, this is what it costs —
	// per station, per output connector, and per browser, which is the figure that
	// exists nowhere else because a console is a browser evaluating a rig.
	system: { title: 'System', component: SystemPanel, fills: false },
	// The third of that family, and the one that is a setting as well as a
	// diagnostic: which cable each service goes out on, and — because the question
	// "why can the previz not see us" is almost never asked at the broken console —
	// every station's interfaces and every station's faults.
	network: { title: 'Network', component: NetworkPanel, fills: false, editable: true, home: 'both' },
	plugins: { title: 'Plugins', component: PluginsPanel, fills: false, editable: true, home: 'setup' },
	show: { title: 'Show', component: ShowPanel, fills: false, home: 'setup' },
	session: { title: 'Session', component: SessionPanel, fills: false, home: 'setup' },
	devices: { title: 'Devices', component: DevicesPanel, fills: false, editable: true, home: 'both' },
	speedmasters: { title: 'Speed masters', component: SpeedMastersPanel, fills: false, editable: true },
	// No edit toggle: this panel is an editor, and it writes to the programmer
	// rather than to the show.
	effects: { title: 'Effects', component: EffectsPanel, fills: false },
	// The sheets, and the one button that writes them. `fills`, because a sheet is a
	// piece of A3 and a tile that hands it half its height is a preview nobody can read.
	paperwork: { title: 'Paperwork', component: PaperworkPanel, fills: true },
	history: { title: 'History', component: HistoryPanel, fills: false },
	// Not the History panel: that is the oplog, this is diagnostics. `fills`,
	// because a log wants every line it can get rather than a fixed block.
	logs: { title: 'System log', component: SystemLogPanel, fills: true },
	settings: { title: 'Settings', component: SettingsPanel, fills: false, editable: true, home: 'setup' }
} as const satisfies Record<string, PanelMeta>;

export const isPanel = (id: string): id is PanelId => id in PANELS;

/** The panels, in menu order. */
export const PANEL_IDS = Object.keys(PANELS) as PanelId[];

/** Where a panel belongs. A plugin's panel has no opinion and gets the workspace. */
export const panelHome = (meta: PanelMeta): 'workspace' | 'setup' | 'both' =>
	meta.home ?? 'workspace';

export const panelTitle = (id: string): string => (isPanel(id) ? PANELS[id].title : id);
