/**
 * Every panel says what family it is and what it looks like.
 *
 * TypeScript already refuses a `PANELS` entry without a category or an icon; this is
 * the same rule as a test, read from the source so it needs none of the panels'
 * components (three.js, pdf.js) to load — and it also checks the colors, which no
 * type can: each category's color has to read on the chrome it is drawn on.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { CATEGORIES, CATEGORY_ORDER } from './categories.js';

const here = (name: string) => fileURLToPath(new URL(name, import.meta.url));
const source = readFileSync(here('./panels.ts'), 'utf8');
const tokens = readFileSync(here('../styles/tokens.css'), 'utf8');

/** The registry's entries, one per line: `id: { title: …, … }`. */
const entries = [...source.slice(source.indexOf('export const PANELS = {')).matchAll(/^\t(\w+): \{ (.*) \},?$/gm)];

describe('the panel registry', () => {
	it('has entries to check', () => {
		expect(entries.length).toBeGreaterThan(30);
	});

	it.each(entries.map((m) => [m[1], m[2]]))('%s names a category and an icon', (_id, body) => {
		const category = body.match(/category: '(\w+)'/)?.[1];
		expect(category, 'a category').toBeDefined();
		expect(CATEGORY_ORDER).toContain(category);
		expect(body, 'an icon').toMatch(/icon: \w+/);
	});

	it('never gives two panels the same icon', () => {
		const icons = entries.map((m) => m[2].match(/icon: (\w+)/)?.[1]);
		expect(new Set(icons).size).toBe(icons.length);
	});
});

/** WCAG relative luminance of `#rrggbb`. */
function luminance(hex: string): number {
	const [r, g, b] = [1, 3, 5].map((i) => {
		const c = parseInt(hex.slice(i, i + 2), 16) / 255;
		return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
	});
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const contrast = (a: string, b: string) => {
	const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
	return (hi + 0.05) / (lo + 0.05);
};

const token = (name: string): string => {
	const value = tokens.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{3,6})`))?.[1];
	if (!value) throw new Error(`no --${name}`);
	return value.length === 4 ? `#${[...value.slice(1)].map((c) => c + c).join('')}` : value;
};

describe('the category colors', () => {
	it.each(CATEGORY_ORDER)('%s reads at 7:1 or better on the chrome', (category) => {
		expect(CATEGORIES[category].color).toBe(`var(--cat-${category})`);
		expect(contrast(token(`cat-${category}`), token('bg-chrome'))).toBeGreaterThanOrEqual(7);
	});
});
