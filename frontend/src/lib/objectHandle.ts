/**
 * The bit of a card's grammar a row that is not a card still needs: right-click or a
 * long-press opens the one context menu, and it can be dragged onto something that
 * takes it — a folder. For a sequence's executor, a flow in a list, a variable: things
 * drawn their own way that are still objects in a pool.
 *
 * `<div use:objectHandle={{ kind: 'flow', id }}>`. Dragging is offered only while the
 * Show lock is off, the rule `Card` follows.
 */

import { get } from 'svelte/store';

import { longPress } from './actions.js';
import { OBJECT_MIME, openMenu, type ObjectRef } from './objects.js';
import { canEdit } from './stores/showLock.js';

export function objectHandle(node: HTMLElement, ref: ObjectRef) {
	let current = ref;
	const menu = (e: MouseEvent) => {
		// A field keeps its own menu: cut, copy and paste.
		if (field(e.target)) return;
		e.preventDefault();
		openMenu(current, e.clientX, e.clientY);
	};
	const start = (e: DragEvent) => {
		if (!get(canEdit) || !e.dataTransfer) return e.preventDefault();
		e.dataTransfer.setData(OBJECT_MIME, JSON.stringify(current));
		e.dataTransfer.effectAllowed = 'move';
	};
	let editable = get(canEdit);
	const unlock = canEdit.subscribe((on) => {
		editable = on;
		node.draggable = on;
	});
	// A press that starts in a field is somebody selecting text in it, not a drag.
	const field = (target: EventTarget | null) =>
		target instanceof HTMLElement && !!target.closest('input, textarea, select, [contenteditable]');
	const down = (e: PointerEvent) => {
		node.draggable = editable && !field(e.target);
	};
	node.addEventListener('pointerdown', down);
	node.addEventListener('contextmenu', menu);
	node.addEventListener('dragstart', start);
	const press = longPress(node, (x, y) => openMenu(current, x, y));
	return {
		update(next: ObjectRef) {
			current = next;
		},
		destroy() {
			unlock();
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('contextmenu', menu);
			node.removeEventListener('dragstart', start);
			press?.destroy?.();
		}
	};
}
