/**
 * The command wing, shown on this page.
 *
 * The wing is plugged into a station and **that station runs it** — every key, fader,
 * encoder and light, whether or not a page is open (`infra/wing_control/`). It used to
 * be this page's: the station relayed what was pressed and the page did the
 * programming, so a page busy drawing the rig lagged every key behind it and a wing
 * with no page open did nothing at all. Found at the gig, and moved.
 *
 * What is left here is what needs a screen. A page that claims the wing:
 *
 * - **shares its selection with it** (`wing.context`), so the encoders turn what the
 *   operator clicked, and takes on a selection made on the wing — a group key, a typed
 *   `fixture 3` — as its own;
 * - **shows** the line, the encoder bar and the calculator, from the station's LOCAL
 *   `wing` row;
 * - **answers the asks** on `wing_asks`: a toast, the Store dialog, a learned control
 *   to bind, ESC and PLEASE for the dialog on top.
 *
 * Whether this page shows the wing is remembered in `localStorage`, so a reload or a
 * reconnect claims it again. Claiming takes it from any other page: the operator
 * standing at the wing is the one who asked.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import { focusedCueSheet } from './cues.js';
import { onOffer } from './learn.js';
import { dialogCount, topDialog } from './dialogs.js';
import { output } from './output.js';
import { openConsoleLine } from './plugins.js';
import { byKey } from './programmer.js';
import { clearSelection, query, recall, selection, setQuery, targetElements } from './selection.js';
import { collection, showClient, showData } from './show.js';
import { userId } from './user.js';
import { addToast } from '../toasts.js';
import { PultWsClient } from '../ws/client.js';
import { WS_PATH, wsUrlFrom } from '../ws/endpoint.js';
import type {
	BindingAction,
	ControlBinding,
	ControlPage,
	ParameterKind,
	ParameterValue,
	StoreOnto,
	SurfaceControl,
	WingAsk,
	WingOperator,
	WingState
} from '../generated/index.js';
import { displayLabel, modeHas, modeOf, parameterKey } from '../patch.js';
import { rowTargets, type Target } from '../programmer.js';
import { idsQuery } from '../selection.js';
import { learnable, type EncoderSlot, type WingMap } from '../wing.js';
import { uuid } from '$lib/uuid.js';

/** The surface kind this page binds under — `pult_schema::types::wing::COMMAND_WING`. */
export const COMMAND_WING = 'ma3-command-wing';

const OPERATOR: WingOperator = {
	line: '',
	calculator: null,
	encoders: { page: 0, pages: 1, slots: [null, null, null, null, null] },
	learning: false,
	selection: { fixtures: [], cells: [], query: null, rev: 0, from_wing: false },
	go_sequence: null,
	page: 1
};
const EMPTY: WingState = { phase: { kind: 'Off' }, device: null, claimed_by: null, digital: null, operator: OPERATOR };
const DRIVE_KEY = 'pult.wing.drive';
const AT_KEY = 'pult.wing.at';

// ── What is known ─────────────────────────────────────────────────────────────

/** The station's `wing` row: whether a wing is here, who has it, what it is doing. */
export const wingState = writable<WingState>(EMPTY);

/** Whether this page means to show the wing. Remembered across reloads. */
export const drivesHere = writable<boolean>(
	typeof localStorage !== 'undefined' && localStorage.getItem(DRIVE_KEY) === '1'
);

/** The key this page's claim landed under, which is how it knows the row means it. */
const claimedAs = writable<string | null>(null);

/** True while this page is the one showing the wing. */
export const driving: Readable<boolean> = derived(
	[wingState, drivesHere, claimedAs],
	([$state, $drives, $as]) => $drives && $as !== null && $state.claimed_by === $as
);

/** MA's names for every control, from the station's grandMA3 install. */
export const wingMap = writable<WingMap | null>(null);

const operator: Readable<WingOperator> = derived(wingState, ($state) => $state.operator ?? OPERATOR);

/** Waiting for the next control touched on the wing, to bind it. The station's. */
export const learning: Readable<boolean> = derived(operator, ($op) => $op.learning);

/** Start or stop learning. */
export function setLearning(on: boolean): void {
	void wingClient()
		.call('wing.learn', { on })
		.catch((e) => addToast(`The wing: ${e instanceof Error ? e.message : e}`));
}

/** The control learning caught, for the panel to open its editor on. This page's. */
export const learned = writable<SurfaceControl | null>(null);

/** What the wing has typed and PLEASE has not run. */
export const wingLine: Readable<string> = derived(operator, ($op) => $op.line);

/** The calculator a large encoder's press opened, while one is. */
export const calculator: Readable<{ slot: number; text: string } | null> = derived(operator, ($op) => $op.calculator);

/** The sequence the wing's GO goes on, for the panel to say so. */
export const wingGoSequence: Readable<string | null> = derived(operator, ($op) => $op.go_sequence);

/** The encoder bar as the station worked it out, labeled for reading. */
export const encoderBar: Readable<{ slots: (EncoderSlot | null)[]; page: number; pages: number }> = derived(
	operator,
	($op) => ({
		page: $op.encoders.page,
		pages: $op.encoders.pages,
		slots: $op.encoders.slots.map((slot) => {
			if (!slot) return null;
			const component = slot.component ?? undefined;
			const label = component ? { r: 'Red', g: 'Green', b: 'Blue' }[component] : displayLabel(slot.kind);
			return { kind: slot.kind, key: parameterKey(slot.kind), label, component, defaultValue: slot.default_value };
		})
	})
);

/** The page of bindings the wing's controls act on, from 1. The station's. */
export const wingPage: Readable<number> = derived(operator, ($op) => $op.page ?? 1);

/**
 * Asked of the page by the wing: open the Store dialog with these entries ticked, and
 * onto a playback control where STORE was followed by one.
 */
export const wingStoreAsk = writable<{ preselect: string[]; onto: StoreOnto | null } | null>(null);

/** How many Store dialogs are open on this page. While any is, a playback key on the wing says where to store. */
export const storeDialogs = writable(0);

/** A playback key pressed on the wing while the Store dialog was open: store onto that. */
export const wingStoreTarget = writable<StoreOnto | null>(null);

/**
 * EDIT and then a control on the wing: what to edit — the cue on it, its sequence, or
 * what the control does. `WingEdit.svelte` asks.
 */
export const wingEditAsk = writable<{
	control: SurfaceControl;
	page: number;
	sequenceId: string | null;
	cueId: string | null;
} | null>(null);

// ── The show ──────────────────────────────────────────────────────────────────

const fixtures = collection('fixtures');
const types = collection('fixture_types');
const bindings = collection('control_bindings');
const pageRows = collection('control_pages');

/** Every binding of this wing's, on every page. */
export const allWingBindings: Readable<ControlBinding[]> = derived(bindings, ($bindings) =>
	$bindings.filter((b) => b.surface === COMMAND_WING)
);

/** What each named page is called, by number. */
export const pageNames: Readable<Map<number, string>> = derived(pageRows, ($rows) => {
	const out = new Map<number, string>();
	for (const row of $rows as ControlPage[]) if (row.surface === COMMAND_WING && row.name) out.set(row.number, row.name);
	return out;
});

/**
 * The pages worth a tab: every page something is bound or named on, the page the wing
 * is on, and one empty page past the last, which is where the next thing is learned.
 */
export const wingPages: Readable<number[]> = derived(
	[allWingBindings, pageNames, wingPage],
	([$bindings, $names, $page]) => {
		const used = new Set<number>([1, $page, ...$names.keys(), ...$bindings.map((b) => b.page ?? 1)]);
		used.add(Math.max(...used) + 1);
		return [...used].sort((a, b) => a - b);
	}
);

/** Name a page, or take its name away with an empty one. */
export async function renamePage(number: number, name: string): Promise<void> {
	const data = showData();
	const row = (get(pageRows) as ControlPage[]).find((r) => r.surface === COMMAND_WING && r.number === number);
	const trimmed = name.trim();
	if (row && !trimmed) await data.control_pages.byId(row.id).delete();
	else if (row) await data.control_pages.byId(row.id).name.set(trimmed);
	else if (trimmed) await data.control_pages.create({ id: uuid(), surface: COMMAND_WING, number, name: trimmed });
}

/** This wing's bindings on the page it is on, by control. A binding with no page is on page 1. */
export const wingBindings: Readable<Map<string, ControlBinding>> = derived(
	[bindings, wingPage],
	([$bindings, $page]) => {
		const out = new Map<string, ControlBinding>();
		for (const b of $bindings) {
			if (b.surface === COMMAND_WING && (b.page ?? 1) === $page) out.set(controlKey(b.control), b);
		}
		return out;
	}
);

export const controlKey = (c: SurfaceControl): string => `${c.kind}:${c.index}`;

const typeOf = derived([fixtures, types], ([$fixtures, $types]) => {
	const byId = new Map($types.map((t) => [t.id, t]));
	return new Map($fixtures.map((f) => [f.id, { fixture: f, type: byId.get(f.fixture_type_id) }]));
});

/** Where a value set on the selection lands — the values panel's rule, asked the same way. */
function selectionTargets(kind: ParameterKind): Target[] {
	const known = get(typeOf);
	const reaches = (id: string, key: string) => {
		const row = known.get(id);
		return !row || modeHas(modeOf(row.type, row.fixture.address), key);
	};
	return rowTargets(kind, get(targetElements), (id) => known.get(id)?.type, reaches);
}

// ── Starting ──────────────────────────────────────────────────────────────────

/**
 * Which station's wing this page follows: its own (`null`), or another's by node id.
 *
 * A wing is plugged into one station, but the page showing it can have come from any
 * of them — the operator's screen is on the desk and the wing is on a Pi under it. So
 * the wing is followed over a socket of its own to the station it is plugged into.
 */
export const wingAt = writable<string | null>(
	(typeof localStorage !== 'undefined' && localStorage.getItem(AT_KEY)) || null
);

/** This page's own station, by node id. */
export const thisStation = writable<string | null>(null);

const stations = collection('stations');

/** A station a wing is plugged into, as the panel offers it. */
export type WingStationRow = { id: string; hostname: string; state: WingState; here: boolean };

/**
 * Every station that was started with a wing, this page's own first. From the
 * replicated `stations` rows, which is the only way a page can know about a wing on a
 * station it did not come from.
 */
export const wingStations: Readable<WingStationRow[]> = derived(
	[stations, thisStation],
	([$stations, $here]) =>
		$stations
			.filter((s) => s.wing && s.wing.phase.kind !== 'Off')
			.map((s) => ({ id: s.id, hostname: s.hostname, state: s.wing as WingState, here: s.id === $here }))
			.sort((a, b) => Number(b.here) - Number(a.here) || a.hostname.localeCompare(b.hostname))
);

/** Follow the wing on this station from now on: `null` for this page's own. */
export function setWingAt(id: string | null): void {
	const own = id !== null && id === get(thisStation) ? null : id;
	wingAt.set(own);
	if (typeof localStorage !== 'undefined') {
		if (own) localStorage.setItem(AT_KEY, own);
		else localStorage.removeItem(AT_KEY);
	}
}

/** The socket the wing is followed over: this page's own, or one to its station. */
let current: PultWsClient | null = null;
const wingClient = (): PultWsClient => current ?? showClient();

let started = false;

/**
 * Follow the wing from this page. Called once, from the layout, after the show stores
 * have a socket. Costs one small LOCAL subscription on a page that never shows it.
 */
export function startWing(): void {
	if (started || typeof window === 'undefined') return;
	started = true;
	const page = showClient();
	// Read as well as followed: a subscription hears changes, and this page's own
	// station is the one thing about the session that never changes — so without the
	// read the page did not know which station was its own, and offered it as another.
	const own = (v: unknown) => {
		if (v && typeof v === 'object') thisStation.set((v as { node_id: string | null }).node_id);
	};
	page.subscribe('session', own);
	const askOwn = () => void page.get(['session']).then(own).catch(() => {});
	askOwn();
	page.addConnectListener(askOwn);

	// Where the chosen station is, as `null` for this page's own. A station whose row is
	// not here (yet, or any more) leaves the address as it was: on a page just loaded
	// that is nothing followed until the rows arrive, and for a station that has gone
	// it is a socket that goes on trying until it is back.
	let address: string | null | undefined;
	let stop: (() => void) | null = null;
	derived([wingAt, stations, thisStation], ([$at, $stations, $here]) => {
		if ($at === null || $at === $here) return null;
		return $stations.find((s) => s.id === $at)?.http_addr || undefined;
	}).subscribe((next) => {
		if (next === undefined || next === address) return;
		address = next;
		stop?.();
		const remote = next === null ? null : new PultWsClient(wsUrlFrom(`${location.protocol}//${next}`, WS_PATH), { keepsTime: false });
		current = remote;
		stop = follow(remote ?? page);
		if (remote) {
			remote.connect();
			const inner = stop;
			stop = () => {
				inner();
				remote.disconnect();
			};
		}
	});

	let session: (() => void) | null = null;
	driving.subscribe(($driving) => {
		if ($driving && !session) session = drive();
		if (!$driving && session) {
			session();
			session = null;
		}
	});
}

/** Follow the wing on the station at the other end of this socket, until stopped. */
function follow(client: PultWsClient): () => void {
	const apply = (value: unknown) => {
		if (value && typeof value === 'object') wingState.set({ ...EMPTY, ...(value as WingState) });
	};
	const refresh = () => {
		void client.get(['wing']).then(apply).catch(() => {});
		void client
			.call('wing.map', {})
			.then((map) => wingMap.set((map as WingMap | null) ?? null))
			.catch(() => wingMap.set(null));
		// A reconnect is a new session, and the claim died with the old one.
		claimedAs.set(null);
		if (get(drivesHere)) void claim();
	};
	const unsubscribe = client.subscribe('wing', apply);
	const unlisten = client.addConnectListener(refresh);
	refresh();
	return () => {
		unsubscribe();
		unlisten();
		if (get(claimedAs) !== null) void client.call('wing.release', {}).catch(() => {});
		claimedAs.set(null);
		wingState.set(EMPTY);
		wingMap.set(null);
	};
}

async function claim(): Promise<void> {
	try {
		const state = (await wingClient().call('wing.claim', {})) as WingState;
		claimedAs.set(state.claimed_by);
		wingState.set({ ...EMPTY, ...state });
	} catch (e) {
		addToast(`The wing: ${e instanceof Error ? e.message : e}`);
	}
}

/** Show the wing on this page, or stop. */
export async function setDrivesHere(on: boolean): Promise<void> {
	drivesHere.set(on);
	if (typeof localStorage !== 'undefined') localStorage.setItem(DRIVE_KEY, on ? '1' : '0');
	if (on) await claim();
	else {
		claimedAs.set(null);
		await wingClient().call('wing.release', {}).catch(() => {});
	}
}

/**
 * Everything that runs only while this page shows the wing: telling the station what
 * this page has selected, taking on what the wing selects, and answering its asks.
 * Answers the function that stops it all.
 */
function drive(): () => void {
	const client = wingClient();
	const stops: (() => void)[] = [];
	// Held open for as long as this page shows the wing, so `get` on them reads a
	// value rather than opening and closing a deep watch each time.
	for (const store of [fixtures, types, bindings, byKey, typeOf, output]) {
		stops.push((store as Readable<unknown>).subscribe(() => {}));
	}

	stops.push(
		client.subscribe('wing_asks', (value: unknown) => {
			if (value && typeof value === 'object') answer(value as WingAsk);
		})
	);

	// A selection made on the wing becomes this page's. Only once per change, and only
	// one the wing made: this page's own selection coming back in the row is not news.
	let applied = 0;
	stops.push(
		operator.subscribe(($op) => {
			const s = $op.selection;
			const rev = s.rev;
			if (!s.from_wing || rev <= applied) return;
			applied = rev;
			if (s.fixtures.length === 0) clearSelection();
			else if (s.query) recall(s.query);
			else setQuery(idsQuery(s.fixtures, undefined, s.cells));
		})
	);

	// And this page's selection, cue sheet and dialogs go the other way.
	stops.push(
		derived(
			[selection, query, focusedCueSheet, dialogCount, storeDialogs, userId, operator],
			([$selection, $query, $focused, $dialogs, $storing, $user, $op]) => ({
				fixtures: $selection,
				cells: $query.cells ?? [],
				query: $query,
				seenRev: $op.selection.rev,
				focusedSequence: $focused,
				dialog: $dialogs > 0,
				storing: $storing > 0,
				userId: $user
			})
		).subscribe((context) => {
			const text = JSON.stringify(context);
			if (text === sentContext) return;
			sentContext = text;
			void client.call('wing.context', context).catch(() => {});
		})
	);

	// The line the wing types goes into a command line panel that is open on this
	// page, so it reads where the operator is used to reading it.
	let lastLine = get(wingLine);
	stops.push(
		wingLine.subscribe((line) => {
			if (line === lastLine) return;
			lastLine = line;
			openConsoleLine()?.write(line);
		})
	);

	return () => {
		for (const stop of stops) stop();
		sentContext = '';
	};
}

let sentContext = '';

/** What the wing asked of this page. */
function answer(ask: WingAsk): void {
	switch (ask.kind) {
		case 'Toast':
			addToast(ask.text, ask.good ? 'success' : undefined);
			return;
		case 'Learned':
			learned.set(ask.control);
			return;
		case 'Store':
			wingStoreAsk.set({ preselect: ask.preselect, onto: ask.onto ?? null });
			return;
		case 'StoreTarget':
			wingStoreTarget.set(ask.onto);
			return;
		case 'Edit':
			wingEditAsk.set({
				control: ask.control,
				page: ask.page,
				sequenceId: ask.sequence_id,
				cueId: ask.cue_id
			});
			return;
		case 'Dialog': {
			const dialog = topDialog();
			if (!dialog) return;
			if (ask.accept) dialog.accept?.();
			else dialog.close();
			return;
		}
	}
}

/** What a control is called, by MA's table where the station could read it. */
export function controlName(control: SurfaceControl, map: WingMap | null = get(wingMap)): string {
	if (control.kind === 'Digital') return `Digital in ${control.index + 1}`;
	const table = control.kind === 'Key' ? map?.hardkeys : control.kind === 'Fader' ? map?.faders : map?.encoders;
	const label = table?.[String(control.index)]?.label;
	return label ? `${control.kind} ${label}` : `${control.kind} ${control.index}`;
}

// The third step of learning: a control caught, then whatever the operator touches next
// on the page that fits it is what it does. The panel's dropdowns stay for what has no
// place to be touched — a group, a command line.
onOffer(({ action }) => {
	const control = get(learned);
	if (!control) return;
	const fits = learnable(control, action);
	if (!fits) return;
	learned.set(null);
	void bind(control, fits)
		.then(() => addToast(`${controlName(control)} learned`, 'success'))
		.catch((e) => addToast(`${e instanceof Error ? e.message : e}`));
});

// ── The calculator, from the panel ────────────────────────────────────────────

/** Open the calculator on this encoder, or close it. The station keeps it. */
export function openCalculator(slot: number | null): void {
	void wingClient().call('wing.calculator', { slot }).catch(() => {});
}

/** What is typed into the open calculator, from the panel's own input. */
export function typeCalculator(slot: number, text: string): void {
	void wingClient().call('wing.calculator', { slot, text }).catch(() => {});
}

/** Set what the calculator holds on the selection, as one gesture. */
export function setTyped(slot: number, text: string): void {
	void wingClient().call('wing.calculator', { slot, text, set: true }).catch(() => {});
}

/** What an encoder slot reads as, for the panel: the first target's value. */
export function slotValue(slot: EncoderSlot): ParameterValue | null {
	const first = selectionTargets(slot.kind)[0];
	if (!first) return null;
	const key = parameterKey(first.kind);
	return get(byKey).get(`${first.fixtureId}/${key}`)?.value ?? get(output).value(first.fixtureId, key) ?? slot.defaultValue;
}

// ── Binding ───────────────────────────────────────────────────────────────────

/** Bind a control, replacing whatever it was bound to. One write either way. */
export async function bind(control: SurfaceControl, action: BindingAction, label = ''): Promise<void> {
	const data = showData();
	const existing = get(wingBindings).get(controlKey(control));
	if (existing) await data.control_bindings.byId(existing.id).set({ ...existing, action, label });
	else {
		const page = get(wingPage);
		await data.control_bindings.create({ id: uuid(), surface: COMMAND_WING, control, action, label, page });
	}
}

export async function unbind(control: SurfaceControl): Promise<void> {
	const existing = get(wingBindings).get(controlKey(control));
	if (existing) await showData().control_bindings.byId(existing.id).delete();
}

/**
 * The parameter the selection's first fixture has under this kind, for "bind to what
 * is selected": a fader on the jaw of the skeleton that is selected.
 */
export function selectedParameter(kind: ParameterKind): { fixture_id: string; parameter: ParameterKind } | null {
	const first = selectionTargets(kind)[0];
	return first ? { fixture_id: first.fixtureId, parameter: first.kind } : null;
}
