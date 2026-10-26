/**
 * The effect pool's kind: a template. A tap applies it to the selection, as one
 * gesture over every layer; "Open in Effects" loads it into the editor to change.
 */

import { get, writable } from 'svelte/store';

import Sparkles from '@lucide/svelte/icons/sparkles';

import type { EffectTemplate, Fixture, FixtureType } from '$lib/generated/index.js';
import { splitTarget, targetIds, templateApply } from '$lib/effectPool.js';
import { copyName, registerKind } from '$lib/objects.js';
import {
	applyLayers,
	deleteTemplate,
	renameTemplate,
	templates
} from '$lib/stores/effectPool.js';
import { revealPanel } from '$lib/stores/layout.js';
import { elements } from '$lib/stores/selection.js';
import { collection, showData } from '$lib/stores/show.js';
import { addToast } from '$lib/toasts.js';
import TemplateInspector from '$lib/components/effects/TemplateInspector.svelte';
import { uuid } from '$lib/uuid.js';

const fixtures = collection('fixtures');
const types = collection('fixture_types');

/** A template somebody asked to open in the Effects panel. The panel takes it. */
export const openTemplate = writable<EffectTemplate | null>(null);

/** Apply a template to the selection now. Answers how many fixtures it reached. */
export async function applyToSelection(template: EffectTemplate): Promise<number> {
	const byId = new Map<string, Fixture>(get(fixtures).map((f) => [f.id, f]));
	const typeById = new Map<string, FixtureType>(get(types).map((t) => [t.id, t]));
	const order = targetIds(get(elements).filter((e) => byId.has(e.fixture)));
	const applies = templateApply(template, order, byId, typeById);
	await applyLayers(applies);
	return new Set(applies.flatMap((a) => a.order).map((t) => splitTarget(t).fixture)).size;
}

registerKind<EffectTemplate>({
	kind: 'effect',
	table: 'effect_templates',
	label: 'Effect',
	category: 'program',
	icon: Sparkles,
	rows: templates,
	panel: 'effects',
	actLabel: 'Apply to the selection',
	act: async (template) => {
		const reached = await applyToSelection(template);
		if (reached === 0) addToast(`Nothing selected can take “${template.name}”`);
	},
	rename: (template, name) => renameTemplate(template.id, name),
	recolor: (template, color) => showData().effect_templates.byId(template.id).color.set(color),
	duplicate: async (template) => {
		await showData().effect_templates.create({
			...template,
			id: uuid(),
			name: copyName(template.name, get(templates).map((t) => t.name))
		});
	},
	remove: (template) => deleteTemplate(template.id),
	verbs: (template) => [
		{
			id: 'open',
			label: 'Open in Effects',
			structural: false,
			run: () => {
				openTemplate.set(template);
				revealPanel('effects');
			}
		}
	],
	inspector: TemplateInspector
});
