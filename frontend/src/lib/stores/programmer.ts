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
	Preset,
	PresetValue,
	ProgrammerValue,
	Sequence
} from '$lib/generated/index.js';
import { parameterKey } from '$lib/patch.js';
import {
	applyPreset,
	entryId,
	entriesFromCue,
	mergePresetValues,
	presetValues,
	sameValue,
	storeCaptures
} from '$lib/programmer.js';
import { drivingCues } from '$lib/cues.js';
import { beginGesture, endGesture } from './gesture.js';
import { collection, show, showData } from './show.js';

// ── What is in the buffer ─────────────────────────────────────────────────────

export const entries: Readable<ProgrammerValue[]> = collection('programmer_values');

/// Read for one question — what timing the cue being edited already had — since the
/// programmer itself carries none.
const cues: Readable<Cue[]> = collection('cues');

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
 *
 * The cue's own captures go in even so, because *replace* is about which parameters
 * the cue holds and not about their timing: the programmer carries values and never
 * a fade time, a delay or a curve, so an Update that passed nothing here would throw
 * away, every time, timing an operator set in a control that is not on this screen.
 */
export async function updateEdit(): Promise<void> {
	const cueId = get(editingCue);
	if (!cueId) return;
	const data = showData();
	const before = get(cues).find((c) => c.id === cueId)?.captures ?? [];
	const captures = storeCaptures(before, held, 'replace', new Set(held.map((e) => e.id)));
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

// ── Update ────────────────────────────────────────────────────────────────────

/**
 * Update: every held value into the cue that is **driving it now**.
 *
 * This is the verb every other desk has and this console did not, and the reason it
 * needs no target is that the console already knows one: an operator who has nudged a
 * light in the middle of cue 12 means cue 12, and being asked which cue they meant is
 * being asked a question the desk can answer.
 *
 * *How* it answers is `drivingCues`, and not `live_fades[key].cue_id` — see the note
 * there. A key the programmer holds is exactly the key playback stops publishing.
 *
 * Keys nothing is driving are the honest exception and are handed back rather than
 * guessed at: there is no cue they belong to, and the Store dialog opens with exactly
 * those ticked.
 *
 * One gesture over however many cues it touches, so an Update across three cues is
 * one Ctrl-Z. A merge rather than a replace, and the timing is kept — `storeCaptures`
 * is where both of those rules live, and this does not restate them.
 */
export async function updateDriven(): Promise<{ updated: number; orphans: string[] }> {
	const data = showData();
	// Fetched rather than read out of the shared stores. `get()` on a lazy store runs
	// its start function and hands back the *initial* value — an empty array — because
	// the subscription behind it is a socket round trip; the note on `held` above says
	// as much, and this is the other half of it. Update is one deliberate act, so one
	// round trip costs nothing and reading a stale empty rig would answer "nothing here
	// is driven by a cue" every time.
	const [sequenceList, cueList] = await Promise.all([data.sequences.get(), data.cues.get()]);
	const byId = new Map(cueList.map((c) => [c.id, c]));
	const wanted = new Set(
		held.map((entry) => `${entry.fixture_id}/${parameterKey(entry.parameter_kind)}`)
	);
	const driving = drivingCues(sequenceList, (id) => byId.get(id), wanted);

	/** Entry ids, grouped by the cue driving each. */
	const perCue = new Map<string, string[]>();
	const orphans: string[] = [];

	for (const entry of held) {
		const from = driving.get(`${entry.fixture_id}/${parameterKey(entry.parameter_kind)}`);
		if (!from) {
			orphans.push(entry.id);
			continue;
		}
		perCue.set(from, [...(perCue.get(from) ?? []), entry.id]);
	}

	if (perCue.size > 0) {
		beginGesture();
		try {
			for (const [cueId, ids] of perCue) {
				const cue = cueList.find((c) => c.id === cueId);
				if (!cue) continue;
				await data.cues.byId(cueId).captures.set(
					storeCaptures(cue.captures, held, 'merge', new Set(ids))
				);
			}
		} finally {
			endGesture();
		}
	}

	return { updated: perCue.size, orphans };
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
		id: crypto.randomUUID(),
		name,
		values: presetValues(held, chosen)
	};
	await showData().presets.create(preset);
	return preset;
}

/**
 * Fold what the programmer is holding into a preset that already exists.
 *
 * A merge rather than a replace, for the reason a store into a cue is: a preset that
 * also holds a position should not lose it because somebody re-grabbed the colour.
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
