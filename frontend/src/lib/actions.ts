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
