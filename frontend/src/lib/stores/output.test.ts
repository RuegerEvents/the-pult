import { describe, expect, it } from 'vitest';

import { reading } from './output.js';

/**
 * A frame is four floats a parameter, which a line of text does not fit in — so text
 * comes back marked as text and is asked for by name. A strip's Effect is the one that
 * matters: without it the stage views drew every node effect as `solid`.
 */
describe('a reading of the packed frame', () => {
	const FLOAT = 1;
	const TEXT = 5;
	const packed = new Float32Array([FLOAT, 0.5, 0, 0, TEXT, 0, 0, 0]);

	it('reads numbers out of the frame, and text by name, once', () => {
		const asked: string[] = [];
		const read = reading(1000, ['f/Intensity', 'f/Named:Effect'], packed, (key, at) => {
			asked.push(`${key}@${at}`);
			return 'comet';
		});
		expect(read.value('f', 'Intensity')).toEqual({ type: 'Float', value: 0.5 });
		expect(read.value('f', 'Named:Effect')).toEqual({ type: 'Text', value: 'comet' });
		expect(read.value('f', 'Named:Effect')).toEqual({ type: 'Text', value: 'comet' });
		expect(asked).toEqual(['f/Named:Effect@1000']);
	});

	it('asks for nothing that is not text, and nothing nobody reads', () => {
		const asked: string[] = [];
		const read = reading(1000, ['f/Intensity', 'f/Named:Effect'], packed, (key) => {
			asked.push(key);
			return 'x';
		});
		expect(read.value('f', 'Intensity')).not.toBeNull();
		expect(read.value('f', 'Unknown')).toBeNull();
		expect(asked).toEqual([]);
	});
});
