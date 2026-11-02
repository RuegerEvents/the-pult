/**
 * The part of the show a surface reads, as one store — so the phone view and the
 * editor both hand `$lib/surface.ts` the same shape.
 */

import { derived, type Readable } from 'svelte/store';

import type { SurfaceShow } from '../surface.js';
import { collection } from './show.js';

export const surfaceShow: Readable<SurfaceShow> = derived(
	[
		collection('variables'),
		collection('timelines'),
		collection('sequences'),
		collection('flows'),
		collection('flow_nodes'),
		collection('fixtures'),
		collection('fixture_types')
	],
	([variables, timelines, sequences, flows, flowNodes, fixtures, types]) => ({
		variables,
		timelines,
		sequences,
		flows,
		flowNodes,
		fixtures,
		types
	})
);
