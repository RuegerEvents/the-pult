/**
 * What can go in a tile.
 *
 * The schema stores panel ids as plain strings and knows nothing about what they
 * draw, which is deliberate: which panels a console has is a frontend question, and
 * a layout saved by a newer build should open on an older one with the panel it does
 * not recognize simply missing rather than breaking the tree.
 *
 * This is the one place that turns a *built-in* id into a component. Adding a
 * panel is a line here and nothing anywhere else. Panels contributed by the
 * station's plugins merge in beside these at runtime — `$lib/stores/plugins.ts`
 * derives them from the LOCAL `plugins` state under `plugin:*` ids, and the
 * workspace reads the merged `allPanels` store rather than this table.
 */

import type { Component } from 'svelte';

import Activity from '@lucide/svelte/icons/activity';
import ArrowLeftRight from '@lucide/svelte/icons/arrow-left-right';
import AudioWaveform from '@lucide/svelte/icons/audio-waveform';
import Box from '@lucide/svelte/icons/box';
import Cable from '@lucide/svelte/icons/cable';
import Cpu from '@lucide/svelte/icons/cpu';
import Drama from '@lucide/svelte/icons/drama';
import FileText from '@lucide/svelte/icons/file-text';
import Gauge from '@lucide/svelte/icons/gauge';
import Info from '@lucide/svelte/icons/info';
import Layers from '@lucide/svelte/icons/layers';
import LayoutGrid from '@lucide/svelte/icons/layout-grid';
import Lightbulb from '@lucide/svelte/icons/lightbulb';
import ListOrdered from '@lucide/svelte/icons/list-ordered';
import ListTree from '@lucide/svelte/icons/list-tree';
import MapIcon from '@lucide/svelte/icons/map';
import Network from '@lucide/svelte/icons/network';
import PanelRight from '@lucide/svelte/icons/panel-right';
import Play from '@lucide/svelte/icons/play';
import PlugZap from '@lucide/svelte/icons/plug-zap';
import Puzzle from '@lucide/svelte/icons/puzzle';
import RadioTower from '@lucide/svelte/icons/radio-tower';
import Router from '@lucide/svelte/icons/router';
import ScrollText from '@lucide/svelte/icons/scroll-text';
import Settings from '@lucide/svelte/icons/settings';
import Shapes from '@lucide/svelte/icons/shapes';
import Share2 from '@lucide/svelte/icons/share-2';
import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
import Smartphone from '@lucide/svelte/icons/smartphone';
import Sparkles from '@lucide/svelte/icons/sparkles';
import SquareMousePointer from '@lucide/svelte/icons/square-mouse-pointer';
import Table2 from '@lucide/svelte/icons/table-2';
import Undo2 from '@lucide/svelte/icons/undo-2';
import Users from '@lucide/svelte/icons/users';
import Variable from '@lucide/svelte/icons/variable';
import Workflow from '@lucide/svelte/icons/workflow';
import Wrench from '@lucide/svelte/icons/wrench';

import type { Category } from './categories.js';

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
import PoolsPanel from '$lib/components/pools/PoolsPanel.svelte';
import SequenceRunner from '$lib/components/SequenceRunner.svelte';
import CueSheet from '$lib/components/cues/CueSheet.svelte';
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
import VariablesPanel from '$lib/components/VariablesPanel.svelte';
import SurfacePanel from '$lib/components/surface/SurfacePanel.svelte';
import InspectorPanel from '$lib/components/inspector/InspectorPanel.svelte';

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
	 * Which family it belongs to, and so which color its tab, its picker entry and
	 * its palette row are drawn in. See `categories.ts` for why the color is the
	 * category's and not the panel's.
	 */
	category: Category;
	/**
	 * Its own mark: a Lucide icon component, drawn beside the title everywhere the
	 * panel is named. A test fails when a panel is added without one.
	 */
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	icon: Component<any>;
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
	playback: { title: 'Playback', component: SequenceRunner, fills: false, category: 'playback', icon: Play },
	// The other half of the pair, and the reason the runner lost its cue expander: this
	// is the *editor* of one sequence — numbers, both fade times, the curve, the follow —
	// where the runner is every sequence at once with a Go under each thumb. A click on
	// a row *shows* the cue in the fixture sheet; a double-click or the Go column takes.
	cues: { title: 'Cues', component: CueSheet, fills: true, category: 'playback', icon: ListOrdered },
	values: { title: 'Programmer', component: ValuesPanel, fills: true, category: 'program', icon: SlidersHorizontal },
	// The rig as a table, and the one panel that says *where* a value came from. It
	// fills, because a sheet handed half its tile is a sheet with three rows in it.
	sheet: { title: 'Fixtures', component: FixtureSheet, fills: true, category: 'program', icon: Table2 },
	selection: { title: 'Selection', component: SelectionPanel, fills: true, category: 'program', icon: SquareMousePointer },
	// Direct selects: the saved groups, and the presets. Not sequences — a pool of
	// those would be an executor page, which is a bound physical thing and its own
	// piece of work.
	pools: { title: 'Pools', component: PoolsPanel, fills: false, category: 'program', icon: LayoutGrid },
	plan: { title: 'Plan', component: PlanPanel, fills: true, category: 'stage', icon: MapIcon },
	rig: { title: '3D Rig', component: RigPanel, fills: true, category: 'stage', icon: Box },
	layers: { title: 'Layers', component: LayersPanel, fills: false, category: 'stage', icon: Layers },
	// The drawing itself, which the Layers panel only ever counted. A tree by parent,
	// because a truss run is a handle with its sections under it — and the one way to
	// reach a `Group`, which has no geometry and so cannot be clicked in the rig.
	objects: { title: 'Objects', component: ObjectsPanel, fills: false, category: 'stage', icon: ListTree },
	// The three the editor was built out of, and they are panels rather than sheets on
	// the rig's own toolbar for one reason: a strip that opens above the canvas pushes
	// the picture down, and the picture is what somebody is aiming a pointer at. What
	// stayed on that toolbar is only what is about *looking* — where the camera stands
	// and how the rig is drawn.
	pieces: { title: 'Pieces', component: PiecesPanel, fills: false, category: 'stage', icon: Shapes },
	tools: { title: 'Rig tools', component: ToolsPanel, fills: false, category: 'stage', icon: Wrench },
	object: { title: 'Object', component: ObjectPanel, fills: false, category: 'stage', icon: Info },
	patch: { title: 'Patch', component: PatchPanel, fills: false, category: 'setup', icon: Cable, home: 'both' },
	// The fixture types themselves, which the Patch panel used to carry above the
	// rig. Setup only: a type is made once and patched from all season, and it took
	// the top third of the one panel somebody actually patches in.
	fixturetypes: { title: 'Fixture types', component: FixtureTypeEditor, fills: false, category: 'setup', icon: Lightbulb, home: 'setup' },
	flows: { title: 'Flows', component: FlowEditor, fills: true, category: 'automation', icon: Workflow },
	// What the flows remember. A workspace panel rather than an errand: it is watched
	// and set during a show — whether area 2 is empty, a count put right by hand.
	variables: { title: 'Variables', component: VariablesPanel, fills: false, category: 'automation', icon: Variable },
	// The pages a phone runs the show from. A panel rather than an errand, because
	// locked it *is* the surface — a desk can press the same tiles — and `/surface/<id>`
	// is the same component with no workspace round it.
	surface: { title: 'Surfaces', component: SurfacePanel, fills: false, category: 'playback', icon: Smartphone },
	// Both directions in one panel: an input is an output read backwards and has the
	// same fields, so learning a second vocabulary for the same cable would be the
	// only thing separating them bought.
	outputs: { title: 'I/O', component: OutputsPanel, fills: false, category: 'setup', icon: PlugZap, home: 'both' },
	// A position, and what is written against it: events that Go cues, markers, and
	// the takes recorded off an input.
	timeline: { title: 'Timeline', component: TimelinePanel, fills: true, category: 'playback', icon: AudioWaveform },
	// Where an output is configured is the panel above; this is what it is actually
	// putting on the wire. Asked for while somebody is looking rather than published,
	// so a console with it shut costs the station nothing.
	wire: { title: 'On the wire', component: WirePanel, fills: false, category: 'system', icon: Activity },
	// Other people's software on the network, and the rigs it is offering. Its own
	// panel rather than a section of Rig tools: that is a strip of buttons, and this is
	// a live list of who is there and what they have.
	xchange: { title: 'MVR-xchange', component: XchangePanel, fills: false, category: 'setup', icon: Share2, home: 'both' },
	stations: { title: 'Stations', component: StationsPanel, fills: false, category: 'system', icon: RadioTower },
	// The other half of the pair: Stations is who is here, this is what it costs —
	// per station, per output connector, and per browser, which is the figure that
	// exists nowhere else because a console is a browser evaluating a rig.
	system: { title: 'System', component: SystemPanel, fills: false, category: 'system', icon: Cpu },
	// The third of that family, and the one that is a setting as well as a
	// diagnostic: which cable each service goes out on, and — because the question
	// "why can the previz not see us" is almost never asked at the broken console —
	// every station's interfaces and every station's faults.
	network: { title: 'Network', component: NetworkPanel, fills: false, category: 'setup', icon: Network, home: 'both' },
	plugins: { title: 'Plugins', component: PluginsPanel, fills: false, category: 'setup', icon: Puzzle, home: 'setup' },
	show: { title: 'Show', component: ShowPanel, fills: false, category: 'setup', icon: Drama, home: 'setup' },
	session: { title: 'Session', component: SessionPanel, fills: false, category: 'setup', icon: Users, home: 'setup' },
	devices: { title: 'Devices', component: DevicesPanel, fills: false, category: 'setup', icon: Router, home: 'both' },
	speedmasters: { title: 'Speed masters', component: SpeedMastersPanel, fills: false, category: 'program', icon: Gauge },
	effects: { title: 'Effects', component: EffectsPanel, fills: false, category: 'program', icon: Sparkles },
	// The sheets, and the one button that writes them. `fills`, because a sheet is a
	// piece of A3 and a tile that hands it half its height is a preview nobody can read.
	paperwork: { title: 'Paperwork', component: PaperworkPanel, fills: true, category: 'stage', icon: FileText },
	history: { title: 'History', component: HistoryPanel, fills: false, category: 'system', icon: Undo2 },
	// Not the History panel: that is the oplog, this is diagnostics. `fills`,
	// because a log wants every line it can get rather than a fixed block.
	logs: { title: 'System log', component: SystemLogPanel, fills: true, category: 'system', icon: ScrollText },
	settings: { title: 'Settings', component: SettingsPanel, fills: false, category: 'setup', icon: Settings, home: 'setup' },
	// What is selected, whichever panel it lives in: a preset, a group, an effect, a
	// surface. One editor for every object rather than a form under every list —
	// each panel contributes its own kinds through `$lib/objects.ts`.
	inspector: { title: 'Inspector', component: InspectorPanel, fills: false, category: 'program', icon: PanelRight }
} as const satisfies Record<string, PanelMeta>;

export const isPanel = (id: string): id is PanelId => id in PANELS;

/** The panels, in menu order. */
export const PANEL_IDS = Object.keys(PANELS) as PanelId[];

/** Where a panel belongs. A plugin's panel has no opinion and gets the workspace. */
export const panelHome = (meta: PanelMeta): 'workspace' | 'setup' | 'both' =>
	meta.home ?? 'workspace';

export const panelTitle = (id: string): string => (isPanel(id) ? PANELS[id].title : id);
