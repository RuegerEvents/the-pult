import { describe, it, expect } from 'vitest';

import type { NamedAsset } from './generated/index.js';
import { resolveAssetUrl } from './geometry.js';

const named = (name: string, asset: string): NamedAsset => ({
	id: name,
	name,
	asset,
	mime: 'image/jpeg'
});

/**
 * A `.3ds` asks for its texture by the bare name the archive carried, and the asset
 * store has no names in it. This is the bridge, and it is the whole reason
 * `named_assets` is a collection.
 */
describe('what a mesh asking for a file by name should fetch', () => {
	const names = [named('tx603.jpg', 'abc123'), named('truss 3m.glb', 'def456')];

	it('turns a name the archive carried into the asset it was stored as', () => {
		expect(resolveAssetUrl('tx603.jpg', names)).toBe('/assets/abc123');
	});

	it('looks at the last part, because a loader resolves against its own path', () => {
		expect(resolveAssetUrl('http://console/assets/textures/tx603.jpg', names)).toBe(
			'/assets/abc123'
		);
	});

	it('decodes a name a URL escaped', () => {
		expect(resolveAssetUrl('truss%203m.glb', names)).toBe('/assets/def456');
	});

	it('leaves a lone percent sign alone rather than throwing', () => {
		expect(resolveAssetUrl('100%.jpg', names)).toBe('100%.jpg');
	});

	it('leaves an asset URL and an embedded texture alone', () => {
		expect(resolveAssetUrl('/assets/already', names)).toBe('/assets/already');
		expect(resolveAssetUrl('data:image/png;base64,AAAA', names)).toBe(
			'data:image/png;base64,AAAA'
		);
	});

	it('leaves a name nothing in the archive matched alone', () => {
		expect(resolveAssetUrl('nobody.jpg', names)).toBe('nobody.jpg');
	});
});
