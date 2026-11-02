import { describe, expect, it } from 'vitest';

import { learnable } from './wing.js';

describe('learning from what the operator touches', () => {
	const parameter = { kind: 'Parameter' as const, fixture_id: 'f', parameter: 'Intensity' as const };
	const master = { kind: 'Master' as const, sequence_id: 's' };
	const go = { kind: 'Press' as const, target: { kind: 'SequenceGo' as const, sequence_id: 's' } };

	it('gives a fader or an encoder a parameter or a level, and never a press', () => {
		expect(learnable({ kind: 'Fader', index: 0 }, parameter)).toEqual(parameter);
		expect(learnable({ kind: 'Fader', index: 0 }, master)).toEqual(master);
		expect(learnable({ kind: 'Fader', index: 0 }, go)).toBeNull();
		expect(learnable({ kind: 'Encoder', index: 13 }, master)).toEqual(master);
		expect(learnable({ kind: 'Encoder', index: 13 }, go)).toBeNull();
	});

	it('gives a key a press, or a flash of the parameter it was shown', () => {
		expect(learnable({ kind: 'Key', index: 283 }, go)).toEqual(go);
		expect(learnable({ kind: 'Key', index: 283 }, parameter)).toEqual({
			kind: 'Flash',
			fixture_id: 'f',
			parameter: 'Intensity'
		});
		expect(learnable({ kind: 'Key', index: 283 }, master)).toBeNull();
	});
});

describe('the digital inputs', () => {
	it('learns like a key, a flash of the parameter it was shown included', () => {
		const go = { kind: 'Press' as const, target: { kind: 'SequenceGo' as const, sequence_id: 's' } };
		expect(learnable({ kind: 'Digital', index: 3 }, go)).toEqual(go);
		expect(learnable({ kind: 'Digital', index: 3 }, { kind: 'Master', sequence_id: 's' })).toBeNull();
	});
});

