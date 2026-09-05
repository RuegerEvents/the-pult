/**
 * A whole sheet, composed.
 *
 * The cases are the ones a sheet can get quietly wrong: a scale that claims to be
 * something a rule cannot measure, a drawing that shows fewer heads than the patch
 * says exist and says nothing about it, and a perspective viewport captioned with a
 * ratio somebody could measure off.
 */

import { readFile } from 'node:fs/promises';
import { describe, expect, it } from 'vitest';

import type {
	Fixture,
	FixtureType,
	Production,
	SceneObject,
	Sheet,
	Vec3
} from '../generated/index.js';
import { forgetFont, metrics, type Metrics } from './font.js';
import { composeSheet, scaleLabel, snap, type SheetContext } from './sheet.js';

async function realMetrics(): Promise<Metrics> {
	forgetFont();
	return metrics(async (url) => new Uint8Array(await readFile(`static${url}`)));
}

const NOWHERE: Production = {
	title: 'The Greatest Showman',
	venue: 'The Corn Exchange',
	address: 'Market Square, Northgate',
	dates: '02.05.2025 - 05.05.2025',
	designer: 'Halliwell Lighting Design',
	contact: 'studio@halliwell-ld.example',
	revision: ''
};

const IDENTITY = {
	position: { x: 0, y: 0, z: 0 },
	rotation: { x: 0, y: 0, z: 0 },
	scale: { x: 1, y: 1, z: 1 }
};

function aFixture(id: string, at: Vec3 | null, parent: string | null = null): Fixture {
	return {
		id,
		name: `Spikie ${id}`,
		fixture_type_id: 'type',
		address: { Dmx: { mode: 'Default', breaks: [{ universe: 1, address: 1 }] } },
		position: at ? { ...IDENTITY, position: at } : null,
		parent,
		mount: null,
		layer: null,
		class: null,
		focus: null,
		fixture_number: 1,
		unit_number: null,
		sensed_values: {},
		live_effects: {},
		live_fades: {},
		home_values: {}
	} as unknown as Fixture;
}

const A_TYPE = {
	id: 'type',
	name: 'Spikie',
	manufacturer: 'Robe',
	short_name: 'SPK',
	long_name: '',
	description: '',
	channel_count: 20,
	parameters: [],
	dmx_modes: [],
	physical: {
		weight_kg: 20,
		power_w: 470,
		dimensions_m: { x: 0.3, y: 0.5, z: 0.3 },
		connectors: [],
		leg_height_m: null,
		operating_temperature: null,
		beam_angle_deg: 20
	},
	geometry: [],
	source: 'Manual',
	plan_symbol: 'Auto',
	thumbnail: null
} as unknown as FixtureType;

function aTruss(id: string): SceneObject {
	return {
		id,
		name: 'Front truss',
		kind: 'Truss',
		transform: IDENTITY,
		parent: null,
		layer: null,
		class: null,
		geometry: [],
		symbol: null,
		catalogue: 'f34-3m',
		properties: null,
		locked: false,
		weight_kg: null
	} as unknown as SceneObject;
}

function aSheet(over: Partial<Sheet> = {}): Sheet {
	return {
		id: 'sheet',
		name: 'Fixtures',
		sort_order: 0,
		paper: 'A3',
		landscape: true,
		frame: true,
		title_block: true,
		blocks: [
			{
				type: 'Viewport',
				rect: { x: 18, y: 18, w: 384, h: 220 },
				title: 'Plan',
				view: 'Plan',
				projection: 'Orthographic',
				scale: { type: 'Fit' },
				style: { type: 'Drafting', lines: 'Hidden', ink: 'Mono' },
				layers: null,
				labels: ['Name'],
				scale_bar: true,
				orientation_mark: true
			}
		],
		...over
	} as unknown as Sheet;
}

function context(over: Partial<SheetContext>, font: Metrics): SheetContext {
	return {
		metrics: font,
		showName: 'Kelter',
		production: NOWHERE,
		sheetNumber: 1,
		sheetCount: 6,
		fixtures: [],
		fixtureTypes: [A_TYPE],
		sceneObjects: [],
		layers: [],
		sizes: new Map(),
		tables: new Map(),
		rasters: new Map(),
		thumbnails: new Map(),
		...over
	};
}

const textsOf = (items: { kind: string }[]) =>
	items.filter((i) => i.kind === 'text').map((i) => (i as unknown as { text: string }).text);

describe('the page', () => {
	it('is A3 landscape in millimetres', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(aSheet(), context({}, font));
		expect(drawing.width).toBe(420);
		expect(drawing.height).toBe(297);
	});

	it('turns to portrait when the sheet says so', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(aSheet({ landscape: false }), context({}, font));
		expect(drawing.width).toBe(297);
		expect(drawing.height).toBe(420);
	});
});

describe('the title block', () => {
	it('prints the production, the venue and who drew it', async () => {
		const font = await realMetrics();
		const texts = textsOf(composeSheet(aSheet(), context({}, font)).items);
		expect(texts).toContain('The Greatest Showman');
		expect(texts).toContain('The Corn Exchange');
		expect(texts).toContain('Halliwell Lighting Design');
		expect(texts).toContain('Fixtures');
		expect(texts).toContain('1/6');
	});

	it('omits an empty line rather than printing a blank one', async () => {
		const font = await realMetrics();
		const bare = composeSheet(
			aSheet(),
			context(
				{
					production: { ...NOWHERE, venue: '', address: '', revision: '' }
				},
				font
			)
		);
		expect(textsOf(bare.items)).not.toContain('');
	});

	it('drops a title-block heading when there is nothing under it', async () => {
		// "Drawing" over an empty box reads as a title block that failed to print.
		const font = await realMetrics();
		const texts = textsOf(
			composeSheet(
				aSheet(),
				context({ production: { ...NOWHERE, designer: '', contact: '', revision: '' } }, font)
			).items
		);
		expect(texts).not.toContain('Drawing');
		expect(texts).toContain('Production');
	});

	it('falls back to the showfile name when no production is named', async () => {
		const font = await realMetrics();
		const texts = textsOf(
			composeSheet(aSheet(), context({ production: { ...NOWHERE, title: '' } }, font)).items
		);
		expect(texts).toContain('Kelter');
	});
});

describe('scale', () => {
	it('snaps a fitted ratio up to one a rule is cut for', () => {
		// The reference drawing prints 1:35, which no rule measures.
		expect(snap(35)).toBe(50);
		expect(snap(12)).toBe(20);
		expect(snap(50)).toBe(50);
	});

	it('prints NTS for a perspective viewport and never a ratio', () => {
		expect(scaleLabel('Perspective', 50)).toBe('NTS');
		expect(scaleLabel('Orthographic', 50)).toBe('1:50');
	});

	it('prints NTS for a picture viewport, orthographic or not', () => {
		// A raster is fitted by the rig renderer, not by this code, so claiming a ratio
		// for one is claiming a number nothing here worked out. It printed `1:0`.
		expect(scaleLabel('Orthographic', 0, true)).toBe('NTS');
		expect(scaleLabel('Orthographic', 50, true)).toBe('NTS');
		expect(scaleLabel('Orthographic', 0)).toBe('NTS');
	});

	it('captions a drafting viewport with the scale it drew at', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet(),
			context(
				{
					sceneObjects: [aTruss('truss')],
					sizes: new Map([['truss', { x: 3, y: 0.29, z: 0.29 }]]),
					fixtures: [aFixture('a', { x: 0, y: 5, z: 0 }, 'truss')]
				},
				font
			)
		);
		expect(textsOf(drawing.items).some((t) => /^1:\d+/.test(t))).toBe(true);
	});
});

describe('what a drawing does not show', () => {
	it('says so when a fixture could not be placed', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet(),
			context({ fixtures: [aFixture('a', { x: 0, y: 5, z: 0 }), aFixture('b', null)] }, font)
		);
		const texts = textsOf(drawing.items);
		expect(texts.some((t) => t.includes('not placed'))).toBe(true);
	});

	it('says nothing when everything is drawn', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet(),
			context({ fixtures: [aFixture('a', { x: 0, y: 5, z: 0 })] }, font)
		);
		expect(textsOf(drawing.items).some((t) => t.includes('not placed'))).toBe(false);
	});

	it('never captions a picture viewport with a ratio', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet({
				blocks: [
					{
						type: 'Viewport',
						rect: { x: 18, y: 18, w: 190, h: 200 },
						title: 'Axonometric',
						view: 'ThreeQuarter',
						projection: 'Orthographic',
						scale: { type: 'Fit' },
						style: { type: 'Picture', mode: 'Real', work_light: 0.35, dpi: 300 },
						layers: null,
						labels: [],
						scale_bar: false,
						orientation_mark: false
					}
				]
			} as unknown as Partial<Sheet>),
			context({}, font)
		);
		const texts = textsOf(drawing.items);
		expect(texts).toContain('NTS');
		expect(texts.some((t) => /^1:/.test(t))).toBe(false);
	});

	it('says a picture viewport was not rendered rather than leaving a hole', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet({
				blocks: [
					{
						type: 'Viewport',
						rect: { x: 18, y: 18, w: 190, h: 200 },
						title: 'Axonometric',
						view: 'ThreeQuarter',
						projection: 'Orthographic',
						scale: { type: 'Fit' },
						style: { type: 'Picture', mode: 'Real', work_light: 0.35, dpi: 300 },
						layers: null,
						labels: [],
						scale_bar: false,
						orientation_mark: false
					}
				]
			} as unknown as Partial<Sheet>),
			context({}, font)
		);
		expect(textsOf(drawing.items).some((t) => t.includes('not been rendered'))).toBe(true);
	});
});

describe('dimensions', () => {
	/**
	 * The shape every demo builds: a `Group` handle with truss sections under it, and
	 * the lights clamped to the *handle*. The first version of this dimensioned the
	 * sections instead and drew nothing at all, because no fixture is parented to one.
	 */
	function aRun() {
		const handle = { ...aTruss('run'), name: 'FOH bar', kind: 'Group', catalogue: null };
		const sections = [-4.5, -1.5, 1.5, 4.5].map((x, i) => ({
			...aTruss(`s${i}`),
			parent: 'run',
			transform: { ...IDENTITY, position: { x, y: 0, z: 0 } }
		}));
		return { handle, sections } as unknown as {
			handle: SceneObject;
			sections: SceneObject[];
		};
	}

	function withDimensions(): Sheet {
		const block = { ...(aSheet().blocks[0] as object) } as Record<string, unknown>;
		block.dimensions = { datum: 'Left', running: true, above: false };
		block.labels = [];
		return aSheet({ blocks: [block] } as unknown as Partial<Sheet>);
	}

	it('measures along the handle the lights actually hang from', async () => {
		const font = await realMetrics();
		const { handle, sections } = aRun();
		const sizes = new Map<string, Vec3>([['run', { x: 0, y: 0, z: 0 }]]);
		for (const section of sections) sizes.set(section.id, { x: 3, y: 0.29, z: 0.29 });
		sizes.delete('run');

		const drawing = composeSheet(
			withDimensions(),
			context(
				{
					sceneObjects: [handle, ...sections],
					sizes,
					fixtures: [
						aFixture('a', { x: -3, y: 5, z: 0 }, 'run'),
						aFixture('b', { x: 0, y: 5, z: 0 }, 'run'),
						aFixture('c', { x: 3, y: 5, z: 0 }, 'run')
					]
				},
				font
			)
		);
		const texts = textsOf(drawing.items);
		// Three metres between each pair, and the run is 12 m so the first head is 3 m
		// from the left end.
		expect(texts).toContain('3000');
		expect(texts.filter((t) => t === '3000').length).toBeGreaterThanOrEqual(2);
	});

	it('draws none when the viewport does not ask for them', async () => {
		const font = await realMetrics();
		const { handle, sections } = aRun();
		const sizes = new Map<string, Vec3>();
		for (const section of sections) sizes.set(section.id, { x: 3, y: 0.29, z: 0.29 });
		const drawing = composeSheet(
			aSheet(),
			context(
				{
					sceneObjects: [handle, ...sections],
					sizes,
					fixtures: [aFixture('a', { x: -3, y: 5, z: 0 }, 'run')]
				},
				font
			)
		);
		expect(textsOf(drawing.items)).not.toContain('3000');
	});
});

describe('labels', () => {
	it('draws one per fixture when the viewport asks for them', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet(),
			context(
				{
					fixtures: [
						aFixture('a', { x: -2, y: 5, z: 0 }),
						aFixture('b', { x: 2, y: 5, z: 0 })
					]
				},
				font
			)
		);
		const texts = textsOf(drawing.items);
		expect(texts).toContain('Spikie a');
		expect(texts).toContain('Spikie b');
	});

	it('draws none when the viewport asks for none', async () => {
		const font = await realMetrics();
		const drawing = composeSheet(
			aSheet({
				blocks: [{ ...(aSheet().blocks[0] as object), labels: [] }]
			} as unknown as Partial<Sheet>),
			context({ fixtures: [aFixture('a', { x: 0, y: 5, z: 0 })] }, font)
		);
		expect(textsOf(drawing.items)).not.toContain('Spikie a');
	});
});
