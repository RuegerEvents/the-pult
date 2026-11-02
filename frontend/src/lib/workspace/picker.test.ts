import { describe, expect, it } from 'vitest';

import type { PanelMeta } from './panels.js';
import { flatEntries, pickerColumns } from './picker.js';

const stub = {} as PanelMeta['component'];
const meta = (title: string, category: PanelMeta['category'], home?: PanelMeta['home']): PanelMeta => ({
	title,
	category,
	icon: stub,
	component: stub,
	fills: false,
	home
});

const panels: Record<string, PanelMeta> = {
	values: meta('Programmer', 'program'),
	cues: meta('Cues', 'playback'),
	timeline: meta('Timeline', 'playback'),
	patch: meta('Patch', 'setup', 'both'),
	settings: meta('Settings', 'setup', 'setup')
};

describe('the panel picker', () => {
	it('draws one column per category, in category order, and leaves empty ones out', () => {
		const columns = pickerColumns(panels, new Set());
		expect(columns.map((c) => c.category)).toEqual(['program', 'playback', 'setup']);
	});

	it('offers an open panel as "go to" rather than hiding it', () => {
		const columns = pickerColumns(panels, new Set(['cues']));
		const cues = flatEntries(columns).find((e) => e.id === 'cues');
		expect(cues?.action).toBe('goto');
	});

	it('sends an errand to Setup, and a both-panel to the workspace', () => {
		const flat = flatEntries(pickerColumns(panels, new Set()));
		expect(flat.find((e) => e.id === 'settings')?.action).toBe('setup');
		expect(flat.find((e) => e.id === 'patch')?.action).toBe('open');
	});

	it('searches across every column', () => {
		const flat = flatEntries(pickerColumns(panels, new Set(), 'tim'));
		expect(flat.map((e) => e.id)).toEqual(['timeline']);
	});
});
