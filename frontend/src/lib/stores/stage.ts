/**
 * Which stage plan this browser is looking at.
 *
 * A show has as many plans as it has rooms — a main stage and a foyer, a ground plan
 * and a truss plot — and two panels draw the same one: the plan itself, and the floor
 * under the 3D rig. They have to agree, or a show with two rooms in it draws the
 * first one's floor beneath the second one's lights.
 *
 * Not show data, for the reason the layout is not: two operators at two screens
 * plainly want different rooms up. `null` means "whichever is first", which is what
 * the panels did before there was a picker at all.
 */

import { writable } from 'svelte/store';

export const shownPlanId = writable<string | null>(null);
