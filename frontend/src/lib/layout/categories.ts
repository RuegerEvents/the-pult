/**
 * The six families a panel belongs to, and the one color each is drawn in.
 *
 * The color is the *category's* and never the panel's own. Six colors stay readable
 * at a glance on a tab strip; thirty-nine would be noise, and an operator looking for
 * "something to do with playback" reads green long before they read a word. The
 * panel's own identity is its icon.
 *
 * The order here is the order the picker draws its columns in: what somebody
 * programming reaches for first, then what runs the show, then the room, then what
 * the console itself is doing, and the errands last.
 */

export type Category = 'program' | 'playback' | 'automation' | 'stage' | 'system' | 'setup';

export type CategoryMeta = {
	label: string;
	/** A CSS color, always one of the `--cat-*` tokens in `styles/tokens.css`. */
	color: string;
};

export const CATEGORIES: Record<Category, CategoryMeta> = {
	program: { label: 'Program', color: 'var(--cat-program)' },
	playback: { label: 'Playback', color: 'var(--cat-playback)' },
	automation: { label: 'Automation', color: 'var(--cat-automation)' },
	stage: { label: 'Stage', color: 'var(--cat-stage)' },
	system: { label: 'System', color: 'var(--cat-system)' },
	setup: { label: 'Setup', color: 'var(--cat-setup)' }
};

export const CATEGORY_ORDER = Object.keys(CATEGORIES) as Category[];

export const isCategory = (value: unknown): value is Category =>
	typeof value === 'string' && value in CATEGORIES;

/** The color of a category, for a style attribute. Unknown falls back to System. */
export const categoryColor = (category: string | undefined): string =>
	isCategory(category) ? CATEGORIES[category].color : CATEGORIES.system.color;
