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
	Metres: ' m',
	Watts: ' W'
};

/** The readout of a 0..1 value: `short` for inside a fader, `text` for a screen reader. */
export function physicalReadout(
	normalised: number,
	range: PhysicalRange | null | undefined
): { short: string; text: string } {
	if (!range || range.unit === 'Percent' && range.from === 0 && range.to === 100) {
		const percent = `${Math.round(normalised * 100)}`;
		return { short: percent, text: `${percent}%` };
	}
	const value = range.from + (range.to - range.from) * normalised;
	const span = Math.abs(range.to - range.from);
	// As many places as the span needs to move in hundredths of its travel.
	const places = span >= 100 ? 0 : span >= 10 ? 1 : 2;
	const figure = value.toFixed(places);
	const unit = SUFFIX[range.unit] ?? '';
	return { short: `${figure}${unit}`, text: `${figure}${unit}` };
}
