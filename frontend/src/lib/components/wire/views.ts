/**
 * What can draw one part of a connector's traffic.
 *
 * This is the seam. A station describes what it is putting on the wire in **shapes**
 * rather than in protocols — whole universes, discrete messages — and this table is
 * the one place that turns a shape into a component. Which means a new output is
 * cheap in exactly the way it should be:
 *
 * - An output whose traffic looks like something already here — anything carrying
 *   universes, say — gets a viewer for **nothing**. It answers `observe` with a
 *   `Universes` section and the sheet draws it.
 * - An output whose traffic looks like nothing here adds a `SectionBody` variant in
 *   `pult-schema`, a component beside these, and **one line below**. No panel
 *   changes, no protocol changes, and nothing enumerates outputs anywhere.
 * - A shape this build has never heard of — an older console reading a newer
 *   station, or a connector that arrived as a plugin — draws as itself rather than
 *   vanishing. Same rule the layout tree follows for a panel id it does not know: a
 *   thing you can see and not read beats a blank where a thing should be.
 */

import type { Component } from 'svelte';

import type { ChannelTest, SectionBody } from '$lib/generated/index.js';

import MessageLog from './MessageLog.svelte';
import UniverseSheet from './UniverseSheet.svelte';
import UnknownShape from './UnknownShape.svelte';

/**
 * The hand on the wire: what this browser is forcing on the output being looked at,
 * and how to change it. Handed to a view only where forcing means something — an
 * output rather than an input, and a shape made of channels — so a view that has no
 * use for it never learns it exists.
 */
export type Tester = {
	test: ChannelTest;
	/** Replace the whole test. The station keeps one value per browser, not a history. */
	set: (next: ChannelTest) => void;
};

/** What every view is handed, whatever it draws. */
export type SectionProps = {
	/** The shape's own name, for the fallback that has only that to go on. */
	shape: string;
	/** The body, as the connector described it. */
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	of: any;
	/** What this viewer asked to look at, in the connector's own terms. */
	focus: string | null;
	/** Ask for a different part of this connector's traffic. */
	ask: (focus: string | null) => void;
	/** Present only on an output's own wire, and only to a view that can use it. */
	tester?: Tester;
};

const SECTION_VIEWS: Record<SectionBody['shape'], Component<SectionProps>> = {
	universes: UniverseSheet as Component<SectionProps>,
	messages: MessageLog as Component<SectionProps>
};

export const viewFor = (shape: string): Component<SectionProps> =>
	SECTION_VIEWS[shape as SectionBody['shape']] ?? (UnknownShape as Component<SectionProps>);
