<script lang="ts">
	/**
	 * What the flows remember: occupancy, counts, a maze that is open or closed.
	 *
	 * Setting a value stays live while the panel is locked, the way Tap does on a speed
	 * master: putting a count right after a group walked out through the entrance is done
	 * during a show, at speed. What the lock covers is how the show is *built* — making a
	 * variable, its bounds, deleting one.
	 *
	 * Values go through the `variable.*` station RPCs and never through a field write,
	 * so a reset is not a row in the History panel and Ctrl-Z cannot take it back — the
	 * same as a flow's own writes. See `types::variable`.
	 */

	import { focusOnMount } from '$lib/actions.js';
	import type { Variable, VariableKind } from '$lib/generated/index.js';
	import { currentValue, variableLabel } from '$lib/flow.js';
	import { collection } from '$lib/stores/show.js';
	import { editing } from '$lib/stores/editing.js';
	import { getClientContext, getDataContext } from '$lib/ws/context.js';

	const data = getDataContext();
	const client = getClientContext();
	const variables = collection('variables');
	const unlocked = editing('variables');

	let creating = $state(false);
	let newName = $state('');
	let newKind = $state<VariableKind>('Number');
	let problem = $state<string | null>(null);

	async function ask(method: string, args: Record<string, unknown>) {
		try {
			problem = null;
			await client.call(method, args);
		} catch (e) {
			problem = e instanceof Error ? e.message : String(e);
		}
	}

	const add = (v: Variable, by: number) => ask('variable.add', { variableId: v.id, by });
	const toggle = (v: Variable) => ask('variable.toggle', { variableId: v.id });
	const reset = (v: Variable) => ask('variable.reset', { variableId: v.id });
	const setTo = (v: Variable, value: number) => ask('variable.set', { variableId: v.id, value });

	async function createVariable() {
		const name = newName.trim();
		if (!name) return;
		await data.variables.create({
			id: crypto.randomUUID(),
			name,
			kind: newKind,
			initial: 0,
			// A count of people starts at nobody and cannot go below it.
			min: newKind === 'Number' ? 0 : null,
			max: null,
			value: null
		});
		newName = '';
		creating = false;
	}

	const bound = (text: string) => (text.trim() === '' ? null : Number(text));
</script>

<div class="variables">
	<header class="head">
		<h2>Variables</h2>
		{#if $unlocked}
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>
				{creating ? 'Cancel' : '+ Variable'}
			</button>
		{/if}
	</header>

	{#if creating && $unlocked}
		<form class="new" onsubmit={(e) => { e.preventDefault(); createVariable(); }}>
			<input class="input" placeholder="Name, e.g. Area 2 inside" bind:value={newName} use:focusOnMount />
			<select class="input" bind:value={newKind}>
				<option value="Number">a number</option>
				<option value="Flag">on or off</option>
			</select>
			<button class="btn btn-primary" type="submit">Create</button>
		</form>
	{/if}

	{#if problem}<p class="problem">{problem}</p>{/if}

	{#if $variables.length === 0}
		<p class="empty">
			No variables. A variable is something a flow remembers — how many groups are in an
			area, whether the maze is open — and a flow can read it, change it, and let a pulse
			through only while it says so.
		</p>
	{/if}

	{#each $variables as variable (variable.id)}
		<section class="variable" class:on={currentValue(variable) !== 0}>
			<div class="top">
				{#if $unlocked}
					<input
						class="input name"
						value={variable.name}
						onchange={(e) => data.variables.byId(variable.id).name.set(e.currentTarget.value)}
					/>
				{:else}
					<span class="name">{variable.name}</span>
				{/if}
				<span class="value" title={variable.value === null ? 'At its initial value' : ''}>
					{variableLabel(variable)}
				</span>
				{#if $unlocked}
					<button
						class="btn btn-danger btn-icon"
						title="Delete {variable.name}"
						onclick={() => data.variables.byId(variable.id).delete()}
					>×</button>
				{/if}
			</div>

			<div class="controls">
				{#if variable.kind === 'Flag'}
					<button class="btn big" onclick={() => toggle(variable)}>
						{currentValue(variable) !== 0 ? 'Turn off' : 'Turn on'}
					</button>
				{:else}
					<button class="btn big" aria-label="Subtract one" onclick={() => add(variable, -1)}>−</button>
					<button class="btn big" aria-label="Add one" onclick={() => add(variable, 1)}>+</button>
					<input
						class="input narrow"
						type="number"
						aria-label="Set {variable.name} to"
						placeholder="set"
						onchange={(e) => {
							if (!Number.isNaN(e.currentTarget.valueAsNumber)) setTo(variable, e.currentTarget.valueAsNumber);
							e.currentTarget.value = '';
						}}
					/>
				{/if}
				<button class="btn" onclick={() => reset(variable)} disabled={variable.value === null}>
					Reset
				</button>
			</div>

			{#if $unlocked}
				<div class="build">
					<label>
						starts at
						<input
							class="input narrow"
							type="number"
							value={variable.initial}
							onchange={(e) =>
								data.variables.byId(variable.id).initial.set(e.currentTarget.valueAsNumber || 0)}
						/>
					</label>
					{#if variable.kind === 'Number'}
						<label>
							at least
							<input
								class="input narrow"
								type="number"
								placeholder="—"
								value={variable.min ?? ''}
								onchange={(e) => data.variables.byId(variable.id).min.set(bound(e.currentTarget.value))}
							/>
						</label>
						<label>
							at most
							<input
								class="input narrow"
								type="number"
								placeholder="—"
								value={variable.max ?? ''}
								onchange={(e) => data.variables.byId(variable.id).max.set(bound(e.currentTarget.value))}
							/>
						</label>
					{/if}
				</div>
			{/if}
		</section>
	{/each}

	<p class="note">
		Every value goes back to where it starts when the show is opened, carries over to the
		spare if this console fails, and is never in History.
	</p>
</div>

<style>
	.variables { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
	.head { display: flex; align-items: center; justify-content: space-between; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.new { display: flex; gap: 6px; flex-wrap: wrap; }
	.variable { border: 1px solid #2e2e2e; border-left: 3px solid #3a3a3a; border-radius: 4px; padding: 8px 10px; display: flex; flex-direction: column; gap: 8px; }
	.variable.on { border-left-color: #22c55e; }
	.top { display: flex; align-items: center; gap: 8px; }
	.name { flex: 1; font-size: 14px; }
	.input.name { flex: 1; }
	.value { font-variant-numeric: tabular-nums; font-size: 18px; font-weight: 600; min-width: 2.5em; text-align: right; }
	.controls { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
	/* Finger-sized: this is pressed on a phone at the maze entrance. */
	.big { min-width: 48px; min-height: 40px; font-size: 18px; }
	.narrow { width: 72px; }
	.build { display: flex; gap: 12px; flex-wrap: wrap; color: #888; font-size: 12px; }
	.build label { display: flex; align-items: center; gap: 5px; }
	.empty, .note { color: #777; font-size: 12px; line-height: 1.4; }
	.note { font-style: italic; }
	.problem { color: #e05555; font-size: 12px; }
</style>
