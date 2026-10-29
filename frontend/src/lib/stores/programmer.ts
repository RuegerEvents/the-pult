/**
 * The programmer, as an operator's console sees it.
 *
 * The buffer itself is show state — a SYNCED `programmer_values` collection, so two
 * consoles work the same look and see each other doing it. What is here is the
 * talking: which entries exist, what a control does when it moves, and the three
 * things that end a session with the buffer — Clear, Store, and the Update at the
 * end of editing a cue.
 *
 * # Why writes are coalesced
 *
 * Every write is a replicated field change and an oplog row. A fader dragged across
 * its travel is a few hundred pointer events, and a selection of twenty fixtures
 * would turn each of those into twenty rows. So a move is remembered per key and
 * sent once a frame, and a value that has not actually changed is not sent at all.
 * That is a reduction rather than a fix: the oplog is still never pruned, which is
 * its own piece of work.
 */

import { derived, get, type Readable } from 'svelte/store';
import type {
	Cue,
	Easing,
	EffectSpec,
	ParameterKind,
	ParameterValue,
	Preset,
	PresetValue,
	ProgrammerValue,
	Sequence,
	StoreOnto
} from '$lib/generated/index.js';
import { parameterKey } from '$lib/patch.js';
import { kindOn, splitTarget } from '$lib/effectPool.js';
import {
	applyPreset,
	entryId,
	entriesFromCue,
	mergePresetValues,
	presetValues,
	sameValue
} from '$lib/programmer.js';
import { beginGesture, endGesture } from './gesture.js';
import { offer } from './learn.js';
import { collection, show, showClient, showData } from './show.js';
import { uuid } from '$lib/uuid.js';
import { actAt } from '$lib/ws/clock.js';

// ── What is in the buffer ─────────────────────────────────────────────────────

export const entries: Readable<ProgrammerValue[]> = collection('programmer_values');


/** The entries by `(fixture, parameter key)`, for a control asking about itself. */
export const byKey: Readable<Map<string, ProgrammerValue>> = derived(entries, ($entries) => {
	const map = new Map<string, ProgrammerValue>();
	for (const entry of $entries) {
		map.set(`${entry.fixture_id}/${parameterKey(entry.parameter_kind)}`, entry);
	}
	return map;
});

/** The cue currently loaded for editing, if any. */
export const editingCue: Readable<string | null> = derived(
	show,
	($show) => $show?.editing_cue ?? null
);

/**
 * A copy the actions can read without subscribing and unsubscribing.
 *
 * `get()` on a lazy store opens and closes the underlying subscription every time it
 * is called, which for a collection means tearing down and rebuilding a deep watch
 * on every pointer move. One permanent subscriber is cheaper and simpler.
 */
let held: ProgrammerValue[] = [];
entries.subscribe((value) => {
	held = value;
});

// ── Moving a value ────────────────────────────────────────────────────────────

type Pending = { fixtureId: string; kind: ParameterKind; value: ParameterValue };

const pending = new Map<string, Pending>();
let frame: number | null = null;

/**
 * Put a value into the programmer, for every fixture named.
 *
 * The entry id is derived from the fixture and the parameter rather than minted, so
 * a second console moving the same fader patches the same row instead of adding a
 * rival one beside it.
 */
export function setValue(fixtureIds: string[], kind: ParameterKind, value: ParameterValue): void {
	setValueOn(
		fixtureIds.map((fixtureId) => ({ fixtureId, kind })),
		value
	);
}

/**
 * The same, onto targets that may each hold the parameter under its own kind: the
 * cells of a strip, each its own `ColorRgb@n`. See `rowTargets`.
 */
export function setValueOn(targets: { fixtureId: string; kind: ParameterKind }[], value: ParameterValue): void {
	// A control being learned can take this parameter for its own: the first of them,
	// since a fader moves one parameter of one fixture.
	if (targets[0]) offer({ kind: 'Parameter', fixture_id: targets[0].fixtureId, parameter: targets[0].kind });
	for (const { fixtureId, kind } of targets) {
		pending.set(entryId(fixtureId, parameterKey(kind)), { fixtureId, kind, value });
	}
	if (frame !== null) return;
	frame = requestAnimationFrame(() => {
		frame = null;
		// At most one flush waits behind the one being sent. Queued once per frame
		// instead, a moving fader queued sixty a second behind writes that each wait for
		// the station's answer, and everything trailed further behind (found at the gig).
		// The waiting flush takes whatever is pending when its turn comes, so it is
		// always the newest value that goes.
		if (flushWaiting) return;
		flushWaiting = true;
		void inOrder(async () => {
			flushWaiting = false;
			await flush();
		});
	});
}

let flushWaiting = false;

/**
 * Every programmer write, one after another.
 *
 * A wing flash key pressed and let go fast lost its release (found at the gig): the
 * press's create was already on its way to the station when the release's delete was
 * sent, the delete found nothing, and the create landed after it — a mouth left at
 * full. Writes made from here now leave in the order they were asked for, so a
 * release always lands after the press it undoes.
 */
let writes: Promise<void> = Promise.resolve();
function inOrder(write: () => Promise<void>): Promise<void> {
	const next = writes.then(write, write);
	writes = next.catch(() => {});
	return next;
}

async function flush(): Promise<void> {
	const batch = [...pending];
	pending.clear();
	const data = showData();
	const current = new Map(held.map((entry) => [entry.id, entry]));

	for (const [id, { fixtureId, kind, value }] of batch) {
		const existing = current.get(id);
		if (existing) {
			if (sameValue(existing.value, value)) continue;
			// **Moving a value breaks its link to a preset**, and it has to be one
			// write: a value and a `preset: null` sent separately leave a moment in
			// which the show says the parameter is still that palette's while holding
			// a number the palette does not say. So a row that carries a reference is
			// written whole, and everything else keeps the cheap field write a fader
			// does a few hundred of.
			if (existing.preset) {
				await data.programmer_values.byId(id).set({ ...existing, value, preset: null });
			} else {
				await data.programmer_values.byId(id).value.set(value);
			}
		} else {
			await data.programmer_values.create({
				id,
				fixture_id: fixtureId,
				parameter_kind: kind,
				value,
				effect: null,
				// A value typed or dragged is nobody's palette. Recalling a preset is the
				// one path that writes one — see `applyPreset`.
				preset: null,
				locked: false
			});
		}
	}
}

/**
 * Send fixtures back to where their parameters rest.
 *
 * Deliberately *not* worked out here. The station decides both what a fixture has
 * and where each of its parameters rests — its own override, or what its type
 * declares — so this browser sends one write per fixture naming no parameter and no
 * value. Which keeps the resolution in one place: the values panel already carries a
 * type's `default_value` per row for an empty readout, and that is a display
 * fallback, not an answer about the rig.
 *
 * A parked value is left where it was parked, by the same rule as Clear.
 */
export async function home(fixtureIds: string[]): Promise<void> {
	const data = showData();
	for (const fixtureId of fixtureIds) {
		await data.programmer_values.home({ fixtureId });
	}
}

/**
 * Put an effect into the programmer, one spec per fixture.
 *
 * The specs differ only in phase — `specsFor` builds them, sharing one `effect_id`
 * so the panel can gather them back afterwards. The anchor is set here rather than
 * when the panel was opened, so the effect starts its first cycle at the moment the
 * operator pressed Apply.
 *
 * Written straight through rather than staged like `setValue`: applying an effect is
 * one deliberate act, not a fader being dragged, so there is no burst to coalesce.
 */
export async function setEffect(
	kind: ParameterKind,
	specs: Record<string, EffectSpec>
): Promise<void> {
	const data = showData();
	// The show clock: an effect's `t0` is its phase on every station.
	const at = actAt() ?? Date.now();
	const current = new Map(held.map((entry) => [entry.id, entry]));

	// Keyed by target: a fixture, or `fixture@cell` for one cell of a multi-cell one,
	// which holds the parameter under its cell's own kind.
	for (const [target, spec] of Object.entries(specs)) {
		const fixtureId = splitTarget(target).fixture;
		const heldKind = kindOn(kind, target);
		const key = parameterKey(heldKind);
		const id = entryId(fixtureId, key);
		const anchored = { ...spec, t0: at };
		// A value pending from a fader drag would land after this and cover it.
		pending.delete(id);

		if (current.has(id)) {
			await data.programmer_values.byId(id).effect.set(anchored);
		} else {
			await data.programmer_values.create({
				id,
				fixture_id: fixtureId,
				parameter_kind: heldKind,
				// Where the parameter falls back to if the effect cannot be rendered.
				value: spec.low,
				effect: anchored,
				preset: null,
				locked: false
			});
		}
	}
}

/**
 * Take an effect off every entry that carries it.
 *
 * The entries themselves stay, holding their last value: an operator taking the
 * chase off a group is asking for it to stop moving, not for the lights to drop
 * back to whatever the cue underneath says.
 */
export async function removeEffect(effectId: string): Promise<void> {
	const data = showData();
	for (const entry of held) {
		if (entry.effect?.effect_id !== effectId) continue;
		await data.programmer_values.byId(entry.id).effect.set(null);
	}
}

/** Every distinct effect the programmer is holding, with the entries under it. */
export function effectsHeld(entries: ProgrammerValue[]): Map<string, ProgrammerValue[]> {
	const out = new Map<string, ProgrammerValue[]>();
	for (const entry of entries) {
		if (!entry.effect) continue;
		const run = out.get(entry.effect.effect_id) ?? [];
		run.push(entry);
		out.set(entry.effect.effect_id, run);
	}
	return out;
}

// ── Emptying it ───────────────────────────────────────────────────────────────

export async function remove(id: string): Promise<void> {
	// A value still waiting for its frame is dropped; one already sent is deleted after
	// it has landed, not before.
	pending.delete(id);
	await inOrder(() => showData().programmer_values.byId(id).delete());
}

/**
 * Give everything back to playback.
 *
 * Locked values stay: parking a value is exactly the ask that it survive a Clear,
 * so the same look can go into several cues without being built twice.
 */
export async function clear({ keepLocked = true } = {}): Promise<void> {
	pending.clear();
	const data = showData();
	for (const entry of held) {
		if (keepLocked && entry.locked) continue;
		await data.programmer_values.byId(entry.id).delete();
	}
}

export async function toggleLock(id: string): Promise<void> {
	const entry = held.find((e) => e.id === id);
	if (!entry) return;
	await showData().programmer_values.byId(id).locked.set(!entry.locked);
}

/** Park everything at once, for a look about to go into several cues. */
export async function lockAll(locked = true): Promise<void> {
	const data = showData();
	for (const entry of held) {
		if (entry.locked === locked) continue;
		await data.programmer_values.byId(entry.id).locked.set(locked);
	}
}

// ── Editing a cue ─────────────────────────────────────────────────────────────

/**
 * Load a cue into the programmer to change it.
 *
 * Load, tweak, Update — not live editing. A cue that rewrote itself as an operator
 * touched a fader would have no way back from a mistake, and would be doing it on
 * every console at once.
 *
 * The cue is also taken, so what is on stage is what is being edited. Anything
 * unlocked in the buffer goes first: whatever was half-built before is not part of
 * this cue, and leaving it would quietly store it into one.
 */
export async function beginEdit(cue: Cue, sequence: Sequence | null): Promise<void> {
	const data = showData();
	await clear({ keepLocked: true });
	if (sequence) await data.sequences.byId(sequence.id).goToCue({ cueId: cue.id, at: actAt() });
	for (const entry of entriesFromCue(cue)) {
		await data.programmer_values.create(entry);
	}
	await data.show.editing_cue.set(cue.id);
}

export async function cancelEdit(): Promise<void> {
	await clear({ keepLocked: true });
	await showData().show.editing_cue.set(null);
}

// ── Update and Store ──────────────────────────────────────────────────────────

/**
 * Update and Store are the station's (`infra/storing.rs`), and so are the wing's: one
 * code path, so a store from the wing and one from this screen are the same act. Each
 * is one gesture under this page's user, so one Ctrl-Z however many cues it touched.
 *
 * Whatever a fader moved this frame goes first, or the act would miss the last move.
 */
async function settled(): Promise<void> {
	await inOrder(flush);
}

/** What an Update did: see `storing::Updated`. */
export type Updated = {
	updated: number;
	/** Entries no cue drives. They stay in the programmer for the Store dialog. */
	orphans: string[];
	/** The cue loaded for editing, when Update wrote back into that instead. */
	edited: { cueId: string; number: number; name: string } | null;
};

/**
 * Update: every held value into the cue that is **driving it now** — or, while a cue
 * is loaded for editing, into that cue, which becomes exactly what the programmer holds
 * and ends the edit.
 *
 * The verb every other desk has, and it needs no target because the console already
 * knows one: an operator who nudged a light in the middle of cue 12 means cue 12. Keys
 * nothing is driving are handed back rather than guessed at, and the programmer is
 * cleared only once every value has found its cue.
 */
/**
 * Blind: the programmer reaches no lamp, and every screen goes on showing it. One for
 * the show, SYNCED, so every page and the wing's BLIND light agree. See `Show::blind`.
 */
export const blind: Readable<boolean> = derived(show, ($show) => !!$show?.blind);

/**
 * Switch blind on or off, or over with no argument. The station's `programmer.blind`,
 * the one the wing's BLIND key runs: unattributed, so Ctrl-Z never switches it off on
 * the way past an operator's own change.
 */
export async function setBlind(on?: boolean): Promise<boolean> {
	const answer = (await showClient().call('programmer.blind', on === undefined ? {} : { on })) as { blind: boolean };
	return answer.blind;
}

export async function update(): Promise<Updated> {
	await settled();
	return (await showClient().call('programmer.update', {})) as Updated;
}

/** Everything the Store dialog asks: see `storing::StoreRequest`. */
export type StoreRequest = {
	include?: string[];
	sequenceId?: string;
	newSequence?: string;
	cueId?: string;
	replace?: boolean;
	name?: string;
	after?: string | null;
	fadeInMs?: number;
	fadeOutMs?: number;
	easing?: Easing | null;
	followMode?: Cue['follow_mode'];
	/** Per entry, only for the rows somebody typed into; the rest keep what the cue had. */
	timing?: Record<string, { fadeMs: number; delayMs: number; easing: Easing | null }>;
	cueOnly?: boolean;
	keep?: boolean;
	onto?: StoreOnto | null;
};

export type Stored = {
	sequenceId: string;
	sequenceName: string;
	cue: { cueId: string; number: number; name: string };
	created: boolean;
};

/** Store the programmer into a cue: a new one, or one that exists. */
export async function store(request: StoreRequest): Promise<Stored> {
	await settled();
	return (await showClient().call('programmer.store', request)) as Stored;
}

// ── Presets ───────────────────────────────────────────────────────────────────

/**
 * Recall a preset into the programmer.
 *
 * One gesture, so a preset applied to forty heads is one Ctrl-Z. Each entry carries
 * the reference **and** the value it resolved to — see `applyPreset` for why both.
 *
 * With nothing selected it applies to every fixture the preset knows; with a selection
 * it is the intersection. A preset that reaches none of the selection writes nothing
 * rather than clearing what is held, which is what an operator means by a button that
 * does not apply here.
 */
export async function recallPreset(preset: Preset, selection: string[]): Promise<number> {
	const rows = applyPreset(preset, selection);
	if (rows.length === 0) return 0;
	const data = showData();
	const current = new Map(held.map((entry) => [entry.id, entry]));
	beginGesture();
	try {
		for (const row of rows) {
			// A pending fader move would land after this and cover it.
			pending.delete(row.id);
			if (current.has(row.id)) await data.programmer_values.byId(row.id).set(row);
			else await data.programmer_values.create(row);
		}
	} finally {
		endGesture();
	}
	return rows.length;
}

/** Make a preset out of what the programmer is holding. */
export async function storePreset(name: string, include?: Set<string>): Promise<Preset> {
	const chosen = include ?? new Set(held.map((entry) => entry.id));
	const preset: Preset = {
		id: uuid(),
		name,
		color: null,
		values: presetValues(held, chosen)
	};
	await showData().presets.create(preset);
	return preset;
}

/**
 * Fold what the programmer is holding into a preset that already exists.
 *
 * A merge rather than a replace, for the reason a store into a cue is: a preset that
 * also holds a position should not lose it because somebody re-grabbed the color.
 * The right-click verb on a pool button, and the one that makes a palette live —
 * every standing cue that references it moves as soon as this lands.
 */
export async function updatePreset(preset: Preset, include?: Set<string>): Promise<number> {
	const chosen = include ?? new Set(held.map((entry) => entry.id));
	const incoming = presetValues(held, chosen);
	if (incoming.length === 0) return 0;
	await showData()
		.presets.byId(preset.id)
		.values.set(mergePresetValues(preset.values, incoming));
	return incoming.length;
}

/** Edit one value of a preset in place — the sheet's inspector, in preset mode. */
export async function setPresetValue(
	preset: Preset,
	fixtureId: string,
	kind: ParameterKind,
	value: ParameterValue | null
): Promise<void> {
	const key = parameterKey(kind);
	const at = (v: PresetValue) => `${v.fixture_id}/${parameterKey(v.parameter_kind)}`;
	const without = preset.values.filter((v) => at(v) !== `${fixtureId}/${key}`);
	const next =
		value === null
			? without
			: [...without, { fixture_id: fixtureId, parameter_kind: kind, value }];
	await showData().presets.byId(preset.id).values.set(next);
}
