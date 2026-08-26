<script lang="ts">
	import { onMount } from 'svelte';
	import { getDataContext } from '$lib/ws/context.js';
	import type {
		Fixture,
		FixtureType,
		ParameterDefinition,
		Sequence,
		Trigger,
		TriggerCondition
	} from '$lib/generated/index.js';
	import { focusOnMount } from '$lib/actions.js';
	import { kindLabel, parameterKindLabel } from '$lib/patch.js';

	const data = getDataContext();

	let triggers = $state<Trigger[]>([]);
	let fixtures = $state<Fixture[]>([]);
	let types = $state<FixtureType[]>([]);
	let sequences = $state<Sequence[]>([]);
	let creating = $state(false);
	let newName = $state('');

	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);

	/// The parameters of one fixture, inputs first: a trigger almost always watches
	/// something the rig reports rather than something the console drives.
	function parametersOf(fixtureId: string): ParameterDefinition[] {
		const fixture = fixtures.find((f) => f.id === fixtureId);
		const parameters = fixture ? (typeOf(fixture)?.parameters ?? []) : [];
		return [...parameters].sort((a, b) =>
			a.direction === b.direction ? 0 : a.direction === 'Input' ? -1 : 1
		);
	}

	const watchedFixture = (trigger: Trigger) => trigger.source.Parameter.fixture_id;
	const watchedParameter = (trigger: Trigger) =>
		parameterKindLabel(trigger.source.Parameter.parameter);

	/// Is the watched parameter a switch or a level? Decides which conditions apply.
	function watchesABoolean(trigger: Trigger): boolean {
		const kind = trigger.source.Parameter.parameter;
		const label = kindLabel(kind);
		return ['Contact', 'Switch'].includes(label);
	}

	const conditionLabel = (condition: TriggerCondition): string =>
		typeof condition === 'string'
			? condition
			: 'Above' in condition
				? `Above ${condition.Above}`
				: `Below ${condition.Below}`;

	function conditionFromLabel(label: string, threshold: number): TriggerCondition {
		if (label === 'Above') return { Above: threshold };
		if (label === 'Below') return { Below: threshold };
		return label as TriggerCondition;
	}

	const thresholdOf = (condition: TriggerCondition): number =>
		typeof condition === 'string' ? 0 : 'Above' in condition ? condition.Above : condition.Below;

	const actionKind = (trigger: Trigger) => Object.keys(trigger.action)[0];

	function actionSequence(trigger: Trigger): string | null {
		if ('GoNext' in trigger.action) return trigger.action.GoNext.sequence_id;
		if ('GoToCue' in trigger.action) return trigger.action.GoToCue.sequence_id;
		return null;
	}

	async function createTrigger() {
		const name = newName.trim();
		const fixture = fixtures[0];
		const parameter = fixture ? parametersOf(fixture.id)[0] : undefined;
		if (!name || !fixture || !parameter || sequences.length === 0) return;

		await data.triggers.create({
			id: crypto.randomUUID(),
			name,
			source: { Parameter: { fixture_id: fixture.id, parameter: parameter.kind } },
			condition: 'RisingEdge',
			action: { GoNext: { sequence_id: sequences[0].id } },
			delay_ms: 0,
			enabled: true,
			pending: false,
			last_fired_at: null
		});
		newName = '';
		creating = false;
	}

	async function watch(trigger: Trigger, fixtureId: string, parameterLabel?: string) {
		const parameters = parametersOf(fixtureId);
		const chosen =
			parameters.find((p) => parameterKindLabel(p.kind) === parameterLabel) ?? parameters[0];
		if (!chosen) return;
		await data.triggers
			.byId(trigger.id)
			.source.set({ Parameter: { fixture_id: fixtureId, parameter: chosen.kind } });
	}

	const firedAt = (trigger: Trigger) =>
		trigger.last_fired_at ? new Date(trigger.last_fired_at).toLocaleTimeString() : '–';

	onMount(() => {
		const stops = [
			data.triggers.subscribeDeep((v) => { triggers = v; }),
			data.fixtures.subscribeDeep((v) => { fixtures = v; }),
			data.fixture_types.subscribeDeep((v) => { types = v; }),
			data.sequences.subscribeDeep((v) => { sequences = v; })
		];
		return () => stops.forEach((stop) => stop());
	});
</script>

<div class="triggers">
	<section class="block">
		<header class="block-head">
			<h2>Triggers</h2>
			<button
				class="ghost"
				disabled={fixtures.length === 0 || sequences.length === 0}
				onclick={() => (creating = !creating)}
			>
				{creating ? 'Cancel' : '+ Trigger'}
			</button>
		</header>

		{#if creating}
			<form class="new-row" onsubmit={(e) => { e.preventDefault(); createTrigger(); }}>
				<input class="text-input" placeholder="What does it do?" bind:value={newName} use:focusOnMount />
				<button class="primary" type="submit">Add</button>
			</form>
		{/if}

		{#if fixtures.length === 0 || sequences.length === 0}
			<p class="empty">
				A trigger watches a fixture parameter and drives a sequence, so it needs one of each
				first.
			</p>
		{:else if triggers.length === 0}
			<p class="empty">Nothing wired up yet.</p>
		{:else}
			<table class="rules">
				<thead>
					<tr>
						<th>Name</th><th>Watches</th><th>Parameter</th><th>When</th><th>Then</th>
						<th>Delay</th><th>On</th><th>Last</th><th></th>
					</tr>
				</thead>
				<tbody>
					{#each triggers as trigger (trigger.id)}
						{@const boolean = watchesABoolean(trigger)}
						<tr class:pending={trigger.pending} class:off={!trigger.enabled}>
							<td>
								<input
									class="text-input"
									value={trigger.name}
									onchange={(e) => data.triggers.byId(trigger.id).name.set(e.currentTarget.value)}
								/>
							</td>
							<td>
								<select
									class="text-input"
									value={watchedFixture(trigger)}
									onchange={(e) => watch(trigger, e.currentTarget.value)}
								>
									{#each fixtures as fixture (fixture.id)}
										<option value={fixture.id}>{fixture.name}</option>
									{/each}
								</select>
							</td>
							<td>
								<select
									class="text-input"
									value={watchedParameter(trigger)}
									onchange={(e) =>
										watch(trigger, watchedFixture(trigger), e.currentTarget.value)}
								>
									{#each parametersOf(watchedFixture(trigger)) as param (parameterKindLabel(param.kind))}
										<option value={parameterKindLabel(param.kind)}>
											{parameterKindLabel(param.kind)}{param.direction === 'Input' ? '' : ' (driven)'}
										</option>
									{/each}
								</select>
							</td>
							<td class="when">
								<select
									class="text-input"
									value={typeof trigger.condition === 'string'
										? trigger.condition
										: Object.keys(trigger.condition)[0]}
									onchange={(e) =>
										data.triggers
											.byId(trigger.id)
											.condition.set(
												conditionFromLabel(
													e.currentTarget.value,
													thresholdOf(trigger.condition)
												)
											)}
								>
									{#if boolean}
										<option value="RisingEdge">Closes</option>
										<option value="FallingEdge">Opens</option>
										<option value="AnyChange">Changes</option>
									{:else}
										<option value="Above">Rises above</option>
										<option value="Below">Falls below</option>
										<option value="AnyChange">Changes</option>
									{/if}
								</select>
								{#if typeof trigger.condition !== 'string'}
									<input
										class="text-input narrow"
										type="number"
										step="0.1"
										value={thresholdOf(trigger.condition)}
										onchange={(e) =>
											data.triggers
												.byId(trigger.id)
												.condition.set(
													conditionFromLabel(
														Object.keys(trigger.condition)[0],
														Number(e.currentTarget.value)
													)
												)}
									/>
								{/if}
							</td>
							<td>
								{#if actionKind(trigger) === 'SetParameter'}
									<span class="hint">Set a parameter</span>
								{:else}
									<select
										class="text-input"
										value={actionSequence(trigger)}
										onchange={(e) =>
											data.triggers
												.byId(trigger.id)
												.action.set({ GoNext: { sequence_id: e.currentTarget.value } })}
									>
										{#each sequences as sequence (sequence.id)}
											<option value={sequence.id}>Go: {sequence.name}</option>
										{/each}
									</select>
								{/if}
							</td>
							<td>
								<input
									class="text-input narrow"
									type="number"
									min="0"
									step="100"
									value={trigger.delay_ms}
									onchange={(e) =>
										data.triggers.byId(trigger.id).delay_ms.set(Number(e.currentTarget.value))}
								/>
							</td>
							<td>
								<input
									type="checkbox"
									checked={trigger.enabled}
									onchange={(e) =>
										data.triggers.byId(trigger.id).enabled.set(e.currentTarget.checked)}
								/>
							</td>
							<td class="last">
								{#if trigger.pending}
									<span class="badge">waiting</span>
								{:else}
									<span class="stamp">{firedAt(trigger)}</span>
								{/if}
							</td>
							<td>
								<button
									class="danger"
									title="Delete trigger"
									onclick={() => data.triggers.byId(trigger.id).delete()}>×</button
								>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
			<p class="note">
				Only the node leading the session fires a trigger. A trigger that sets a parameter a cue
				is also fading will lose to it on the next tick.
			</p>
		{/if}
	</section>
</div>

<style>
	.triggers { padding: 16px 20px; }
	.block { margin-bottom: 24px; }
	.block-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
	h2 { font-size: 13px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; color: #999; }
	.rules { width: 100%; border-collapse: collapse; font-size: 13px; }
	.rules th { text-align: left; color: #777; font-weight: 500; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; padding: 0 6px 6px 0; }
	.rules td { padding: 3px 6px 3px 0; vertical-align: middle; }
	.rules tr.off td { opacity: 0.5; }
	.rules tr.pending td { background: #1f2a3a; }
	.when { display: flex; gap: 4px; align-items: center; }
	.last { font-variant-numeric: tabular-nums; }
	.stamp { color: #777; font-size: 12px; }
	.badge { background: #1e3a5f44; color: #60a5fa; border: 1px solid #1e3a5f; border-radius: 10px; padding: 1px 7px; font-size: 11px; }
	.hint { color: #777; font-size: 12px; }
	.new-row { display: flex; gap: 6px; margin-bottom: 8px; }
	.empty { color: #777; font-size: 13px; padding: 8px 0; }
	.note { color: #666; font-size: 12px; margin-top: 10px; font-style: italic; }
	.text-input { background: #171717; border: 1px solid #3a3a3a; border-radius: 3px; color: #e0e0e0; padding: 4px 6px; font: inherit; }
	.text-input.narrow { width: 76px; }
	.primary { background: #2f6fd0; border: none; border-radius: 3px; color: #fff; padding: 5px 12px; font: inherit; cursor: pointer; }
	.ghost { background: none; border: 1px solid #3a3a3a; border-radius: 3px; color: #bbb; padding: 4px 10px; font: inherit; cursor: pointer; }
	.ghost:hover:not(:disabled) { border-color: #555; color: #fff; }
	.ghost:disabled { opacity: 0.4; cursor: not-allowed; }
	.danger { background: none; border: none; color: #777; font-size: 16px; line-height: 1; padding: 4px 8px; cursor: pointer; }
	.danger:hover { color: #e05555; }
</style>
