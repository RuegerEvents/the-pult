/**
 * The command palette's registry: everything ⌘K can find.
 *
 * One registry and no list. A *source* is a function that answers the commands it
 * offers right now — the panels, the objects of every registered kind, a panel's own
 * verbs — and the palette asks every source each time it opens. So a panel adds
 * commands by registering a source, the palette never learns what a panel is, and a
 * preset created a second ago is findable without anybody having told the palette.
 */

import type { Component } from 'svelte';
import { get, writable } from 'svelte/store';

import Lock from '@lucide/svelte/icons/lock';
import LockOpen from '@lucide/svelte/icons/lock-open';
import Terminal from '@lucide/svelte/icons/terminal';

import { fuzzyFilter } from '$lib/fuzzy.js';
import type { Category } from '$lib/layout/categories.js';
import { objectKinds, selectSubject, type ObjectKind } from '$lib/objects.js';
import { pathOf } from '$lib/stores/folders.js';
import { findPanel } from '$lib/layout.js';
import { revealPanel, tree } from '$lib/stores/layout.js';
import { allPanels, focusConsole } from '$lib/stores/plugins.js';
import { openSetup } from '$lib/stores/setup.js';
import { showLocked, toggleShowLock } from '$lib/stores/showLock.js';
import { panelHome } from '$lib/layout/panels.js';

export type Command = {
	/** Unique across sources, so a list can key on it. */
	id: string;
	title: string;
	/** Which heading it is listed under. */
	section: 'Panels' | 'Objects' | 'Console';
	/** A word on the right: "go to", "Recall", "in Setup". */
	hint?: string;
	category?: Category;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	icon?: Component<any>;
	/** More words it should be found by. */
	keywords?: string;
	run: () => unknown;
};

export type CommandSource = () => Command[];

const sources = writable<Map<string, CommandSource>>(new Map());

/** Offer commands. Answers a function that withdraws them. */
export function registerCommands(name: string, source: CommandSource): () => void {
	sources.update((all) => new Map(all).set(name, source));
	return () =>
		sources.update((all) => {
			if (all.get(name) !== source) return all;
			const next = new Map(all);
			next.delete(name);
			return next;
		});
}

/** Every command every source offers now. */
export function allCommands(): Command[] {
	return [...get(sources).values()].flatMap((source) => {
		try {
			return source();
		} catch {
			// A source that throws loses its own commands, not the palette.
			return [];
		}
	});
}

/** What matches `query`, best first. Panels and objects alike, by title and keywords. */
export function searchCommands(query: string, commands: Command[] = allCommands()): Command[] {
	return fuzzyFilter(query, commands, (c) => (c.keywords ? `${c.title} ${c.keywords}` : c.title));
}

// ── The sources every console has ─────────────────────────────────────────────

/** Every panel: open it, or go to it where it already is. */
export const panelCommands: CommandSource = () =>
	Object.entries(get(allPanels)).map(([id, meta]) => {
		const open = findPanel(get(tree), id) !== null;
		const setup = !open && panelHome(meta) === 'setup';
		return {
			id: `panel:${id}`,
			title: meta.title,
			section: 'Panels' as const,
			hint: open ? 'go to' : setup ? 'in Setup' : 'open',
			category: meta.category,
			icon: meta.icon,
			keywords: 'panel',
			run: () => (setup ? openSetup(id) : revealPanel(id))
		};
	});

/** Every object of every kind: tap-equivalent first, inspect beside it. */
export function objectCommands(kinds: Map<string, ObjectKind>): Command[] {
	// Where each object is filed, so "scares" finds every timeline in the Scares folder.
	const path = get(pathOf);
	return [...kinds.values()].flatMap((kind) =>
		get(kind.rows).flatMap((row) => {
			const where = path(row.folder);
			const act: Command[] = kind.act
				? [
						{
							id: `act:${kind.kind}:${row.id}`,
							title: row.name,
							section: 'Objects',
							hint: kind.actLabel ?? 'Use',
							category: kind.category,
							icon: kind.icon,
							keywords: where ? `${kind.label} ${where}` : kind.label,
							run: () => kind.act!(row)
						}
					]
				: [];
			return [
				...act,
				{
					id: `inspect:${kind.kind}:${row.id}`,
					title: row.name,
					section: 'Objects' as const,
					hint: 'Inspect',
					category: kind.category,
					icon: kind.icon,
					keywords: `${kind.label} inspect edit${where ? ` ${where}` : ''}`,
					run: () => {
						selectSubject({ kind: kind.kind, id: row.id });
						revealPanel('inspector');
					}
				}
			];
		})
	);
}

/**
 * The console's own: the Show lock, and the command line where a plugin offers one.
 *
 * ⌘K used to land in the command line directly. It opens this palette now, and the
 * command line is one Enter away from it — found by name like everything else, and
 * focused where it is open or opened where it is not.
 */
export const consoleCommands: CommandSource = () => {
	const consoles = Object.entries(get(allPanels)).filter(
		([id, meta]) => id.startsWith('plugin:') && meta.props?.surfaceId !== undefined && meta.fills
	);
	return [
	{
		id: 'console:show-lock',
		title: get(showLocked) ? 'Unlock the show' : 'Lock the show',
		section: 'Console',
		icon: get(showLocked) ? LockOpen : Lock,
		keywords: 'show lock edit protect',
		run: toggleShowLock
	},
	...consoles.map(([id, meta]) => ({
		id: `console:${id}`,
		title: meta.title,
		section: 'Console' as const,
		hint: 'focus',
		icon: Terminal,
		keywords: 'command line cli prompt type',
		run: () => {
			revealPanel(id);
			// The surface registers its focus when it mounts, a frame after it opens.
			requestAnimationFrame(() => focusConsole());
		}
	}))
	];
};

/** Register the sources every console has. Called once by the root layout. */
export function registerBuiltInCommands(): void {
	registerCommands('panels', panelCommands);
	registerCommands('objects', () => objectCommands(get(objectKinds)));
	registerCommands('console', consoleCommands);
}
