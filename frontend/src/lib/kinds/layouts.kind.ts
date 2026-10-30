/**
 * The Layouts panel's kinds: a layout — a drawing of somewhere with what is happening
 * there painted on it — and each element on one.
 *
 * A layout's tap opens it full-screen, the way the phone will draw it, as a surface's
 * does. An element is a kind for the reason a timeline's clip is: it is something a
 * person picks up on a canvas, so its editor belongs in the Inspector rather than in a
 * form under the drawing. Its rows are read off the `layouts` collection, so it is
 * still a field of its layout in the schema — registering it changes how it is
 * reached, not where it is kept. A tap on one presses it, where it presses anything.
 */

import { derived, get, type Readable } from 'svelte/store';

import MapPinned from '@lucide/svelte/icons/map-pinned';
import SquareDashed from '@lucide/svelte/icons/square-dashed';

import type { Layout, LayoutElement } from '$lib/generated/index.js';
import { duplicated, elementName, layoutUrl } from '$lib/layoutView.js';
import { copyName, registerKind, selectSubject, type ObjectRow } from '$lib/objects.js';
import { editElements, layouts, pressElement } from '$lib/stores/layouts.js';
import { showData } from '$lib/stores/show.js';
import { surfaceShow } from '$lib/stores/surface.js';
import { copyText, uuid } from '$lib/uuid.js';
import LayoutInspector from '$lib/components/layouts/LayoutInspector.svelte';
import ElementInspector from '$lib/components/layouts/ElementInspector.svelte';

registerKind<Layout>({
	kind: 'layout',
	table: 'layouts',
	label: 'Layout',
	category: 'stage',
	icon: MapPinned,
	rows: layouts,
	panel: 'layout',
	actLabel: 'Open full-screen',
	act: (layout) => {
		window.open(layoutUrl(window.location, layout.id), '_blank');
	},
	rename: (layout, name) => showData().layouts.byId(layout.id).name.set(name),
	recolor: (layout, color) => showData().layouts.byId(layout.id).color.set(color),
	duplicate: async (layout) => {
		await showData().layouts.create({
			...layout,
			id: uuid(),
			name: copyName(layout.name, get(layouts).map((l) => l.name)),
			// Fresh element ids: two layouts sharing one would select each other's.
			elements: layout.elements.map((e) => ({ ...e, id: uuid() })),
			backdrop: layout.backdrop ?? null,
			folder: layout.folder ?? null
		});
	},
	remove: (layout) => showData().layouts.byId(layout.id).delete(),
	inspector: LayoutInspector,
	verbs: (layout) => [
		{
			id: 'copy-link',
			label: 'Copy the phone link',
			structural: false,
			run: () => copyText(layoutUrl(window.location, layout.id))
		}
	]
});

export type ElementRow = ObjectRow & { layoutId: string; element: LayoutElement };

const elementRows: Readable<ElementRow[]> = derived([layouts, surfaceShow], ([$layouts, $show]) =>
	$layouts.flatMap((layout) =>
		layout.elements.map((element) => ({
			id: element.id,
			name: elementName(element, $show),
			layoutId: layout.id,
			element
		}))
	)
);

registerKind<ElementRow>({
	kind: 'layout-element',
	label: 'Layout element',
	category: 'stage',
	icon: SquareDashed,
	rows: elementRows,
	panel: 'layout',
	actLabel: 'Press',
	act: async (row) => {
		// An element with nothing to press is opened instead, which is the only useful
		// thing a tap on a wall can do.
		if (!(await pressElement(row.element))) selectSubject({ kind: 'layout-element', id: row.id });
	},
	rename: (row, label) =>
		editElements(row.layoutId, (elements) => elements.map((e) => (e.id === row.id ? { ...e, label } : e))),
	duplicate: async (row) => {
		const layout = get(layouts).find((l) => l.id === row.layoutId);
		if (!layout) return;
		const copy = duplicated(row.element, layout, layout.grid);
		await editElements(row.layoutId, (elements) => [...elements, copy]);
		selectSubject({ kind: 'layout-element', id: copy.id });
	},
	remove: (row) => editElements(row.layoutId, (elements) => elements.filter((e) => e.id !== row.id)),
	inspector: ElementInspector
});
