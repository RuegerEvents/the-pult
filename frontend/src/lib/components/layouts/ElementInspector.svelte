<script lang="ts">
	/**
	 * One element of a layout, in the Inspector: what it points at, what it says, how
	 * it is drawn, how it decays, its colors for a number, and what pressing it does on
	 * the desk.
	 *
	 * Every change rewrites the layout's element list in one gesture, so one change is
	 * one Ctrl-Z however many fields of the element it touched. Under the Show lock it
	 * shows everything and changes nothing.
	 */

	import type { ColorStop, ElementPress, LayoutElement, ParameterKind, VariableOp } from '$lib/generated/index.js';
	import type { ElementRow } from '$lib/kinds/layouts.kind.js';
	import Pick from '$lib/components/objects/Pick.svelte';
	import { flagOpFrom, flagOpOf, type FlagOp } from '$lib/flow.js';
	import {
		boxOf,
		duplicated,
		inputsOf,
		layoutButtons,
		newElement,
		placed,
		resolveDecay,
		TARGET_KINDS,
		targetName,
		toBack,
		toFront,
		withPoints,
		type TargetKind
	} from '$lib/layoutView.js';
	import { controls, parameterKey, parameterName } from '$lib/patch.js';
	import { selectSubject } from '$lib/objects.js';
	import { editElements, layouts } from '$lib/stores/layouts.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { surfaceShow } from '$lib/stores/surface.js';
	import { addToast } from '$lib/toasts.js';

	let { rows }: { rows: ElementRow[] } = $props();

	const layoutOf = (row: ElementRow) => $layouts.find((l) => l.id === row.layoutId) ?? null;

	const patch = (row: ElementRow, change: Partial<LayoutElement>) =>
		editElements(row.layoutId, (elements) => elements.map((e) => (e.id === row.id ? { ...e, ...change } : e)));

	const restack = (row: ElementRow, how: typeof toFront) => editElements(row.layoutId, (elements) => how(elements, row.id));

	/**
	 * Point an element at a different kind of thing: the first of that kind the show has,
	 * kept where it is. A press that made sense for the old target does not carry over.
	 */
	function retarget(row: ElementRow, kind: TargetKind) {
		const layout = layoutOf(row);
		if (!layout || kind === row.element.target.kind) return;
		const box = boxOf(row.element);
		const fixtureId = row.element.target.kind === 'Parameter' ? row.element.target.fixture_id : null;
		const made = newElement(kind, $surfaceShow, { x: box.x + box.w / 2, y: box.y + box.h / 2 }, layout, layout.grid, fixtureId, row.id);
		if (!made) {
			addToast('The show has nothing that kind of element could point at yet.');
			return;
		}
		const kept = { ...made, label: row.element.label, decay: row.element.decay, stops: row.element.stops };
		void editElements(row.layoutId, (elements) => elements.map((e) => (e.id === row.id ? kept : e)));
	}

	/** Every parameter of a fixture worth pointing at: what it reports, then what it is driven on. */
	function parametersOf(fixtureId: string): { key: string; kind: ParameterKind; label: string }[] {
		const fixture = $surfaceShow.fixtures.find((f) => f.id === fixtureId);
		const type = fixture && $surfaceShow.types.find((t) => t.id === fixture.fixture_type_id);
		if (!type) return [];
		return [
			...inputsOf(type).map(({ kind }) => ({ key: parameterKey(kind), kind, label: `${parameterName(fixture, kind)} (reported)` })),
			...controls(type).map(({ kind }) => ({ key: parameterKey(kind), kind, label: parameterName(fixture, kind) }))
		];
	}

	function pickParameter(row: ElementRow, fixtureId: string, key?: string) {
		const options = parametersOf(fixtureId);
		const chosen = options.find((o) => o.key === key) ?? options[0];
		if (!chosen) return;
		void patch(row, { target: { kind: 'Parameter', fixture_id: fixtureId, parameter: chosen.kind } });
	}

	// ── A line's points, as text ────────────────────────────────────────────────

	const pointsText = (points: [number, number][]) => points.map(([x, y]) => `${round(x)},${round(y)}`).join('  ');
	const round = (n: number) => Math.round(n * 100) / 100;

	function setPoints(row: ElementRow, text: string) {
		const points = text
			.trim()
			.split(/\s+/)
			.map((pair) => pair.split(',').map(Number))
			.filter((p) => p.length === 2 && p.every(Number.isFinite)) as [number, number][];
		if (points.length < 2) {
			addToast('A line needs at least two points, written as x,y with a space between them.');
			return;
		}
		void editElements(row.layoutId, (elements) => elements.map((e) => (e.id === row.id ? withPoints(e, points) : e)));
	}

	function setBox(row: ElementRow, field: 'x' | 'y' | 'w' | 'h', value: number) {
		if (!Number.isFinite(value) || ((field === 'w' || field === 'h') && value <= 0)) return;
		const box = { ...boxOf(row.element), [field]: value };
		void editElements(row.layoutId, (elements) => elements.map((e) => (e.id === row.id ? placed(e, box) : e)));
	}

	// ── What pressing it does ───────────────────────────────────────────────────

	/** The press options a target can take, as `value → words`. */
	function pressChoices(row: ElementRow): { value: string; label: string }[] {
		const target = row.element.target;
		const none = { value: 'None', label: 'only shows it' };
		if (target.kind === 'Variable') {
			const variable = $surfaceShow.variables.find((v) => v.id === target.variable_id);
			if (variable?.kind === 'Flag') {
				return [none, { value: 'Toggle', label: 'pressed, toggles it' }, { value: 'On', label: 'pressed, turns it on' }, { value: 'Off', label: 'pressed, turns it off' }];
			}
			return [
				none,
				{ value: 'Up', label: 'pressed, adds' },
				{ value: 'Down', label: 'pressed, subtracts' },
				{ value: 'Set', label: 'pressed, sets it to' },
				{ value: 'Reset', label: 'pressed, resets it' }
			];
		}
		if (target.kind === 'Timeline') {
			return [none, { value: 'Play', label: 'pressed, plays or stops it' }, { value: 'Start', label: 'pressed, plays from the top or stops it' }];
		}
		if (target.kind === 'Parameter') return [none, { value: 'Select', label: 'pressed, selects the fixture' }];
		return [none];
	}

	function pressTag(row: ElementRow): string {
		const press = row.element.press;
		if (!press) return 'None';
		if (press.kind === 'Select') return 'Select';
		if (press.kind === 'Timeline') return press.from_start ? 'Start' : 'Play';
		const op = press.op;
		const target = row.element.target;
		const flag = target.kind === 'Variable' && $surfaceShow.variables.find((v) => v.id === target.variable_id)?.kind === 'Flag';
		if (flag) return flagOpOf(op);
		if (op === 'Toggle' || op === 'Reset') return op;
		return 'Set' in op ? 'Set' : op.Add >= 0 ? 'Up' : 'Down';
	}

	const amountOf = (op: VariableOp): number => (typeof op === 'string' ? 1 : 'Set' in op ? op.Set : Math.abs(op.Add));

	function opFrom(tag: string, amount: number): VariableOp {
		if (tag === 'On' || tag === 'Off') return flagOpFrom(tag as FlagOp);
		if (tag === 'Toggle' || tag === 'Reset') return tag;
		if (tag === 'Set') return { Set: amount };
		return { Add: tag === 'Up' ? amount : -amount };
	}

	function setPress(row: ElementRow, tag: string, amount = 1) {
		let press: ElementPress | null;
		if (tag === 'None') press = null;
		else if (tag === 'Select') press = { kind: 'Select' };
		else if (tag === 'Play' || tag === 'Start') press = { kind: 'Timeline', from_start: tag === 'Start' };
		else press = { kind: 'Variable', op: opFrom(tag, amount) };
		void patch(row, { press });
	}

	// ── Stops ───────────────────────────────────────────────────────────────────

	const setStops = (row: ElementRow, stops: ColorStop[]) => patch(row, { stops });

	function addStop(row: ElementRow) {
		const stops = row.element.stops;
		const at = stops.length ? Math.max(...stops.map((s) => s.at)) + 1 : 0;
		void setStops(row, [...stops, { at, color: stops.length ? '#ef4444' : '#22c55e' }]);
	}

	/** Whether a target shows a number or an on/off reading, which is what stops and decay are for. */
	const reads = (row: ElementRow) => ['Parameter', 'Variable', 'Timeline'].includes(row.element.target.kind);
	const drawn = (row: ElementRow) => ['Shape', 'Line', 'Text'].includes(row.element.target.kind);
</script>

{#each rows as row (row.id)}
	{@const element = row.element}
	{@const target = element.target}
	{@const layout = layoutOf(row)}
	{@const decay = resolveDecay(element.decay, layout?.decay)}
	{@const box = boxOf(element)}
	<div class="fields">
		{#if rows.length > 1}<span class="wide name">{row.name}</span>{/if}

		<span class="label">Shows</span>
		<div class="inline">
			<select class="input" disabled={!$canEdit} value={target.kind} onchange={(e) => retarget(row, e.currentTarget.value as TargetKind)}>
				{#each TARGET_KINDS as k (k.kind)}<option value={k.kind}>{k.label}</option>{/each}
			</select>
		</div>

		{#if target.kind === 'Parameter'}
			<span class="label">Fixture</span>
			<div class="inline">
				<Pick class="input" rows={$surfaceShow.fixtures} value={target.fixture_id} disabled={!$canEdit} onpick={(id) => id && pickParameter(row, id)} />
				<select
					class="input"
					disabled={!$canEdit}
					value={parameterKey(target.parameter)}
					aria-label="Parameter"
					onchange={(e) => pickParameter(row, target.fixture_id, e.currentTarget.value)}
				>
					{#each parametersOf(target.fixture_id) as p (p.key)}<option value={p.key}>{p.label}</option>{/each}
				</select>
			</div>
		{:else if target.kind === 'Variable'}
			<span class="label">Variable</span>
			<Pick class="input" rows={$surfaceShow.variables} value={target.variable_id} disabled={!$canEdit} onpick={(id) => id && patch(row, { target: { kind: 'Variable', variable_id: id }, press: null })} />
		{:else if target.kind === 'Timeline'}
			<span class="label">Timeline</span>
			<Pick class="input" rows={$surfaceShow.timelines} value={target.timeline_id} disabled={!$canEdit} onpick={(id) => id && patch(row, { target: { kind: 'Timeline', timeline_id: id } })} />
		{:else if target.kind === 'FlowButton'}
			<span class="label">Button</span>
			<Pick class="input" rows={layoutButtons($surfaceShow)} value={target.node_id} disabled={!$canEdit} onpick={(id) => id && patch(row, { target: { kind: 'FlowButton', node_id: id } })} />
		{:else if target.kind === 'Line'}
			<span class="label">Points</span>
			<input
				class="input"
				disabled={!$canEdit}
				value={pointsText(target.points)}
				aria-label="Points, as x,y pairs"
				title="x,y pairs in canvas units, with a space between them"
				onchange={(e) => setPoints(row, e.currentTarget.value)}
			/>
		{/if}

		<span class="label">Label</span>
		<div class="inline">
			<input
				class="input grow"
				disabled={!$canEdit}
				value={element.label}
				placeholder={targetName(target, $surfaceShow) || (target.kind === 'Text' ? 'Words' : 'No label')}
				aria-label="Label"
				onchange={(e) => patch(row, { label: e.currentTarget.value })}
			/>
			<label class="check">
				<input type="checkbox" disabled={!$canEdit} checked={element.show_label} onchange={(e) => patch(row, { show_label: e.currentTarget.checked })} />
				shown
			</label>
		</div>

		{#if target.kind !== 'Line' && target.kind !== 'Text'}
			<span class="label">Outline</span>
			<select class="input" disabled={!$canEdit} value={element.shape} onchange={(e) => patch(row, { shape: e.currentTarget.value as LayoutElement['shape'] })}>
				<option value="Rect">rectangle</option>
				<option value="Ellipse">ellipse</option>
			</select>
		{/if}

		{#if drawn(row) || target.kind === 'FlowButton'}
			<span class="label">Color</span>
			<div class="inline">
				<input type="color" disabled={!$canEdit} value={element.color || (target.kind === 'FlowButton' ? '#2d3a4f' : '#9aa4b2')} aria-label="Color" onchange={(e) => patch(row, { color: e.currentTarget.value })} />
			</div>
		{/if}

		<span class="label">Place</span>
		<div class="inline">
			{#each [['x', box.x], ['y', box.y], ['w', box.w], ['h', box.h]] as [field, value] (field)}
				<label class="check">
					{field}
					<input
						class="input tiny"
						type="number"
						disabled={!$canEdit}
						value={Math.round((value as number) * 100) / 100}
						onchange={(e) => setBox(row, field as 'x' | 'y' | 'w' | 'h', e.currentTarget.valueAsNumber)}
					/>
				</label>
			{/each}
		</div>

		{#if reads(row)}
			<span class="wide section">{target.kind === 'Timeline' ? 'Running and stopped' : 'On, and fading after it goes off'}</span>
			<span class="label">On</span>
			<div class="inline">
				<input
					type="color"
					disabled={!$canEdit}
					value={decay.active}
					aria-label="Active color"
					onchange={(e) => patch(row, { decay: { ...element.decay, active_color: e.currentTarget.value } })}
				/>
				{#if element.decay.active_color}
					{#if $canEdit}<button class="btn btn-ghost" onclick={() => patch(row, { decay: { ...element.decay, active_color: null } })}>the layout's</button>{/if}
				{:else}
					<span class="hint">the layout's</span>
				{/if}
			</div>
			<span class="label">{target.kind === 'Timeline' ? 'Stopped' : 'Idle'}</span>
			<div class="inline">
				<input
					type="color"
					disabled={!$canEdit}
					value={decay.idle}
					aria-label="Idle color"
					onchange={(e) => patch(row, { decay: { ...element.decay, idle_color: e.currentTarget.value } })}
				/>
				{#if element.decay.idle_color}
					{#if $canEdit}<button class="btn btn-ghost" onclick={() => patch(row, { decay: { ...element.decay, idle_color: null } })}>the layout's</button>{/if}
				{:else}
					<span class="hint">the layout's</span>
				{/if}
			</div>
			{#if target.kind !== 'Timeline'}
				<span class="label">Fades</span>
				<div class="inline">
					<input
						class="input narrow"
						type="number"
						min="0"
						disabled={!$canEdit}
						value={decay.ms / 1000}
						aria-label="Decay time in seconds"
						onchange={(e) => {
							const s = e.currentTarget.valueAsNumber;
							if (Number.isFinite(s) && s >= 0) void patch(row, { decay: { ...element.decay, decay_ms: Math.round(s * 1000) } });
						}}
					/>
					<span class="hint">s</span>
					{#if element.decay.decay_ms !== null && element.decay.decay_ms !== undefined}
						{#if $canEdit}<button class="btn btn-ghost" onclick={() => patch(row, { decay: { ...element.decay, decay_ms: null } })}>the layout's</button>{/if}
					{:else}
						<span class="hint">the layout's</span>
					{/if}
				</div>
			{/if}
		{/if}

		{#if target.kind === 'Parameter' || target.kind === 'Variable'}
			<span class="wide section">A number's color, from a value up{element.stops.length ? '' : ' — none: drawn idle with the figure on it'}</span>
			{#each element.stops as stop, s (s)}
				<span class="label">≥</span>
				<div class="inline">
					<input
						class="input narrow"
						type="number"
						disabled={!$canEdit}
						value={stop.at}
						aria-label="From"
						onchange={(e) => {
							const at = e.currentTarget.valueAsNumber;
							if (Number.isFinite(at)) void setStops(row, element.stops.map((x, i) => (i === s ? { ...x, at } : x)));
						}}
					/>
					<input
						type="color"
						disabled={!$canEdit}
						value={stop.color}
						aria-label="Color"
						onchange={(e) => setStops(row, element.stops.map((x, i) => (i === s ? { ...x, color: e.currentTarget.value } : x)))}
					/>
					{#if $canEdit}
						<button class="btn btn-ghost" aria-label="Remove this stop" onclick={() => setStops(row, element.stops.filter((_, i) => i !== s))}>×</button>
					{/if}
				</div>
			{/each}
			{#if $canEdit}
				<span class="label"></span>
				<div class="inline"><button class="btn btn-ghost" onclick={() => addStop(row)}>+ Stop</button></div>
			{/if}
		{/if}

		{#if target.kind === 'FlowButton'}
			<span class="label">Press</span>
			<span class="hint">presses the flow's button</span>
			<span class="wide hint">{layout?.operate_key ? 'On the desk, and on the operating link.' : 'On the desk. An operating link presses it too, once the layout has one (Open on a phone).'}</span>
		{:else if !drawn(row)}
			{@const choices = pressChoices(row)}
			{@const tag = pressTag(row)}
			<span class="label">Press</span>
			<div class="inline">
				<select
					class="input"
					disabled={!$canEdit}
					value={tag}
					aria-label="Pressing it, on the desk"
					onchange={(e) => setPress(row, e.currentTarget.value, element.press?.kind === 'Variable' ? amountOf(element.press.op) : 1)}
				>
					{#each choices as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
				</select>
				{#if element.press?.kind === 'Variable' && (tag === 'Up' || tag === 'Down' || tag === 'Set')}
					{@const op = element.press.op}
					<input
						class="input narrow"
						type="number"
						disabled={!$canEdit}
						value={amountOf(op)}
						aria-label="Amount"
						onchange={(e) => setPress(row, tag, e.currentTarget.valueAsNumber || 0)}
					/>
				{/if}
			</div>
			<span class="wide hint">
				{#if !layout?.operate_key}On the desk only: a phone presses nothing until the layout has an operating link (Open on a phone).
				{:else if element.press?.kind === 'Select'}On the desk only: a phone has no selection to make.
				{:else}On the desk, and on the operating link.{/if}
			</span>
		{/if}

		{#if $canEdit && rows.length === 1 && layout}
			<div class="wide inline actions">
				<button class="btn btn-ghost" onclick={() => restack(row, toFront)}>To front</button>
				<button class="btn btn-ghost" onclick={() => restack(row, toBack)}>To back</button>
				<button
					class="btn btn-ghost"
					onclick={async () => {
						const copy = duplicated(element, layout, layout.grid);
						await editElements(row.layoutId, (elements) => [...elements, copy]);
						selectSubject({ kind: 'layout-element', id: copy.id });
					}}>Duplicate</button
				>
				<span class="spacer"></span>
				<button class="btn btn-danger" onclick={() => editElements(row.layoutId, (elements) => elements.filter((e) => e.id !== row.id))}>Delete</button>
			</div>
		{/if}
	</div>
{/each}

<style>
	.fields {
		display: grid;
		grid-template-columns: 64px minmax(0, 1fr);
		align-items: center;
		gap: 8px;
		font-size: 12px;
		color: #999;
	}
	.wide {
		grid-column: 1 / -1;
	}
	.name {
		color: #ddd;
		font-weight: 600;
	}
	.section {
		margin-top: 6px;
		color: #bbb;
	}
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}
	.grow {
		flex: 1;
		min-width: 100px;
	}
	.narrow {
		width: 72px;
	}
	.tiny {
		width: 58px;
	}
	.check {
		display: flex;
		align-items: center;
		gap: 4px;
		color: #999;
	}
	.hint {
		color: #777;
	}
	.actions {
		margin-top: 6px;
	}
	.spacer {
		flex: 1;
	}
</style>
