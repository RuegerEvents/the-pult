/**
 * The effect pool, as arithmetic: a template turned into specs for a selection, and
 * back, and what the preview asks the evaluator.
 *
 * Pure. A template is everything an effect is except the selection (see
 * `effect_template.rs`), so applying one is the only place the phases are worked out,
 * and they are worked out **in the selection's own order**. A strip's cells recalled
 * with *Select pixels* arrive in order along it, and a chase across them has to run
 * along it, not in the order they were patched.
 */

import type {
	EffectLayer,
	EffectSpec,
	EffectTemplate,
	Fixture,
	FixtureType,
	ParameterKind,
	Spread
} from './generated/index.js';
import { phases } from './effects.js';
import { inCell, kindBase, parameterKey, splitCell } from './patch.js';

/**
 * Something an effect is written onto: a fixture, or one cell of one, spelled
 * `fixture` or `fixture@cell` — the cell key's own separator, so a target splits the
 * way a key does. Fixture ids are uuids and never contain one.
 */
export type TargetId = string;

/** A selection's elements as targets. */
export function targetIds(elements: { fixture: string; cell: number | null }[]): TargetId[] {
	return elements.map((e) => (e.cell === null ? e.fixture : `${e.fixture}@${e.cell}`));
}

/** A target back into its fixture and cell. */
export function splitTarget(target: TargetId): { fixture: string; cell: number | null } {
	const [fixture, cell] = splitCell(target);
	return { fixture, cell };
}

/** The kind a layer's parameter is held under on one target: the cell's, on a cell. */
export function kindOn(kind: ParameterKind, target: TargetId): ParameterKind {
	const { cell } = splitTarget(target);
	return cell === null ? kindBase(kind) : inCell(kind, cell);
}

/** One layer of a template, resolved against a selection. */
export type LayerApply = {
	/** Which of the template's layers this came from. */
	layer: number;
	kind: ParameterKind;
	/** The targets that have this parameter, in selection order, and their specs. */
	specs: Record<TargetId, EffectSpec>;
	/** In the same order, because a record's key order is not a promise. */
	order: TargetId[];
};

/**
 * Which of `selection`'s targets (in its order) take `kind`, as the targets it is
 * written to. A cell takes a per-cell parameter as itself; a cell of a fixture whose
 * parameter is whole (a bar's master dimmer) takes it as the fixture, once.
 */
export function havingKind(
	kind: ParameterKind,
	selection: TargetId[],
	fixtures: Map<string, Fixture>,
	types: Map<string, FixtureType>
): TargetId[] {
	const key = parameterKey(kindBase(kind));
	const out: TargetId[] = [];
	const seen = new Set<TargetId>();
	for (const target of selection) {
		const { fixture: id, cell } = splitTarget(target);
		const fixture = fixtures.get(id);
		const type = fixture && types.get(fixture.fixture_type_id);
		const param = type?.parameters.find(
			(p) => p.direction === 'Output' && parameterKey(p.kind) === key
		);
		if (!param || !type) continue;
		const perCell = param.per_cell && type.cells > 0;
		// A per-cell parameter on a fixture selected whole is every cell of it.
		const reached: TargetId[] = perCell
			? cell !== null
				? [target]
				: Array.from({ length: type.cells }, (_, i) => `${id}@${i + 1}`)
			: [id];
		for (const t of reached) {
			if (!seen.has(t)) {
				seen.add(t);
				out.push(t);
			}
		}
	}
	return out;
}

/** The phases a layer gives a selection of `n`, wrapped into one cycle. */
export function layerPhases(layer: Pick<EffectLayer, 'spread' | 'spread_cycles'>, n: number): number[] {
	return phases(layer.spread, n, layer.spread_cycles ?? 1);
}

/**
 * Phases for targets, either across all of them or **restarting at each fixture**:
 * three strips chasing in step rather than one chase running along all three.
 */
export function targetPhases(
	layer: Pick<EffectLayer, 'spread' | 'spread_cycles'>,
	targets: TargetId[],
	eachFixture: boolean
): number[] {
	if (!eachFixture) return layerPhases(layer, targets.length);
	const groups = new Map<string, number[]>();
	targets.forEach((t, i) => {
		const { fixture } = splitTarget(t);
		groups.set(fixture, [...(groups.get(fixture) ?? []), i]);
	});
	const out = new Array<number>(targets.length).fill(0);
	for (const indices of groups.values()) {
		const offsets = layerPhases(layer, indices.length);
		indices.forEach((at, i) => (out[at] = offsets[i] ?? 0));
	}
	return out;
}

/**
 * One spec per fixture for one layer, sharing an effect id and differing in phase.
 *
 * `t0` is left for the programmer to anchor at the moment it writes, which is the
 * rule `setEffect` already follows.
 */
export function layerSpecs(
	layer: EffectLayer,
	ids: TargetId[],
	template: string | null,
	effectId: string = crypto.randomUUID(),
	eachFixture = false
): Record<TargetId, EffectSpec> {
	const offsets = targetPhases(layer, ids, eachFixture);
	const out: Record<string, EffectSpec> = {};
	ids.forEach((id, i) => {
		out[id] = {
			effect_id: effectId,
			curve: layer.curve,
			rate: layer.rate,
			low: layer.low,
			high: layer.high,
			width: layer.width,
			direction: layer.direction,
			phase: offsets[i] ?? 0,
			spread: layer.spread,
			t0: null,
			template
		};
	});
	return out;
}

/**
 * A whole template over a selection: a layer per parameter, each reaching the
 * fixtures that have it. A layer nothing selected can take is left out rather than
 * refused, so "fire" on a row of single-color pars is still the flicker.
 *
 * **A pixel has no dimmer**, only a color, and nearly every look in the pool is an
 * intensity. So an intensity layer reaches a fixture that has a color and no
 * intensity *as* that color: white scaled by the level, so a chase is white light
 * running along the strip. Only where the template does not say what color that
 * fixture is: fire on a strip is its color layer, which already flickers.
 */
export function templateApply(
	template: Pick<EffectTemplate, 'id' | 'layers'>,
	selection: TargetId[],
	fixtures: Map<string, Fixture>,
	types: Map<string, FixtureType>,
	eachFixture = false
): LayerApply[] {
	const out: LayerApply[] = [];
	const colored = new Set(
		template.layers
			.filter((l) => l.kind === 'ColorRgb')
			.flatMap((l) => havingKind(l.kind, selection, fixtures, types))
	);
	// By fixture: a bar with a master dimmer has a dimmer for all its cells.
	const dimmable = new Set(
		havingKind('Intensity', selection, fixtures, types).map((t) => splitTarget(t).fixture)
	);
	const colorOnly = havingKind('ColorRgb', selection, fixtures, types).filter(
		(id) => !dimmable.has(splitTarget(id).fixture) && !colored.has(id)
	);
	template.layers.forEach((layer, index) => {
		const order = havingKind(layer.kind, selection, fixtures, types);
		if (order.length > 0) {
			out.push({
				layer: index,
				kind: layer.kind,
				order,
				specs: layerSpecs(layer, order, template.id || null, undefined, eachFixture)
			});
		}
		if (layer.kind === 'Intensity' && colorOnly.length > 0) {
			const asColor = intensityAsColor(layer);
			out.push({
				layer: index,
				kind: 'ColorRgb',
				order: colorOnly,
				specs: layerSpecs(asColor, colorOnly, template.id || null, undefined, eachFixture)
			});
		}
	});
	return out;
}

/** An intensity layer said as a color: every level a gray of that brightness. */
export function intensityAsColor(layer: EffectLayer): EffectLayer {
	const gray = (v: EffectLayer['low']): EffectLayer['low'] => {
		const level = v.type === 'Float' || v.type === 'Int' ? Math.min(1, Math.max(0, v.value)) : 1;
		return { type: 'Color', value: { r: level, g: level, b: level, overrides: {} } };
	};
	const curve: EffectLayer['curve'] =
		'Steps' in layer.curve
			? { Steps: layer.curve.Steps.map((s) => ({ ...s, value: gray(s.value) })) }
			: layer.curve;
	return { ...layer, kind: 'ColorRgb', curve, low: gray(layer.low), high: gray(layer.high) };
}

/** What a spread toggle means, for the strip's controls. */
export type PixelSpread = {
	/** Along the strip the way it was patched, or back from the far end. */
	reverse: boolean;
	/** Mirrored about the middle, so both ends move together. */
	mirror: boolean;
};

/** A strip's direction and mirror, as the spread the effect engine already has. */
export function pixelSpread({ reverse, mirror }: PixelSpread): Spread {
	if (mirror) return 'Center';
	return reverse ? 'Reversed' : 'Linear';
}

/** And back, for showing the toggles a layer's spread amounts to. */
export function readPixelSpread(spread: Spread): PixelSpread {
	return { reverse: spread === 'Reversed', mirror: spread === 'Center' };
}

/**
 * A color gradient along a strip: the stops as an eased step list, run across the
 * cells by a linear spread. At rate zero it stands still; any rate scrolls it.
 */
export function gradientCurve(stops: { r: number; g: number; b: number }[]): EffectLayer['curve'] {
	const n = Math.max(stops.length, 1);
	return {
		Steps: stops.map((c, i) => ({
			at: i / n,
			value: { type: 'Color', value: { r: c.r, g: c.g, b: c.b, overrides: {} } },
			easing: 'Linear'
		}))
	};
}

/**
 * A step list drawn on a grid: one column per step, `levels[i]` from 0 to 1 the height
 * clicked. Hard steps jump, soft ones crossfade.
 */
export function gridCurve(levels: number[], soft: boolean): EffectLayer['curve'] {
	const n = Math.max(levels.length, 1);
	return {
		Steps: levels.map((level, i) => ({
			at: i / n,
			value: { type: 'Float', value: Math.min(1, Math.max(0, level)) },
			easing: soft ? 'Linear' : 'Step'
		}))
	};
}

/** The grid a Float step list reads as, for redrawing it. Anything else is `null`. */
export function gridLevels(curve: EffectLayer['curve']): number[] | null {
	if (!('Steps' in curve)) return null;
	const out: number[] = [];
	for (const step of [...curve.Steps].sort((a, b) => a.at - b.at)) {
		if (step.value.type !== 'Float') return null;
		out.push(step.value.value);
	}
	return out;
}

// ── Dragging ──────────────────────────────────────────────────────────────────

/**
 * A rate dragged across the waveform: every 120 px doubles or halves it, so a drag
 * covers 0.05 Hz to 20 Hz without the fine end being unreachable. Measured from where
 * the drag started, never accumulated.
 */
export function draggedRate(startHz: number, dx: number): number {
	const hz = Math.max(startHz, 0.01) * 2 ** (dx / 120);
	return Math.round(Math.min(20, Math.max(0.01, hz)) * 100) / 100;
}

/** A width handle at a fraction of the waveform, kept off both ends. */
export function draggedWidth(fraction: number): number {
	return Math.round(Math.min(0.95, Math.max(0.05, fraction)) * 100) / 100;
}

/**
 * How many cycles the spread covers, from a handle dragged across the preview row: the
 * far edge of the row is four, the near edge none. Snapped to quarters, which is where
 * every useful arrangement lives.
 */
export function draggedCycles(fraction: number): number {
	return Math.round(Math.min(4, Math.max(0, fraction * 4)) * 4) / 4;
}

/** A level dragged up and down the waveform, 0..1 from the bottom. */
export function draggedLevel(fraction: number): number {
	return Math.round(Math.min(1, Math.max(0, fraction)) * 100) / 100;
}
