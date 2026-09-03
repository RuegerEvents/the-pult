/**
 * What a page has to work out about the show it is looking at.
 *
 * Two rules and a handful of shapes, all pure so they can be tested without a
 * console: when this tab is looking at a different console from the one it loaded
 * onto, and what a name an operator typed becomes.
 */

import type { BackendConfig } from '$lib/ws/endpoint.js';

/** What `show.list` answers about one show, opened or not. */
export type ShowSummary = {
	path: string;
	name: string;
	createdAt?: string | null;
	fixtures?: number;
	cues?: number;
	versions?: number;
	bytes?: number;
	/** The station could not read it — its stamp disagrees with this build. */
	madeByAnotherBuild?: boolean;
	/** Whatever went wrong reading it, where something did. */
	problem?: string | null;
	/** The folder is not there any more. Recent shows only. */
	missing?: boolean;
	/** When this station last opened it. Recent shows only. */
	lastOpened?: string;
};

/** One of the shows the console can make for itself. */
export type DemoCardInfo = { id: string; title: string; blurb: string };

/** What `show.list` answers. */
export type ShowList = {
	showsDir: string | null;
	open: string | null;
	demos: DemoCardInfo[];
	recent: ShowSummary[];
	inDir: ShowSummary[];
};

/**
 * Whether this page is now looking at a different console from the one it loaded
 * onto, and must therefore reload.
 *
 * Opening a show is the station stopping and another one starting in its place: a
 * different engine, a different show, and every store in this tab still holding the
 * last one's rig. Reloading is not a fallback, it is the correct thing — and it is
 * the same answer for the tablet at the back of the room on somebody else's socket,
 * which is why this compares what the station *says* rather than what this tab did.
 *
 * Only ever on a change. A page that has just loaded has nothing to compare against,
 * and reloading then would be a loop.
 */
export function shouldReload(loadedWith: BackendConfig | null, now: BackendConfig | null): boolean {
	if (!loadedWith || !now) return false;
	return showKey(loadedWith) !== showKey(now);
}

/** Which show a config names, as one comparable string. "No show" has one too. */
function showKey(config: BackendConfig): string {
	return config.show?.path ?? '';
}

/**
 * A name for a new show, as the folder it would become.
 *
 * The station decides for real; this is what to show somebody while they type, so
 * the two agree about what a name with a slash in it does. Deliberately not a slug —
 * an operator naming a show *Hänsel & Gretel* should find a folder called that.
 */
export function folderName(name: string): string {
	const cleaned = name
		.replace(/[/\\:*?"<>|]/g, '-')
		// Control characters, which a paste can carry and a filesystem cannot.
		.replace(/[\u0000-\u001f]/g, '-')
		.trim()
		.replace(/^\.+/, '')
		.trim();
	return `${(cleaned || 'Untitled Show').slice(0, 120)}.pult`;
}

/** The last part of a path, for showing a bundle by name rather than by route. */
export function baseName(path: string): string {
	const parts = path.split(/[/\\]/).filter(Boolean);
	return parts[parts.length - 1] ?? path;
}

/** Everything above it, for the second line of a card. */
export function parentPath(path: string): string {
	const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
	return cut > 0 ? path.slice(0, cut) : '';
}

/**
 * Recent shows, most recent first, with the ones that are no longer there last.
 *
 * Kept rather than dropped: a show on a stick that is not plugged in is exactly the
 * row somebody is looking for, and a list that forgot it the moment it went would be
 * one nobody could rely on. At the bottom because it is not the one they are about
 * to open.
 */
export function orderRecent(shows: ShowSummary[]): ShowSummary[] {
	return [...shows].sort((a, b) => {
		if (!!a.missing !== !!b.missing) return a.missing ? 1 : -1;
		return (b.lastOpened ?? '').localeCompare(a.lastOpened ?? '');
	});
}

/** A size in bytes, as a person would say it. */
export function asSize(bytes: number | undefined): string {
	if (!bytes || bytes < 1) return '—';
	const units = ['B', 'kB', 'MB', 'GB'];
	let n = bytes;
	let unit = 0;
	while (n >= 1024 && unit < units.length - 1) {
		n /= 1024;
		unit += 1;
	}
	return `${n < 10 && unit > 0 ? n.toFixed(1) : Math.round(n)} ${units[unit]}`;
}

/** A one-line summary of what is in a show, for a card. */
export function describe(show: ShowSummary): string {
	if (show.missing) return 'not where it was';
	if (show.problem) return show.problem;
	if (show.madeByAnotherBuild) return 'made by another version of this console';
	const parts: string[] = [];
	parts.push(`${show.fixtures ?? 0} ${show.fixtures === 1 ? 'fixture' : 'fixtures'}`);
	parts.push(`${show.cues ?? 0} ${show.cues === 1 ? 'cue' : 'cues'}`);
	if (show.versions) parts.push(`${show.versions} saved`);
	return parts.join(' · ');
}

/**
 * The links the welcome screen offers, built from the repository the binary was
 * compiled with rather than typed in here — so a fork's console points at the fork.
 */
export function links(repository: string): { label: string; href: string }[] {
	const base = repository.replace(/\.git$/, '').replace(/\/$/, '');
	if (!base) return [];
	return [
		{ label: 'Source', href: base },
		{ label: 'Issues', href: `${base}/issues` },
		{ label: 'Releases', href: `${base}/releases` },
		{ label: 'The spec', href: `${base}/blob/main/docs/SPEC.md` }
	];
}
