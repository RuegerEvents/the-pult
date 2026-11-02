/**
 * Focus an element when it appears.
 *
 * The `autofocus` attribute only fires for elements present when the document
 * loads. Every input in this app appears in response to a click, so autofocus did
 * nothing on all of them: focus stayed on the button that opened the form and the
 * operator's first keystrokes went nowhere.
 *
 * Usage: `<input use:focusOnMount />`
 */
export function focusOnMount(node: HTMLElement) {
	// After paint, so the element is laid out and a transition cannot steal focus back.
	requestAnimationFrame(() => node.focus());
}

/** Focus and select the current text, for a field opened to overwrite a value. */
export function selectOnMount(node: HTMLInputElement) {
	requestAnimationFrame(() => {
		node.focus();
		node.select();
	});
}

/**
 * A top-bar dropdown that puts itself away: the mouse leaving it, a press anywhere
 * outside it and its button, or Escape.
 *
 * **The leaving is the mouse's only.** The menus closed on `pointerleave`, and a
 * finger lifting off a screen *is* a pointer leaving — it fires after `pointerup` and
 * before `click`, so a tap on an item unmounted the menu before the item heard the
 * click, and on a tablet no menu item did anything (found at the gig: the Layouts
 * menu). A touch has no hover to leave, so for a touch the press outside is what puts
 * the menu away instead.
 *
 * Usage, on the menu itself: `<div class="menu" use:dropdown={{ close: () => (open = false) }}>`.
 * "Outside" is outside the menu's parent, which is where its button lives, so the press
 * that toggles it shut is not also an outside press. `keep` holds it open while it is
 * busy, the way the user menu stays up while a name is being typed into it.
 */
export function dropdown(node: HTMLElement, options: { close: () => void; keep?: () => boolean }) {
	let { close, keep } = options;
	const root = node.parentElement ?? node;
	const shut = () => {
		if (!keep?.()) close();
	};
	const left = (e: PointerEvent) => {
		if (e.pointerType === 'mouse') shut();
	};
	const pressed = (e: PointerEvent) => {
		if (e.target instanceof Node && root.contains(e.target)) return;
		shut();
	};
	const key = (e: KeyboardEvent) => {
		if (e.key === 'Escape') shut();
	};
	node.addEventListener('pointerleave', left);
	// Capture, so a panel that stops its own pointerdown still counts as outside.
	document.addEventListener('pointerdown', pressed, true);
	document.addEventListener('keydown', key);
	return {
		update(next: { close: () => void; keep?: () => boolean }) {
			({ close, keep } = next);
		},
		destroy() {
			node.removeEventListener('pointerleave', left);
			document.removeEventListener('pointerdown', pressed, true);
			document.removeEventListener('keydown', key);
		}
	};
}

/**
 * Long-press: how a tablet reaches a right-click menu.
 *
 * Half a second held without moving more than a few pixels. A press that turns into a
 * drag or a scroll is not a long-press, and the click a long-press would otherwise end
 * in is swallowed, so holding a preset to reach its menu does not also apply it.
 *
 * Usage: `<button use:longPress={(x, y) => openMenu(ref, x, y)}>`
 */
export function longPress(node: HTMLElement, onpress: (x: number, y: number) => void) {
	let handler = onpress;
	let timer: ReturnType<typeof setTimeout> | null = null;
	let from = { x: 0, y: 0 };
	let fired = false;

	const cancel = () => {
		if (timer) clearTimeout(timer);
		timer = null;
	};
	const down = (e: PointerEvent) => {
		if (e.pointerType === 'mouse') return; // a mouse has a right button
		fired = false;
		from = { x: e.clientX, y: e.clientY };
		cancel();
		timer = setTimeout(() => {
			timer = null;
			fired = true;
			handler(from.x, from.y);
		}, 500);
	};
	const move = (e: PointerEvent) => {
		if (timer && Math.hypot(e.clientX - from.x, e.clientY - from.y) > 8) cancel();
	};
	const click = (e: MouseEvent) => {
		if (!fired) return;
		fired = false;
		e.stopPropagation();
		e.preventDefault();
	};
	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', cancel);
	node.addEventListener('pointercancel', cancel);
	node.addEventListener('click', click, true);
	return {
		update(next: (x: number, y: number) => void) {
			handler = next;
		},
		destroy() {
			cancel();
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', cancel);
			node.removeEventListener('pointercancel', cancel);
			node.removeEventListener('click', click, true);
		}
	};
}
