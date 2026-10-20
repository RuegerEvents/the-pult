/**
 * A node's pixel strips, as the Devices panel lists them.
 *
 * Pure, and over what the station already said: the head fixtures carry how each strip
 * is patched, the device row says which object a strip's pixels hang off, and the cells
 * are the fixtures hanging there. Where a pixel lands is the station's arithmetic and is
 * never worked out again here — the universes printed are read off the cells.
 */
import type { DiscoveredDevice, Fixture, NodeStrip, PixelStrip } from './generated/index.js';

export interface StripRow {
	head: number;
	fixture: Fixture;
	strip: PixelStrip;
	mode: PixelStrip['mode'];
	/** What the node says it is running, where it has said. */
	running: NodeStrip | null;
	/** The fixture per pixel, in `sacn` mode, in order along the strip. */
	cells: Fixture[];
	/** The object the cells hang off, which the strip's group asks for. */
	objectId: string | null;
	groupId: string | null;
}

function addressOf(fixture: Fixture) {
	return 'OpenHaunt' in fixture.address ? fixture.address.OpenHaunt : null;
}

export function stripRows(
	device: DiscoveredDevice,
	fixtures: Fixture[],
	groups: { id: string; query: { clauses: { term: { kind: string; object?: string } }[] } }[] = []
): StripRow[] {
	const rows: StripRow[] = [];
	for (const fixture of fixtures) {
		const address = addressOf(fixture);
		if (!address || address.serial !== device.serial || !address.strip) continue;
		const head = address.head ?? 0;
		const objectId = device.strip_objects?.[head] ?? null;
		const cells = objectId
			? fixtures
					.filter((f) => f.parent === objectId)
					.sort((a, b) => (a.unit_number ?? 0) - (b.unit_number ?? 0))
			: [];
		const group = objectId
			? groups.find((g) => g.query.clauses.some((c) => c.term.kind === 'Hanging' && c.term.object === objectId))
			: undefined;
		rows.push({
			head,
			fixture,
			strip: address.strip,
			mode: address.strip.mode,
			running: device.pixels?.strips.find((s) => s.head === head) ?? null,
			cells,
			objectId,
			groupId: group?.id ?? null
		});
	}
	return rows.sort((a, b) => a.head - b.head);
}

/** "300 GRB pixels in universes 5–6", or "60 GRB pixels, drawn by the node". */
export function stripSummary(strip: PixelStrip, cells: Fixture[]): string {
	const pixels = `${strip.count} ${strip.order} pixel${strip.count === 1 ? '' : 's'}`;
	if (strip.mode !== 'sacn') return `${pixels}, drawn by the node`;
	const universes = [
		...new Set(
			cells.flatMap((c) => ('Dmx' in c.address ? c.address.Dmx.breaks.map((b) => b.universe) : []))
		)
	].sort((a, b) => a - b);
	if (universes.length === 0) return `${pixels} over sACN`;
	const span =
		universes.length === 1
			? `universe ${universes[0]}`
			: `universes ${universes[0]}–${universes[universes.length - 1]}`;
	return `${pixels} in ${span}`;
}
