/**
 * The command wing, driven from this page.
 *
 * The wing is plugged into a station, and the station does no programming with it: it
 * relays what is pressed to the page that claimed it and puts back on the desk the
 * frame of lights and motor positions that page asks for (`infra/wing.rs`). This is
 * that page's half — and it is all of the programming, because the selection, the
 * command line and the gestures a wing acts on are all this page's.
 *
 * Whether this page drives the wing is remembered in `localStorage`, so a reload or a
 * reconnect claims it again. Claiming takes it from any other page: the operator
 * standing at the wing is the one who asked.
 *
 * What a control does, in order: learning takes the next control touched; a binding
 * in the show does what it says; and what is left is MA's own meaning where this
 * console has one — the keypad types, PLEASE runs, the encoders turn the selection,
 * GO is Space.
 */

import { derived, get, writable, type Readable } from 'svelte/store';

import { goOnFocusedSheet } from './cues.js';
import { fromWing, onOffer } from './learn.js';
import { topDialog } from './dialogs.js';
import { beginGesture, endGesture, nudging } from './gesture.js';
import { output, watching } from './output.js';
import { consolePluginOf, openConsoleLine, pluginsState } from './plugins.js';
import { byKey, setValueOn } from './programmer.js';
import { clearStep } from './clear.js';
import { applySelectionEffect, query, recall, selection, targetElements } from './selection.js';
import { collection, showClient, showData } from './show.js';
import { undo } from './undo.js';
import { userId } from './user.js';
import { addToast } from '../toasts.js';
import { PultWsClient } from '../ws/client.js';
import { consoleNow } from '../ws/clock.js';
import { WS_PATH, wsUrlFrom } from '../ws/endpoint.js';
import type {
	BindingAction,
	ControlBinding,
	ParameterKind,
	ParameterValue,
	SurfaceControl,
	WingState
} from '../generated/index.js';
import { modeHas, modeOf, parameterKey } from '../patch.js';
import { editableParameters, rowTargets, type Target } from '../programmer.js';
import { pressTarget, targetLive, type SurfaceShow } from '../surface.js';
import {
	DIM,
	calculatorTyped,
	digitalEdges,
	encoderPress,
	typedValue,
	LED_CHANNELS,
	LIT,
	encoderPage,
	encoderRole,
	encoderSlots,
	faderKeys,
	learnable,
	keyVerb,
	keypadWord,
	KEYPAD_CODES,
	ledChannels,
	ledKeys,
	oops,
	pageCount,
	turned,
	typed,
	type EncoderSlot,
	type WingEvent,
	type WingMap
} from '../wing.js';
import { uuid } from '$lib/uuid.js';

/** The surface kind this page binds under — `pult_schema::types::wing::COMMAND_WING`. */
export const COMMAND_WING = 'ma3-command-wing';

const EMPTY: WingState = { phase: { kind: 'Off' }, device: null, claimed_by: null, digital: null };
const DRIVE_KEY = 'pult.wing.drive';
const AT_KEY = 'pult.wing.at';
/** How often the desk's lights and motors are brought up to date. */
const FEEDBACK_MS = 40;

// ── What is known ─────────────────────────────────────────────────────────────

/** The station's `wing` row: whether a wing is here, and which page has it. */
export const wingState = writable<WingState>(EMPTY);

/** Whether this page means to drive the wing. Remembered across reloads. */
export const drivesHere = writable<boolean>(
	typeof localStorage !== 'undefined' && localStorage.getItem(DRIVE_KEY) === '1'
);

/** The key this page's claim landed under, which is how it knows the row means it. */
const claimedAs = writable<string | null>(null);

/** True while this page is the one driving the wing. */
export const driving: Readable<boolean> = derived(
	[wingState, drivesHere, claimedAs],
	([$state, $drives, $as]) => $drives && $as !== null && $state.claimed_by === $as
);

/** MA's names for every control, from the station's grandMA3 install. */
export const wingMap = writable<WingMap | null>(null);

/** Waiting for the next control touched, to bind it. */
export const learning = writable(false);
/** The control learning caught, for the panel to open its editor on. */
export const learned = writable<SurfaceControl | null>(null);

/** Which five of the selection's attributes the encoders are on. */
export const encoderPageIndex = writable(0);

/** What the wing has typed, when no command line is open to type into. */
export const wingLine = writable('');

/**
 * The calculator a large encoder's press opens: which of the five, and what has been
 * typed. MA's own behavior — the keypad types a number into it, PLEASE sets it on the
 * selection, ESC or a second press closes it. `null` while none is open.
 */
export const calculator = writable<{ slot: number; text: string } | null>(null);

/** The keys held down right now, by index. For lighting them while they are. */
const held = writable<Set<number>>(new Set());

/** The faders a hand is on, by index: those the motors leave alone. */
const touched = writable<Set<number>>(new Set());

// ── The show, while driving ───────────────────────────────────────────────────

const fixtures = collection('fixtures');
const types = collection('fixture_types');
const sequences = collection('sequences');
const timelines = collection('timelines');
const variables = collection('variables');
const flows = collection('flows');
const flowNodes = collection('flow_nodes');
const groups = collection('groups');
const bindings = collection('control_bindings');

/** This wing's bindings, by control. */
export const wingBindings: Readable<Map<string, ControlBinding>> = derived(bindings, ($bindings) => {
	const out = new Map<string, ControlBinding>();
	for (const b of $bindings) if (b.surface === COMMAND_WING) out.set(controlKey(b.control), b);
	return out;
});

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

/** The selection's attributes, as the encoder bar pages through them. */
export const encoderBar: Readable<{ slots: (EncoderSlot | null)[]; page: number; pages: number }> = derived(
	[fixtures, types, selection, encoderPageIndex],
	([$fixtures, $types, $selection, $page]) => {
		const chosen = $fixtures.filter((f) => $selection.includes(f.id));
		const all = encoderSlots(editableParameters($types, chosen));
		const pages = pageCount(all);
		const page = Math.min($page, pages - 1);
		return { slots: encoderPage(all, page), page, pages };
	}
);

// ── Starting ──────────────────────────────────────────────────────────────────

/**
 * Which station's wing this page follows: its own (`null`), or another's by node id.
 *
 * A wing is plugged into one station and that station relays it, but the page driving
 * it can have come from any of them — the operator's screen is on the desk and the wing
 * is on a Pi under it. So the wing is followed over a socket of its own to the station
 * it is plugged into, while everything it *does* goes through this page's own station
 * as anything else this page does would.
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
 * have a socket. Costs one small LOCAL subscription on a page that never drives it.
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
		// Letting go of the wing being left, so it goes dark at once rather than when
		// this socket's session is noticed to have gone.
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
		wingState.set(state);
	} catch (e) {
		addToast(`The wing: ${e instanceof Error ? e.message : e}`);
	}
}

/** Drive the wing from this page, or stop. */
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
 * Everything that runs only while this page is driving: the relay in, the frame out,
 * and the show subscriptions both need. Answers the function that stops it all.
 */
function drive(): () => void {
	const client = wingClient();
	// Where the inputs rest, from the row: the station keeps them while nobody drives.
	digitalBits = get(wingState).digital ?? 0;
	const stops: (() => void)[] = [];
	// Held open for as long as this page drives, so the handlers can read them with
	// `get` without each read opening and closing a deep watch.
	for (const store of [fixtures, types, sequences, timelines, variables, flows, flowNodes, groups, bindings, byKey, typeOf, encoderBar, output]) {
		stops.push((store as Readable<unknown>).subscribe(() => {}));
	}
	stops.push(
		client.subscribe('wing_events', (value: unknown) => {
			if (Array.isArray(value)) for (const event of value as WingEvent[]) void handle(event);
		})
	);

	// What the motors and the encoder bar have to read, evaluated every frame.
	const watched = watching([]);
	stops.push(() => watched.stop());
	stops.push(
		derived([encoderBar, wingBindings, targetElements], ([$bar, $bindings]) => {
			const keys: string[] = [];
			for (const slot of $bar.slots) {
				if (slot) for (const t of selectionTargets(slot.kind)) keys.push(`${t.fixtureId}/${parameterKey(t.kind)}`);
			}
			for (const b of $bindings.values()) {
				if (b.action.kind === 'Parameter') keys.push(`${b.action.fixture_id}/${parameterKey(b.action.parameter)}`);
			}
			return keys;
		}).subscribe((keys) => watched.update(keys))
	);

	let sent = '';
	const timer = setInterval(() => {
		const frame = feedbackFrame();
		const text = JSON.stringify(frame);
		if (text === sent) return;
		sent = text;
		client.call('wing.feedback', frame).catch(() => {});
	}, FEEDBACK_MS);
	stops.push(() => clearInterval(timer));

	return () => {
		for (const stop of stops) stop();
		held.set(new Set());
		calculator.set(null);
		touched.set(new Set());
	};
}

// ── What a control does ───────────────────────────────────────────────────────

async function handle(event: WingEvent): Promise<void> {
	const map = get(wingMap);
	switch (event.kind) {
		case 'key':
			held.update((s) => {
				const next = new Set(s);
				if (event.down) next.add(event.index);
				else next.delete(event.index);
				return next;
			});
			return onKey(event.index, event.down, map);
		case 'fader':
			return onFader(event.index, event.value);
		case 'encoder':
			return onEncoder(event.index, event.delta, map);
		case 'digital':
			return onDigital(event.bits);
		default:
			return;
	}
}

/** The digital inputs as this page last knew them, which is what an edge is against. */
let digitalBits = 0;

/**
 * A contact on the rear panel: a key with no LED and no meaning of MA's, so it does
 * only what it is learned or bound to.
 */
async function onDigital(bits: number): Promise<void> {
	const before = digitalBits;
	digitalBits = bits;
	for (const { index, down } of digitalEdges(before, bits)) {
		const control: SurfaceControl = { kind: 'Digital', index };
		if (down && catchLearn(control)) continue;
		const binding = get(wingBindings).get(controlKey(control));
		if (binding) await run(binding.action, down);
	}
}

function catchLearn(control: SurfaceControl): boolean {
	if (!get(learning)) return false;
	learning.set(false);
	learned.set(control);
	return true;
}

/** What a control is called, by MA's table where the station could read it. */
export function controlName(control: SurfaceControl, map: WingMap | null = get(wingMap)): string {
	if (control.kind === 'Digital') return `Digital in ${control.index + 1}`;
	const table = control.kind === 'Key' ? map?.hardkeys : control.kind === 'Fader' ? map?.faders : map?.encoders;
	const label = table?.[String(control.index)]?.label;
	return label ? `${control.kind} ${label}` : `${control.kind} ${control.index}`;
}

// The third step of learning: a control caught, then whatever the operator touches next
// that fits it is what it does. The panel's dropdowns stay for what has no place to be
// touched — a group, a command line.
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

async function onKey(index: number, down: boolean, map: WingMap | null): Promise<void> {
	const control = map?.hardkeys[String(index)];
	const code = control?.Code;
	const faderKey = faderKeys(map).get(index);

	// A hand on a fader is a gesture beginning, and lifting off is it ending: the one
	// surface that says where an act begins and ends instead of leaving it to a guess.
	if (faderKey?.role === 'touch') {
		touched.update((s) => {
			const next = new Set(s);
			if (down) next.add(faderKey.fader);
			else next.delete(faderKey.fader);
			return next;
		});
		if (down) beginGesture();
		else endGesture();
		return;
	}

	// An open dialog is what the operator is looking at, so ESC and PLEASE answer it
	// rather than the command line behind it: ESC dismisses, PLEASE accepts.
	const dialog = topDialog();
	const verb = keyVerb(code);
	if (dialog && down && (verb === 'escape' || verb === 'please')) {
		if (verb === 'escape') dialog.close();
		else dialog.accept?.();
		return;
	}

	// A press belongs to the attribute under that encoder, which moves with the
	// selection and the page — so it is never learned or bound, and does only this.
	const pressed = encoderPress(code);
	if (pressed !== undefined) {
		if (down) openCalculator(pressed);
		return;
	}
	if (get(calculator) && down && calculatorKey(code)) return;

	if (keyVerb(code) === 'learn') {
		if (down) learning.update((on) => !on);
		return;
	}
	if (down && catchLearn({ kind: 'Key', index })) return;

	const binding = get(wingBindings).get(controlKey({ kind: 'Key', index }));
	if (binding) return run(binding.action, down);

	// The keys of a fader that is a sequence's master are that sequence's Go and Off,
	// which the station holding the wing runs (`infra/wing_control.rs`).
	if (faderKey) {
		const master = get(wingBindings).get(controlKey({ kind: 'Fader', index: faderKey.fader }));
		if (master?.action.kind === 'Master') return;
	}

	if (!down) return;
	const word = keypadWord(code);
	if (word !== undefined) return type((line) => typed(line, word));
	switch (keyVerb(code)) {
		case 'please':
			return please();
		case 'clear':
			return clearKey();
		case 'escape':
			return type(() => '');
		case 'oops': {
			const line = readLine();
			if (line.trim()) return type(oops);
			return undo();
		}
		case 'go':
			return goOnFocusedSheet();
		case 'goBack':
			return goOnFocusedSheet(true);
		case 'pageUp':
			// A page turn moves every encoder to another attribute, the open one included.
			calculator.set(null);
			encoderPageIndex.update((p) => Math.max(0, p - 1));
			return;
		case 'pageDown':
			calculator.set(null);
			encoderPageIndex.update((p) => Math.min(get(encoderBar).pages - 1, p + 1));
			return;
	}
}

/**
 * CLEAR, the way MA's works: a line being typed is emptied first, then the selection,
 * then the programmer — one per press, the same steps as the button and Esc
 * (`stores/clear.ts`).
 */
async function clearKey(): Promise<void> {
	if (readLine().trim()) return type(() => '');
	await clearStep();
}

// ── The command line ──────────────────────────────────────────────────────────

const readLine = (): string => openConsoleLine()?.read() ?? get(wingLine);

function type(edit: (line: string) => string): void {
	const open = openConsoleLine();
	if (open) open.write(edit(open.read()));
	else wingLine.update(edit);
}

async function please(): Promise<void> {
	const open = openConsoleLine();
	if (open) return open.submit();
	const line = get(wingLine).trim();
	if (!line) return;
	wingLine.set('');
	await runLine(line);
}

/**
 * Run a line where no command line is open to run it in: through the same plugin
 * call the panel makes, and what it says comes back as a toast.
 */
async function runLine(line: string): Promise<void> {
	const plugin = consolePluginOf(get(pluginsState));
	if (!plugin) {
		addToast('The wing types into the command line, and no command-line plugin is running');
		return;
	}
	try {
		const result = (await showClient().call(`plugin.${plugin}.surface.exec`, {
			payload: { line },
			ctx: { selection: get(selection), cells: get(query).cells ?? [], userId: get(userId) }
		})) as {
			lines?: { kind: string; text: string }[];
			error?: { message: string };
			effects?: Parameters<typeof applySelectionEffect>[0];
		} | null;
		applySelectionEffect(result?.effects);
		if (result?.error) addToast(`${line}: ${result.error.message}`);
		else {
			const said = result?.lines?.map((l) => l.text).join(' · ');
			if (said) addToast(said, 'success');
		}
	} catch (e) {
		addToast(`${line}: ${e instanceof Error ? e.message : e}`);
	}
}

// ── A binding, run ────────────────────────────────────────────────────────────

function surfaceShow(): SurfaceShow {
	return {
		variables: get(variables),
		timelines: get(timelines),
		sequences: get(sequences),
		flows: get(flows),
		flowNodes: get(flowNodes),
		fixtures: get(fixtures),
		types: get(types)
	};
}

function parameterDefault(fixtureId: string, kind: ParameterKind): ParameterValue {
	const key = parameterKey(kind);
	const type = get(typeOf).get(fixtureId)?.type;
	return type?.parameters.find((p) => parameterKey(p.kind) === key)?.default_value ?? { type: 'Float', value: 0 };
}

async function run(action: BindingAction, down: boolean): Promise<void> {
	try {
		switch (action.kind) {
			case 'Press':
				if (down) await fromWing(() => pressTarget(action.target, surfaceShow(), showData(), showClient(), consoleNow()));
				return;
			case 'CommandLine':
				if (!down) return;
				type(() => action.line);
				return please();
			case 'SelectGroup': {
				if (!down) return;
				const group = get(groups).find((g) => g.id === action.group_id);
				if (group) recall(group.query);
				return;
			}
			case 'Flash':
				// Run by the station holding the wing, with no browser in the way of a
				// key that has to land on time (`infra/wing_control.rs`).
				return;
			default:
				return;
		}
	} catch (e) {
		addToast(`${e instanceof Error ? e.message : e}`);
	}
}

// ── Faders ────────────────────────────────────────────────────────────────────

/**
 * A fader moved. Learning one is this page's; what a bound fader does — a parameter, a
 * sequence's master, and the motor following the show — is the station's that holds
 * the wing (`infra/wing_control.rs`), because a fader is timing-critical and a page's
 * main thread is not somewhere to put one.
 */
function onFader(index: number, _raw: number): void {
	catchLearn({ kind: 'Fader', index });
}

// ── Encoders ──────────────────────────────────────────────────────────────────

/**
 * What each target was last turned to, and when. A run of clicks inside one frame
 * would otherwise each start from a value the programmer has not caught up with yet,
 * and a fast spin would come to one click.
 */
const recent = new Map<string, { value: ParameterValue; at: number }>();
const RECENT_MS = 400;

function currentOf(target: Target, fallback: ParameterValue): ParameterValue {
	const key = `${target.fixtureId}/${parameterKey(target.kind)}`;
	const mine = recent.get(key);
	if (mine && Date.now() - mine.at < RECENT_MS) return mine.value;
	return get(byKey).get(key)?.value ?? get(output).value(target.fixtureId, parameterKey(target.kind)) ?? fallback;
}

function turnTargets(targets: Target[], fallback: ParameterValue, clicks: number, fine: boolean, component?: 'r' | 'g' | 'b'): void {
	nudging();
	for (const target of targets) {
		// Each from its own value, so a fan of levels stays a fan — the relative move
		// every desk's encoder makes.
		const next = turned(currentOf(target, fallback), clicks, fine, component);
		recent.set(`${target.fixtureId}/${parameterKey(target.kind)}`, { value: next, at: Date.now() });
		fromWing(() => setValueOn([target], next));
	}
}

function onEncoder(index: number, delta: number, map: WingMap | null): void {
	if (catchLearn({ kind: 'Encoder', index })) return;
	const binding = get(wingBindings).get(controlKey({ kind: 'Encoder', index }));
	if (binding?.action.kind === 'Parameter') {
		const { fixture_id, parameter } = binding.action;
		turnTargets([{ fixtureId: fixture_id, kind: parameter }], parameterDefault(fixture_id, parameter), delta, false);
		return;
	}
	if (binding) return;
	const role = encoderRole(map, index);
	if (!role) return;
	const slot = get(encoderBar).slots[role.slot];
	if (!slot) return;
	turnTargets(selectionTargets(slot.kind), slot.defaultValue, delta, role.fine, slot.component);
}

// ── Typing a value ────────────────────────────────────────────────────────────

/** Open the calculator on this encoder, or close it if it is the one open. */
export function openCalculator(slot: number): void {
	if (get(calculator)?.slot === slot) return calculator.set(null);
	if (!get(encoderBar).slots[slot]) return calculator.set(null);
	calculator.set({ slot, text: '' });
}

/**
 * A keypad key while the calculator is open. Answers whether it was the calculator's:
 * digits and the dot type, CLEAR empties, Oops takes one back, ESC closes and PLEASE
 * sets. The keypad's other words are swallowed rather than typed into the command line
 * behind it; every other key does what it always does.
 */
function calculatorKey(code: string | undefined): boolean {
	const word = keypadWord(code);
	if (word !== undefined) {
		calculator.update((c) => (c ? { ...c, text: calculatorTyped(c.text, word) } : c));
		return true;
	}
	switch (keyVerb(code)) {
		case 'clear':
			calculator.update((c) => (c ? { ...c, text: '' } : c));
			return true;
		case 'oops':
			calculator.update((c) => (c ? { ...c, text: c.text.slice(0, -1) } : c));
			return true;
		case 'escape':
			calculator.set(null);
			return true;
		case 'please':
			setTyped();
			return true;
		default:
			return false;
	}
}

/**
 * Set what the calculator holds on every element of the selection, as one gesture —
 * the same value for each, since typing a number is saying where, not how far.
 */
export function setTyped(text: string | undefined = get(calculator)?.text): void {
	const open = get(calculator);
	if (!open || text === undefined) return;
	const slot = get(encoderBar).slots[open.slot];
	calculator.set(null);
	if (!slot) return;
	const targets = selectionTargets(slot.kind);
	const values = targets.map((t) => typedValue(text, currentOf(t, slot.defaultValue), slot.component));
	if (values.some((v) => v === null)) {
		addToast(`${slot.label}: nothing to set from "${text}"`);
		return;
	}
	beginGesture();
	try {
		targets.forEach((target, i) => fromWing(() => setValueOn([target], values[i]!)));
	} finally {
		endGesture();
	}
}

/** What an encoder slot reads as, for the panel: the first target's value. */
export function slotValue(slot: EncoderSlot): ParameterValue | null {
	const first = selectionTargets(slot.kind)[0];
	return first ? currentOf(first, slot.defaultValue) : null;
}

// ── The desk, lit ─────────────────────────────────────────────────────────────

function liveOf(action: BindingAction, show: SurfaceShow): boolean {
	switch (action.kind) {
		case 'Press':
			return targetLive(action.target, show);
		default:
			return false;
	}
}

/**
 * The lights the desk should be showing, a level per LED channel. The motors are the
 * station's, which follows the show without a page in between.
 */
function feedbackFrame(): { leds: number[] } {
	const map = get(wingMap);
	const leds = new Array<number>(LED_CHANNELS).fill(0);
	if (!map) return { leds };

	const show = surfaceShow();
	const down = get(held);
	const bound = get(wingBindings);
	const keysOfFaders = faderKeys(map);
	const bar = get(encoderBar);
	const blink = Date.now() % 600 < 300;

	for (const [led, index] of ledKeys(map)) {
		const code = map.hardkeys[String(index)]?.Code;
		let level = 0;
		const binding = bound.get(controlKey({ kind: 'Key', index }));
		const ofFader = keysOfFaders.get(index);
		const master = ofFader ? bound.get(controlKey({ kind: 'Fader', index: ofFader.fader })) : undefined;
		if (down.has(index)) level = LIT;
		else if (keyVerb(code) === 'learn') level = get(learning) ? (blink ? LIT : 0) : DIM;
		else if (binding) level = liveOf(binding.action, show) ? LIT : DIM;
		else if (master?.action.kind === 'Master' && ofFader) {
			const id = master.action.sequence_id;
			const on = show.sequences.some((s) => s.id === id && s.active_cue_index !== null);
			level = ofFader.role === 'go' && on ? LIT : DIM;
		} else if (keyVerb(code) === 'pageUp') level = bar.page > 0 ? DIM : 0;
		else if (keyVerb(code) === 'pageDown') level = bar.page < bar.pages - 1 ? DIM : 0;
		else if ((code && KEYPAD_CODES.has(code)) || keyVerb(code)) level = DIM;
		for (const channel of ledChannels(led)) leds[channel] = level;
	}

	return { leds };
}

// ── Binding ───────────────────────────────────────────────────────────────────

/** Bind a control, replacing whatever it was bound to. One write either way. */
export async function bind(control: SurfaceControl, action: BindingAction, label = ''): Promise<void> {
	const data = showData();
	const existing = get(wingBindings).get(controlKey(control));
	if (existing) await data.control_bindings.byId(existing.id).set({ ...existing, action, label });
	else {
		await data.control_bindings.create({ id: uuid(), surface: COMMAND_WING, control, action, label });
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
