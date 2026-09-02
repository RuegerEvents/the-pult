/**
 * The transforms corpus, from the browser's side.
 *
 * `testdata/transforms.json` is read by this and by
 * `crates/pult-schema/tests/transforms.rs`. Composing a parent chain happens on a
 * station when a group is resolved and here on every frame of a drag, so there are
 * two of it; this is how the two are held to each other.
 *
 * The corpus's `matrices` half starts from an MVR matrix, which only the station can
 * parse, so it is read by `pult-backend` and not here.
 */
import { readFileSync } from 'node:fs';

import { describe, it, expect } from 'vitest';

import type { SceneObject, Transform, Vec3 } from './generated/index.js';
import { at, byId, facing, facingTransform, IDENTITY, worldTransform } from './scene.js';

type ChainCase = {
	name: string;
	objects: { id: string; parent: string | null; transform: Transform }[];
	local: Transform;
	parent: string | null;
	world: Transform;
};

const corpus: { chains: ChainCase[] } = JSON.parse(
	readFileSync(new URL('../../../testdata/transforms.json', import.meta.url), 'utf8')
);

/** A corpus case names the three fields a chain composes from, not a whole object. */
function asObject(placed: ChainCase['objects'][number]): SceneObject {
	return {
		id: placed.id,
		name: '',
		kind: 'Truss',
		transform: placed.transform,
		parent: placed.parent,
		layer: null,
		class: null,
		geometry: [],
		symbol: null
	};
}

function near(got: Vec3, want: Vec3, what: string, name: string) {
	for (const axis of ['x', 'y', 'z'] as const) {
		expect(
			Math.abs(got[axis] - want[axis]),
			`${name}: ${what}.${axis} is ${got[axis]}, expected ${want[axis]}`
		).toBeLessThan(1e-3);
	}
}

describe('the corpus the station agrees with', () => {
	for (const chain of corpus.chains) {
		it(chain.name, () => {
			const objects = byId(chain.objects.map(asObject));
			const got = worldTransform(chain.local, chain.parent, objects);
			near(got.position, chain.world.position, 'position', chain.name);
			near(got.rotation, chain.world.rotation, 'rotation', chain.name);
			near(got.scale, chain.world.scale, 'scale', chain.name);
		});
	}
});

describe('composing a chain', () => {
	it('leaves an orphan where it is', () => {
		const local = { position: { x: 1, y: 2, z: 3 }, rotation: { x: 10, y: 20, z: 30 }, scale: { x: 1, y: 2, z: 3 } };

		const got = worldTransform(local, null, new Map());

		near(got.position, local.position, 'position', 'an orphan');
		near(got.rotation, local.rotation, 'rotation', 'an orphan');
		near(got.scale, local.scale, 'scale', 'an orphan');
	});

	it('stops rather than hangs on a chain that loops', () => {
		const a = 'aaaaaaaa-0000-4000-8000-000000000001';
		const b = 'aaaaaaaa-0000-4000-8000-000000000002';
		const placed = (id: string, parent: string): SceneObject =>
			asObject({ id, parent, transform: at({ x: 1, y: 0, z: 0 }) });

		const got = worldTransform(IDENTITY, a, byId([placed(a, b), placed(b, a)]));

		expect(got.position.x).toBeGreaterThan(0);
	});
});

describe('which way a fixture faces', () => {
	it('hangs straight down when nothing has turned it', () => {
		near(facing(IDENTITY), { x: 0, y: -1, z: 0 }, 'facing', 'unturned');
	});

	it('points where it was aimed', () => {
		for (const direction of [
			{ x: 0, y: -1, z: 0 },
			{ x: 1, y: 0, z: 0 },
			{ x: 0, y: 0, z: -1 },
			{ x: 0.5, y: -0.5, z: 0.7 }
		]) {
			const length = Math.hypot(direction.x, direction.y, direction.z);
			const unit = { x: direction.x / length, y: direction.y / length, z: direction.z / length };

			const got = facing(facingTransform({ x: 0, y: 0, z: 0 }, direction));

			near(got, unit, 'facing', JSON.stringify(direction));
		}
	});
});
