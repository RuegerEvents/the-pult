import { describe, expect, it } from 'vitest';

import type { NetInterface, StationNetwork } from './generated/index.js';
import { choicesFor, faultText, faults, serviceName, slotText } from './network.js';

function iface(name: string, addresses: string[], extra: Partial<NetInterface> = {}): NetInterface {
	return { name, addresses, up: true, loopback: name.startsWith('lo'), ...extra };
}

function cabling(over: Partial<StationNetwork> = {}): StationNetwork {
	return {
		id: '00000000-0000-0000-0000-000000000001',
		interfaces: [],
		bindings: [],
		changed_at: new Date().toISOString(),
		...over
	} as StationNetwork;
}

/** Every value on offer, in order, ignoring which group it landed in. */
const values = (groups: ReturnType<typeof choicesFor>) =>
	groups.flatMap((g) => g.choices.map((c) => c.value));

describe('what a dropdown offers', () => {
	it('names each interface once, and each alias as well', () => {
		// The aliased card is the whole reason an address may be stored at all: `en5`
		// cannot say which of 2.0.0.1 and 10.0.1.7 an Art-Net output should leave by.
		const network = cabling({
			interfaces: [iface('en0', ['192.168.1.20']), iface('en5', ['10.0.1.7', '2.0.0.1'])]
		});
		expect(values(choicesFor(network))).toEqual(['en0', 'en5', '10.0.1.7', '2.0.0.1']);
	});

	it('does not offer a single address twice', () => {
		// One address on a card is fully named by the card; two ways to say one thing
		// is a choice nobody can make correctly.
		expect(values(choicesFor(cabling({ interfaces: [iface('en0', ['192.168.1.20'])] })))).toEqual(
			['en0']
		);
	});

	it('still offers a port with no address, below the ready ones', () => {
		// The measurement that decided this: on a laptop with 24 interfaces and 3
		// addresses, the ones without are `en1`–`en6` — the adapter ports, which is
		// exactly where a show LAN gets plugged in. Hiding them would put the whole
		// re-resolve-on-the-probe-tick rule out of reach of the UI, since its entire
		// point is that a cable can be named before it is ready.
		const network = cabling({
			interfaces: [iface('en0', ['192.168.1.20']), iface('en5', []), iface('gif0', [], { up: false })]
		});
		const groups = choicesFor(network);
		expect(groups.map((g) => g.label)).toEqual(['Ready', 'Not configured yet']);
		expect(groups[0].choices.map((c) => c.value)).toEqual(['en0']);
		expect(groups[1].choices.map((c) => c.value)).toEqual(['en5', 'gif0']);
		expect(values(groups)).toContain('en5');
	});

	it('says why a port is not ready rather than only that it is not', () => {
		const groups = choicesFor(cabling({ interfaces: [iface('en9', [])] }));
		expect(groups[0].choices[0].detail).toContain('no IPv4 address');
	});

	it('offers nothing at all rather than inventing a list', () => {
		expect(choicesFor(null)).toEqual([]);
		expect(choicesFor(cabling())).toEqual([]);
	});
});

describe('faults', () => {
	it('reads every shape the station can send', () => {
		expect(faultText({ Unknown: 'eth9' })).toContain('no interface called eth9');
		expect(faultText({ NoAddress: 'en9' })).toContain('no IPv4 address');
		expect(faultText({ NotHere: '10.9.9.9' })).toContain('not an address');
		expect(faultText({ NotIpv4: '::1' })).toContain('IPv4 only');
	});

	it('counts what is broken on a station, including a peer', () => {
		const network = cabling({
			bindings: [
				{ service: 'Session', label: 'session', wanted: 'en5', bound: '10.0.1.7', fault: null },
				{
					service: 'OpenHaunt',
					label: 'OpenHaunt',
					wanted: 'eth9',
					bound: null,
					fault: { Unknown: 'eth9' }
				}
			]
		});
		expect(faults(network)).toBe(1);
		expect(faults(null)).toBe(0);
	});
});

describe('an output binding is told apart from a service one', () => {
	it('by the variant carrying a row id', () => {
		expect(serviceName('Session')).toBe('Session');
		expect(serviceName({ Output: '00000000-0000-0000-0000-000000000009' })).toBe('Output');
	});
});

describe('the sACN ladder in words', () => {
	it('puts a leader at the top rather than in a slot', () => {
		expect(slotText({ is_leader: true, sacn_slot: null })).toContain('100');
	});

	it('says a follower has not claimed one yet rather than printing a number', () => {
		// The three-state rule the clock column already follows: no slot and slot zero
		// are different things, and one figure meaning both is the plausible wrong
		// number this whole mechanism exists to remove.
		expect(slotText({ is_leader: false, sacn_slot: null })).toBe('no slot yet');
		expect(slotText({ is_leader: false, sacn_slot: 90 })).toContain('90');
	});
});
