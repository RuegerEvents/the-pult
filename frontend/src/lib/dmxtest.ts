/**
 * Forcing DMX channels by hand: the state and the arithmetic, apart from the sheet.
 *
 * A `ChannelTest` is what this browser wants held on an output's wire — channel 5 of
 * universe 3 at 255 — and the station applies it over the render for as long as this
 * page is here. Nothing in it knows what is patched, which is what makes it usable
 * before anything is.
 *
 * Every function returns a new test and none mutates one: the panel hands the result
 * to the station whole, because the station replaces what it was told before and an
 * empty test is the withdrawal. A diff would leave a light on if one message were lost.
 */

import type { ChannelTest, TestUniverse } from '$lib/generated/index.js';

export const CHANNELS = 512;

/** The most universes a station accepts in one test; the same figure it checks. */
export const MAX_UNIVERSES = 16;

export const emptyTest = (): ChannelTest => ({ universes: [] });

export const isEmpty = (test: ChannelTest): boolean => test.universes.length === 0;

const universeOf = (test: ChannelTest, universe: number): TestUniverse | undefined =>
	test.universes.find((each) => each.universe === universe);

/** Every channel forced in `universe`, as a map of channel to level. */
export const forcedIn = (test: ChannelTest, universe: number): Record<number, number> => {
	const levels: Record<number, number> = {};
	for (const [channel, level] of Object.entries(universeOf(test, universe)?.levels ?? {})) {
		if (level !== undefined) levels[Number(channel)] = level;
	}
	return levels;
};

export const isSolo = (test: ChannelTest, universe: number): boolean =>
	universeOf(test, universe)?.solo ?? false;

/** How many channels are forced across the whole test, for a banner. */
export const forcedCount = (test: ChannelTest): number =>
	test.universes.reduce((n, each) => n + Object.keys(each.levels).length, 0);

/**
 * Drop a universe that forces nothing and is not solo, and keep the rest ascending —
 * the form a station wants. A solo universe with no channels is kept: it is how a
 * line is blacked out entirely.
 */
const tidy = (universes: TestUniverse[]): ChannelTest => ({
	universes: universes
		.filter((each) => each.solo || Object.keys(each.levels).length > 0)
		.sort((a, b) => a.universe - b.universe)
});

const edit = (
	test: ChannelTest,
	universe: number,
	change: (current: TestUniverse) => TestUniverse
): ChannelTest => {
	const others = test.universes.filter((each) => each.universe !== universe);
	const current = universeOf(test, universe) ?? { universe, solo: false, levels: {} };
	return tidy([...others, change({ ...current, levels: { ...current.levels } })]);
};

const clampLevel = (level: number) => Math.max(0, Math.min(255, Math.round(level)));

/** Force `channels` of `universe` to `level`. */
export const force = (
	test: ChannelTest,
	universe: number,
	channels: readonly number[],
	level: number
): ChannelTest =>
	edit(test, universe, (current) => {
		for (const channel of channels) {
			if (channel >= 1 && channel <= CHANNELS) current.levels[channel] = clampLevel(level);
		}
		return current;
	});

/** Let go of `channels`, which return to whatever the show says. */
export const release = (
	test: ChannelTest,
	universe: number,
	channels: readonly number[]
): ChannelTest =>
	edit(test, universe, (current) => {
		for (const channel of channels) delete current.levels[channel];
		return current;
	});

/** Let go of everything in one universe, solo included. */
export const releaseUniverse = (test: ChannelTest, universe: number): ChannelTest =>
	tidy(test.universes.filter((each) => each.universe !== universe));

export const setSolo = (test: ChannelTest, universe: number, solo: boolean): ChannelTest =>
	edit(test, universe, (current) => ({ ...current, solo }));

/** Channels `from` to `to` inclusive, whichever way round they were given. */
export const range = (from: number, to: number): number[] => {
	const low = Math.max(1, Math.min(from, to));
	const high = Math.min(CHANNELS, Math.max(from, to));
	return Array.from({ length: high - low + 1 }, (_, i) => low + i);
};

/**
 * "1, 5–8, 12": a selection as an operator would say it. Contiguous runs of three or
 * more collapse; two stay separate, which reads better than "5–6".
 */
export const describe = (channels: readonly number[]): string => {
	const sorted = [...new Set(channels)].sort((a, b) => a - b);
	const parts: string[] = [];
	for (let at = 0; at < sorted.length; ) {
		let end = at;
		while (end + 1 < sorted.length && sorted[end + 1] === sorted[end] + 1) end++;
		if (end - at >= 2) parts.push(`${sorted[at]}–${sorted[end]}`);
		else for (let i = at; i <= end; i++) parts.push(String(sorted[i]));
		at = end + 1;
	}
	return parts.join(', ');
};

/**
 * Move a selection along by `by` channels, or `null` where any of it would leave the
 * universe. Refused rather than clamped: squashing a run against the edge changes its
 * shape, and a walk that quietly tests a different set of channels than it says is the
 * wrong kind of helpful.
 */
export const shifted = (channels: readonly number[], by: number): number[] | null => {
	const moved = channels.map((channel) => channel + by);
	return moved.every((channel) => channel >= 1 && channel <= CHANNELS) ? moved : null;
};

/** "100%" for a level, as a lighting operator reads one. */
export const percent = (level: number): string => `${Math.round((level / 255) * 100)}%`;
