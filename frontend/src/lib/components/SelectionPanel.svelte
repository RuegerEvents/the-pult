<script lang="ts">
	/**
	 * What is selected, in the order it was selected.
	 *
	 * The order is not decoration. It is what an effect will eventually spread along,
	 * so "the third fixture" has to be something the operator decides rather than
	 * something the patch decided — which is why the spec asks for this list to be
	 * reorderable by hand.
	 *
	 * Selection is deliberately separate from the programmer: clearing one leaves the
	 * other alone, so a value can be parked and then reached again from somewhere else.
	 *
	 * Underneath, the selection is a *question* about the rig rather than a list of
	 * answers — so this panel shows both: what the question is, and what it currently
	 * picks out. A question that keeps up with a rig being rebuilt is the whole point,
	 * and an operator has to be able to see the one to trust the other.
	 */

	import {
		addClause,
		clearSelection,
		freeze,
		isQuery,
		query,
		remove,
		removeClause,
		reorder,
		selection,
		setOrder
	} from '$lib/stores/selection.js';
	import { describe as describeQuery, type Order, type Term } from '$lib/selection.js';
	import { collection } from '$lib/stores/show.js';
	import { fixtureTint } from '$lib/stage.js';

	const fixtures = collection('fixtures');
	const types = collection('fixture_types');

	let building = $state(false);
	/** What the next clause will do to the selection. */
	let combine = $state<'Add' | 'Keep' | 'Drop'>('Add');
	let kind = $state<Term['kind']>('OfType');
	let typeId = $state('');
	let text = $state('');
	let radius = $state(3);
	let angleDeg = $state(20);
	let reach = $state(30);

	/**
	 * The centre of a geometric term.
	 *
	 * Defaults to the middle of what is selected right now, which is almost always
	 * what somebody means by "around here" — and saves typing three numbers to say
	 * something they have already said by clicking.
	 */
	const around = $derived.by(() => {
		const placed = chosen.filter((f) => f.position);
		if (placed.length === 0) return { x: 0, y: 5, z: 0 };
		const sum = placed.reduce(
			(a, f) => {
				const p = 'Point' in f.position! ? f.position!.Point : f.position!.Axial.position;
				return { x: a.x + p.x, y: a.y + p.y, z: a.z + p.z };
			},
			{ x: 0, y: 0, z: 0 }
		);
		return { x: sum.x / placed.length, y: sum.y / placed.length, z: sum.z / placed.length };
	});

	function addTerm() {
		const centre = around;
		const term: Term | null =
			kind === 'Everything'
				? { kind: 'Everything' }
				: kind === 'OfType' && typeId
					? { kind: 'OfType', typeId }
					: kind === 'Named' && text.trim()
						? { kind: 'Named', text }
						: kind === 'Sphere'
							? { kind: 'Sphere', centre, radius }
							: kind === 'Box'
								? {
										kind: 'Box',
										from: { x: centre.x - radius, y: centre.y - radius, z: centre.z - radius },
										to: { x: centre.x + radius, y: centre.y + radius, z: centre.z + radius }
									}
								: kind === 'Cone'
									? {
											// Down the room from front of house, which is the shot an
											// operator means by "that lot over there".
											kind: 'Cone',
											from: { x: centre.x, y: centre.y, z: centre.z + reach },
											direction: { x: 0, y: 0, z: -1 },
											angleDeg,
											reach
										}
									: null;
		if (!term) return;
		addClause(combine, term);
		building = false;
	}

	const ORDERS: { label: string; order: Order }[] = [
		{ label: 'As picked', order: { kind: 'Manual' } },
		{ label: 'Left to right', order: { kind: 'ByAxis', axis: 'x' } },
		{ label: 'Right to left', order: { kind: 'ByAxis', axis: 'x', descending: true } },
		{ label: 'Upstage to down', order: { kind: 'ByAxis', axis: 'z' } },
		{ label: 'By height', order: { kind: 'ByAxis', axis: 'y' } },
		{ label: 'By name', order: { kind: 'ByName' } }
	];
	const orderLabel = $derived(
		ORDERS.find((o) => JSON.stringify(o.order) === JSON.stringify($query.order))?.label ?? 'As picked'
	);

	/// The selection in its own order, skipping anything that has left the rig.
	const chosen = $derived(
		$selection.map((id) => $fixtures.find((f) => f.id === id)).filter((f) => f !== undefined)
	);

	let dragging = $state<number | null>(null);
	let over = $state<number | null>(null);

	function drop() {
		if (dragging !== null && over !== null) reorder(dragging, over);
		dragging = null;
		over = null;
	}
</script>

<div class="panel">
	<header>
		<h2>Selection</h2>
		<span class="count">{$selection.length}</span>
		<span class="spacer"></span>
		<button class="ghost" onclick={() => (building = !building)}>{building ? 'Cancel' : '+ Rule'}</button>
		{#if $isQuery}
			<!-- The way out of a question that is nearly right: freeze the answer and
			     edit it by hand. -->
			<button class="ghost" title="Stop following the rig and keep these fixtures" onclick={freeze}>
				Freeze
			</button>
		{/if}
		<button class="ghost" disabled={$selection.length === 0} onclick={clearSelection}>Clear</button>
	</header>

	{#if $query.clauses.length > 0}
		<!-- What the selection is *asking*, as opposed to what it currently answers.
		     A rule list rather than prose, because each one can be taken away. -->
		<div class="rules">
			{#each $query.clauses as clause, i (i)}
				<span class="rule" class:drop={clause.combine === 'Drop'} class:keep={clause.combine === 'Keep'}>
					{describeQuery({ clauses: [clause], order: $query.order }, $types)}
					<button class="rule-x" aria-label="Remove this rule" onclick={() => removeClause(i)}>×</button>
				</span>
			{/each}
		</div>
	{/if}

	{#if building}
		<div class="builder">
			<select class="select" bind:value={combine}>
				<option value="Add">Add</option>
				<option value="Keep">Of those</option>
				<option value="Drop">Except</option>
			</select>
			<select class="select" bind:value={kind}>
				<option value="Everything">everything</option>
				<option value="OfType">of a type</option>
				<option value="Named">named…</option>
				<option value="Sphere">within a radius</option>
				<option value="Box">in a region</option>
				<option value="Cone">in a beam</option>
			</select>

			{#if kind === 'OfType'}
				<select class="select" bind:value={typeId}>
					<option value="">choose a type…</option>
					{#each $types as t (t.id)}
						<option value={t.id}>{t.name}</option>
					{/each}
				</select>
			{:else if kind === 'Named'}
				<input class="input" placeholder="part of the name" bind:value={text} />
			{:else if kind === 'Sphere' || kind === 'Box'}
				<label class="num-field">
					<input class="input narrow" type="number" min="0.1" step="0.5" bind:value={radius} />
					<span>m around the selection</span>
				</label>
			{:else if kind === 'Cone'}
				<label class="num-field">
					<input class="input narrow" type="number" min="1" max="89" bind:value={angleDeg} />
					<span>° half-angle,</span>
					<input class="input narrow" type="number" min="1" bind:value={reach} />
					<span>m reach</span>
				</label>
			{/if}

			<button class="btn btn-primary" onclick={addTerm}>Add rule</button>
		</div>
	{/if}

	<div class="ordering">
		<span>Order</span>
		<select
			class="select"
			value={orderLabel}
			onchange={(e) => {
				const picked = ORDERS.find((o) => o.label === e.currentTarget.value);
				if (picked) setOrder(picked.order);
			}}
		>
			{#each ORDERS as option (option.label)}
				<option value={option.label}>{option.label}</option>
			{/each}
		</select>
	</div>

	<div class="list">
		{#if chosen.length === 0}
			<p class="empty">Nothing selected. Click a fixture in the rig or on the plan.</p>
		{:else}
			{#each chosen as fixture, index (fixture.id)}
				<div
					class="row"
					class:over={over === index && dragging !== index}
					draggable="true"
					role="listitem"
					ondragstart={() => (dragging = index)}
					ondragover={(e) => {
						e.preventDefault();
						over = index;
					}}
					ondragend={drop}
					ondrop={(e) => {
						e.preventDefault();
						drop();
					}}
				>
					<span class="grip" aria-hidden="true">⠿</span>
					<span class="position mono">{index + 1}</span>
					<span class="dot" style:background={fixtureTint(fixture)}></span>
					<span class="name">{fixture.name}</span>
					<button
						class="icon"
						title="Take it out of the selection"
						aria-label="Remove {fixture.name}"
						onclick={() => remove(fixture.id)}
					>
						✕
					</button>
				</div>
			{/each}
		{/if}
	</div>
</div>

<style>
	.panel {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--line);
		flex: none;
	}
	h2 {
		font-size: var(--font-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-dim);
	}
	.count {
		color: var(--accent);
		font-family: monospace;
		font-size: var(--font-xs);
	}
	.spacer {
		flex: 1;
	}

	.list {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 6px;
	}

	.row {
		display: grid;
		grid-template-columns: auto 20px 10px 1fr auto;
		align-items: center;
		gap: var(--pad);
		padding: 4px 6px;
		/* Reordered by dragging with a finger, so a row has to be one. */
		min-height: var(--hit);
		border-radius: 3px;
		border-top: 2px solid transparent;
		color: var(--text);
		font-size: var(--font-sm);
		cursor: grab;
	}
	.row:hover {
		background: var(--bg-hover);
	}
	.row.over {
		border-top-color: var(--accent);
	}

	.grip {
		color: var(--text-faint);
		font-size: var(--font-xs);
	}
	.position {
		color: var(--text-faint);
		font-size: var(--font-xs);
		text-align: right;
	}
	.mono {
		font-family: monospace;
	}
	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
		border: 1px solid #8a8a8a;
	}
	.name {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.icon {
		background: none;
		border: none;
		color: var(--text-faint);
		font: inherit;
		font-size: var(--font-xs);
		line-height: 1;
		cursor: pointer;
	}
	.icon:hover {
		color: var(--bad);
	}

	.empty {
		color: var(--text-faint);
		font-size: var(--font-sm);
		font-style: italic;
		padding: 8px 6px;
	}

	.ghost {
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius);
		color: #bbb;
		padding: 3px 10px;
		font: inherit;
		font-size: var(--font-xs);
		cursor: pointer;
	}
	.ghost:hover:not(:disabled) {
		border-color: var(--line-input);
		color: var(--text-bright);
	}
	.ghost:disabled {
		color: var(--text-faint);
		cursor: not-allowed;
	}
	/* The question, as a row of rules that can each be taken away. */
	.rules {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		padding: 0 10px 8px;
	}
	.rule {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		font-size: var(--font-xs);
		padding: 2px 4px 2px 8px;
		border-radius: 999px;
		border: 1px solid var(--accent);
		color: var(--accent);
	}
	/* Narrowing and removing read differently from adding, so a rule list can be
	   scanned rather than read. */
	.rule.keep {
		border-color: var(--text-dim);
		color: var(--text-dim);
	}
	.rule.drop {
		border-color: var(--bad);
		color: var(--bad);
	}
	.rule-x {
		background: none;
		border: none;
		color: inherit;
		font: inherit;
		cursor: pointer;
		padding: 0 2px;
		opacity: 0.7;
	}
	.rule-x:hover {
		opacity: 1;
	}

	.builder {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		padding: 0 10px 10px;
	}
	.num-field {
		display: flex;
		align-items: center;
		gap: 5px;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
	.builder .input.narrow {
		width: 4.5rem;
	}

	.ordering {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 0 10px 8px;
		color: var(--text-dim);
		font-size: var(--font-xs);
	}
</style>
