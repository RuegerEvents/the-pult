/**
 * A node's pixel strips, as the Devices panel lists them.
 *
 * Pure, and over what the station already said: the head fixtures carry how each strip
 * is patched, and the node says which universes it reads for each. Where a pixel lands
 * is the station's arithmetic and is never worked out again here — the universes
 * printed are the node's own.
 *
 * A strip is **one fixture** with a cell per pixel (see task 82), so there is nothing
 * else to gather: selecting the strip selects its pixels.
 */
import type { DiscoveredDevice, Fixture, NodeStrip, PixelStrip } from './generated/index.js';

export interface StripRow {
	head: number;
	fixture: Fixture;
	strip: PixelStrip;
	mode: PixelStrip['mode'];
	/** What the node says it is running, where it has said. */
	running: NodeStrip | null;
}

function addressOf(fixture: Fixture) {
	return 'OpenHaunt' in fixture.address ? fixture.address.OpenHaunt : null;
}

export function stripRows(device: DiscoveredDevice, fixtures: Fixture[]): StripRow[] {
	const rows: StripRow[] = [];
	for (const fixture of fixtures) {
		const address = addressOf(fixture);
		if (!address || address.serial !== device.serial || !address.strip) continue;
		const head = address.head ?? 0;
		rows.push({
			head,
			fixture,
			strip: address.strip,
			mode: address.strip.mode,
			running: device.pixels?.strips.find((s) => s.head === head) ?? null
		});
	}
	return rows.sort((a, b) => a.head - b.head);
}

/** "300 GRB pixels in universes 5–6", or "60 GRB pixels, drawn by the node". */
export function stripSummary(strip: PixelStrip, running: NodeStrip | null): string {
	const pixels = `${strip.count} ${strip.order} pixel${strip.count === 1 ? '' : 's'}`;
	if (strip.mode !== 'sacn') return `${pixels}, drawn by the node`;
	const universes = [...new Set(running?.mode === 'sacn' ? running.universes : [])].sort((a, b) => a - b);
	if (universes.length === 0) return `${pixels} over sACN`;
	const span =
		universes.length === 1
			? `universe ${universes[0]}`
			: `universes ${universes[0]}–${universes[universes.length - 1]}`;
	return `${pixels} in ${span}`;
}
