/**
 * What is on the wire, as a browser keeps it.
 *
 * A station draws a view of one connector only while somebody is watching, sends it
 * at panel rate rather than wire rate, and **does not send it again when it has not
 * changed**. Two things follow for the reader, and both live here rather than in the
 * panel.
 *
 * **The last view stands.** A settled rig pushes nothing, so what is on screen is
 * the last thing that was true and not a gap. The stamp on it says when.
 *
 * **Messages accumulate here.** A connector's ring is bounded by what it can afford
 * to keep between two looks; the history is bounded by what a person can read. So a
 * `messages` section arrives *drained* — what was said since the last look — and this
 * is what turns a sequence of batches back into a log. It is written into the section
 * in place, so a view component only ever sees one shape: whatever the connector said
 * this part of its traffic is.
 */

import type { OutputMessage, OutputView } from '$lib/generated/index.js';

/** One output, anywhere in the session. */
export type WireKey = string;

export const wireKey = (nodeId: string, outputId: string): WireKey => `${nodeId}:${outputId}`;

/**
 * How many messages a reader keeps per section.
 *
 * Longer than the station's ring on purpose: the ring exists so that nothing is
 * missed between two looks, and this exists so that something is still there to read
 * when somebody looks up.
 */
export const HISTORY = 500;

export class Wire {
	private views = new Map<WireKey, OutputView>();
	/** Per `key` and section, everything said since this browser started watching. */
	private said = new Map<string, OutputMessage[]>();
	/** And how much of it the station threw away before it reached here. */
	private lost = new Map<string, number>();

	/** Take one push. */
	take(view: OutputView): void {
		const key = wireKey(view.node_id, view.output_id);
		view.sections.forEach((section, index) => {
			if (section.body.shape !== 'messages') return;
			const at = `${key}#${index}`;
			const kept = [...(this.said.get(at) ?? []), ...section.body.of.messages];
			const trimmed = kept.slice(-HISTORY);
			this.said.set(at, trimmed);
			// Two ways to lose a message and both are counted: the station's ring
			// overflowed between two looks, or this history did. Neither is silent —
			// a hole a reader cannot see is worse than one they can.
			const lost = (this.lost.get(at) ?? 0) + section.body.of.dropped + (kept.length - trimmed.length);
			this.lost.set(at, lost);
			section.body.of = { messages: trimmed, dropped: lost };
		});
		this.views.set(key, view);
	}

	/** What this output was last seen doing, if it has been seen at all. */
	view(key: WireKey): OutputView | undefined {
		return this.views.get(key);
	}

	/** Let go of an output nobody is watching any more, history and all. */
	forget(key: WireKey): void {
		this.views.delete(key);
		for (const at of [...this.said.keys()]) {
			if (at.startsWith(`${key}#`)) {
				this.said.delete(at);
				this.lost.delete(at);
			}
		}
	}
}

/**
 * The universe a viewer is looking at, as the station spells it.
 *
 * A focus is an opaque string on the wire — the connector names the parts of its own
 * traffic, which is what lets a protocol nobody has written yet be watched at all —
 * so the one place that knows a universe is written as its number is here, beside the
 * sheet that asks for one.
 */
export const universeFocus = (universe: number): string => String(universe);

/**
 * How bright to draw a channel at `value`.
 *
 * A DMX sheet is 512 numbers and reading it as numbers is slow; what an operator is
 * actually looking for is *where the rig is doing something*. So zero is muted, and
 * everything else lifts with the value — a shape to scan rather than a table to read.
 */
export const channelWeight = (value: number): number =>
	value === 0 ? 0 : 0.35 + 0.65 * (value / 255);
