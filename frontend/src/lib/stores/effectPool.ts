/**
 * The effect pool's verbs: apply a template to the selection, keep what is being
 * built, and the rest.
 *
 * Each is **one gesture**, so applying "fire" to forty heads across two parameters is
 * one Ctrl-Z and storing it back is one more.
 */

import type { EffectLayer, EffectTemplate } from '$lib/generated/index.js';
import type { LayerApply } from '$lib/effectPool.js';
import { beginGesture, endGesture } from './gesture.js';
import { setEffect } from './programmer.js';
import { collection, showData } from './show.js';

/** The pool, in the show's order. */
export const templates = collection('effect_templates');

/** Write every layer of an apply into the programmer, as one act. */
export async function applyLayers(applies: LayerApply[]): Promise<number> {
	if (applies.length === 0) return 0;
	let written = 0;
	beginGesture();
	try {
		for (const apply of applies) {
			await setEffect(apply.kind, apply.specs);
			written += apply.order.length;
		}
	} finally {
		endGesture();
	}
	return written;
}

/** Keep what is being built as a new pool entry. */
export async function storeTemplate(name: string, layers: EffectLayer[]): Promise<EffectTemplate> {
	const template: EffectTemplate = { id: crypto.randomUUID(), name, layers };
	await showData().effect_templates.create(template);
	return template;
}

/** Replace a pool entry's layers with what is being built. Nothing already running moves. */
export async function updateTemplate(id: string, layers: EffectLayer[]): Promise<void> {
	await showData().effect_templates.byId(id).layers.set(layers);
}

export async function renameTemplate(id: string, name: string): Promise<void> {
	await showData().effect_templates.byId(id).name.set(name);
}

export async function deleteTemplate(id: string): Promise<void> {
	await showData().effect_templates.byId(id).delete();
}
