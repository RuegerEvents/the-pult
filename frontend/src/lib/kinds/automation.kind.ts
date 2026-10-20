/**
 * Flows and variables, as objects: a flow to find and open, a variable to flip or
 * reset from anywhere — through the same unattributed `variable.*` calls the
 * Variables panel and a phone use, so a palette press writes no history either.
 */

import Variable from '@lucide/svelte/icons/variable';
import Workflow from '@lucide/svelte/icons/workflow';

import type { Flow, Variable as VariableRow } from '$lib/generated/index.js';
import { registerKind } from '$lib/objects.js';
import { revealPanel } from '$lib/stores/layout.js';
import { collection, showClient, showData } from '$lib/stores/show.js';

registerKind<Flow>({
	kind: 'flow',
	label: 'Flow',
	category: 'automation',
	icon: Workflow,
	rows: collection('flows'),
	panel: 'flows',
	actLabel: 'Open',
	act: () => revealPanel('flows'),
	rename: (flow, name) => showData().flows.byId(flow.id).name.set(name),
	verbs: (flow) => [
		{
			id: 'enable',
			label: flow.enabled ? 'Disable' : 'Enable',
			structural: true,
			run: () => showData().flows.byId(flow.id).enabled.set(!flow.enabled)
		}
	]
});

registerKind<VariableRow>({
	kind: 'variable',
	label: 'Variable',
	category: 'automation',
	icon: Variable,
	rows: collection('variables'),
	panel: 'variables',
	actLabel: 'Toggle',
	act: (variable) =>
		variable.kind === 'Flag'
			? showClient().call('variable.toggle', { variableId: variable.id })
			: revealPanel('variables'),
	rename: (variable, name) => showData().variables.byId(variable.id).name.set(name),
	verbs: (variable) => [
		{
			id: 'reset',
			label: 'Reset',
			structural: false,
			run: () => showClient().call('variable.reset', { variableId: variable.id })
		}
	]
});
