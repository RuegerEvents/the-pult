import { describe, expect, it } from 'vitest';

import { offer, onOffer } from './learn.js';

describe('what the operator touches, offered to a control being learned', () => {
	it('is heard while somebody listens, and not once they stop', () => {
		const heard: string[] = [];
		const stop = onOffer(({ action }) => heard.push(action.kind));
		offer({ kind: 'Master', sequence_id: 's' });
		stop();
		offer({ kind: 'Master', sequence_id: 's' });
		expect(heard).toEqual(['Master']);
	});
});
