/**
 * What the operator just touched, offered to whatever is learning.
 *
 * Binding a control on a desk is: press Learn, touch the control, then touch what it
 * should do. This is the third step. The places an operator moves something announce
 * it here — a parameter in the programmer, a sequence's level, a Go, a surface tile —
 * and a control waiting to be learned takes the first offer that fits it.
 *
 * Nothing is kept: an offer is heard by whoever is listening at that moment, and a page
 * nobody is learning on throws every one away.
 *
 * The wing's own writes are not offers, and cannot be: the station runs the wing, so
 * nothing it moves passes through here. A control learning from the very fader that is
 * being learned would otherwise bind itself to whatever it happened to touch.
 */

import type { BindingAction } from '../generated/index.js';

export type Offer = { action: BindingAction };

const listeners = new Set<(offer: Offer) => void>();

/** Say that the operator just touched this. */
export function offer(action: BindingAction): void {
	if (listeners.size === 0) return;
	for (const listen of listeners) listen({ action });
}

/** Hear what the operator touches, until the returned function is called. */
export function onOffer(listen: (offer: Offer) => void): () => void {
	listeners.add(listen);
	return () => listeners.delete(listen);
}
