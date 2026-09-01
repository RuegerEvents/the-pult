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
	EffectSpec,
	ParameterKind,
	ParameterValue,
	ProgrammerValue,
	Sequence
} from '$lib/generated/index.js';
import { parameterKey } from '$lib/patch.js';
import { entryId, entriesFromCue, sameValue, storeCaptures } from '$lib/programmer.js';
import { collection, show, showData } from './show.js';

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
	const key = parameterKey(kind);
	for (const fixtureId of fixtureIds) {
		pending.set(entryId(fixtureId, key), { fixtureId, kind, value });
	}
	if (frame !== null) return;
	frame = requestAnimationFrame(() => {
		frame = null;
		void flush();
	});
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
			await data.programmer_values.byId(id).value.set(value);
		} else {
			await data.programmer_values.create({
				id,
				fixture_id: fixtureId,
				parameter_kind: kind,
				value,
				effect: null,
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
	const key = parameterKey(kind);
	const at = Date.now();
	const current = new Map(held.map((entry) => [entry.id, entry]));

	for (const [fixtureId, spec] of Object.entries(specs)) {
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
				parameter_kind: kind,
				// Where the parameter falls back to if the effect cannot be rendered.
				value: spec.low,
				effect: anchored,
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
	pending.delete(id);
	await showData().programmer_values.byId(id).delete();
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
	if (sequence) await data.sequences.byId(sequence.id).goToCue({ cueId: cue.id, at: Date.now() });
	for (const entry of entriesFromCue(cue)) {
		await data.programmer_values.create(entry);
	}
	await data.show.editing_cue.set(cue.id);
}

/**
 * Write the programmer back into the cue being edited.
 *
 * Replace rather than merge: the operator has the whole cue in front of them, and a
 * parameter they removed from the buffer is one they meant the cue to stop saying.
 */
export async function updateEdit(): Promise<void> {
	const cueId = get(editingCue);
	if (!cueId) return;
	const data = showData();
	const captures = storeCaptures([], held, 'replace', new Set(held.map((e) => e.id)));
	await data.cues.byId(cueId).captures.set(captures);
	await clear({ keepLocked: true });
	await data.show.editing_cue.set(null);
}

export async function cancelEdit(): Promise<void> {
	await clear({ keepLocked: true });
	await showData().show.editing_cue.set(null);
}

// ── Storing ───────────────────────────────────────────────────────────────────

/** Write the chosen entries into a cue that already exists. */
export async function storeInto(
	cue: Cue,
	mode: 'merge' | 'replace',
	include: Set<string>
): Promise<void> {
	const captures = storeCaptures(cue.captures, held, mode, include);
	await showData().cues.byId(cue.id).captures.set(captures);
}
