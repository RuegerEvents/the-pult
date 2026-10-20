/**
 * A number as an operator reads it: in the unit its parameter declared.
 *
 * A strip's Speed spans 0 to 10 Hz, so full is "10 Hz" and not "100". A parameter
 * that declared no range is honestly a fraction of its travel and reads as a percent,
 * which is what every fader here said before ranges existed.
 */

import type { PhysicalRange, PhysicalUnit } from './generated/index.js';

const SUFFIX: Record<PhysicalUnit, string> = {
	None: '',
	Percent: '%',
	Degrees: '°',
	Seconds: ' s',
	Hertz: ' Hz',
	Kelvin: ' K',
	Meters: ' m',
	Watts: ' W'
};

/** The readout of a 0..1 value: `short` for inside a fader, `text` for a screen reader. */
export function physicalReadout(
	normalized: number,
	range: PhysicalRange | null | undefined
): { short: string; text: string } {
	// A GDTF says a dimmer's Percent as 0 to 1 — every file in the corpus does — so a
	// percent range inside ±1 is a fraction and reads ×100, or a master at half reads
	// "0.50%". Either spelling of 0..100% reads as the plain percent every fader shows.
	const fraction = range?.unit === 'Percent' && Math.max(Math.abs(range.from), Math.abs(range.to)) <= 1;
	const scaled = range && fraction ? { ...range, from: range.from * 100, to: range.to * 100 } : range;
	if (!scaled || (scaled.unit === 'Percent' && scaled.from === 0 && scaled.to === 100)) {
		const percent = `${Math.round(normalized * 100)}`;
		return { short: percent, text: `${percent}%` };
	}
	range = scaled;
	const value = range.from + (range.to - range.from) * normalized;
	const span = Math.abs(range.to - range.from);
	// As many places as the span needs to move in hundredths of its travel.
	const places = span >= 100 ? 0 : span >= 10 ? 1 : 2;
	const figure = value.toFixed(places);
	const unit = SUFFIX[range.unit] ?? '';
	return { short: `${figure}${unit}`, text: `${figure}${unit}` };
}
