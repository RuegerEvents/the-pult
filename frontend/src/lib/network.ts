/**
 * Reading a station's cabling.
 *
 * The rule is `pult_schema::types::network` and it stays there — nothing in here
 * resolves a name, decides a priority or judges a fault. This is the panel's half:
 * turning what a station published into the words and the orderings a person reads.
 *
 * Which is worth saying because the temptation is the other way. A dropdown wants to
 * know whether an interface would work, and answering that here would be a second
 * implementation of `resolve` that disagrees with the station's on exactly the cases
 * nobody tests — an alias, a v6-only card, an interface that is down. So the page
 * offers what the station said it has and reports what the station said happened,
 * and never predicts.
 */

import type {
	InterfaceError,
	NetInterface,
	NetService,
	StationNetwork
} from './generated/index.js';

/** The six keys of the `[network]` section, as the preferences route carries them. */
export type NetworkPrefs = {
	http?: string | null;
	session?: string | null;
	mvrXchange?: string | null;
	openhaunt?: string | null;
	artnet?: string | null;
	sacn?: string | null;
};

/** What each key governs, in the order the panel lists them. */
export const NETWORK_SERVICES = [
	{
		key: 'http' as const,
		name: 'Console page',
		what: 'The page, the WebSocket, and a hosted MVR-xchange group.',
		// Said on the panel because it is the one service that behaves differently
		// when it cannot have what it was asked for, and an operator who is not told
		// would read the fault as "the setting did nothing".
		listener: true
	},
	{
		key: 'session' as const,
		name: 'Session',
		what: 'Finding other consoles running this show, and being found by them.',
		listener: true
	},
	{
		key: 'mvrXchange' as const,
		name: 'MVR-xchange',
		what: 'The previz or CAD seat this console shares its rig with.',
		listener: false
	},
	{
		key: 'openhaunt' as const,
		name: 'OpenHaunt',
		what: 'Finding nodes, and the broker this station runs for them.',
		listener: false
	},
	{
		key: 'artnet' as const,
		name: 'Art-Net',
		what: 'What an Art-Net output uses when its own row names no cable.',
		listener: false
	},
	{
		key: 'sacn' as const,
		name: 'sACN',
		what: 'What an sACN output uses when its own row names no cable.',
		listener: false
	}
];

/** A fault in the words the station would have used. */
export function faultText(fault: InterfaceError): string {
	if ('Unknown' in fault) return `no interface called ${fault.Unknown} on that machine`;
	if ('NoAddress' in fault) return `${fault.NoAddress} has no IPv4 address`;
	if ('NotHere' in fault) return `${fault.NotHere} is not an address of any interface there`;
	return `${fault.NotIpv4} is not IPv4, and this console binds IPv4 only`;
}

/** A service's name, for a row that is only ever identified by its variant. */
export function serviceName(service: NetService): string {
	if (typeof service !== 'string') return 'Output';
	return (
		{ Http: 'Console page', Session: 'Session', MvrXchange: 'MVR-xchange', OpenHaunt: 'OpenHaunt' }[
			service
		] ?? service
	);
}

/**
 * What to offer in a dropdown for a given station.
 *
 * Every interface it reported, an address entry for each alias beyond the first, and
 * nothing invented. A card with two addresses gets three rows — the name, which
 * takes whichever address it has, and one per address for the case a name cannot
 * express — because that is precisely the choice the storage rule was designed for.
 *
 * **Grouped and never filtered**, which was measured before it was decided. Hiding
 * interfaces with no address looks obviously right — this laptop has twenty-four
 * interfaces and three addresses — and is wrong, because the ones without an address
 * are `en1` to `en6`: the adapter ports, which is exactly where somebody plugs a show
 * LAN in and exactly what they want to name before it is configured. The whole
 * re-resolve-on-the-probe-tick rule exists so that a cable can be named before it is
 * ready, and a picker that hid it would put that mechanism out of reach of the UI. The
 * filter does not even sort the noise correctly the other way: a VPN's `utun` has an
 * address and is never the answer.
 *
 * So the ready ones come first and the rest stay reachable underneath.
 */
export type Choice = { value: string; label: string; detail: string };
export type ChoiceGroup = { label: string; choices: Choice[] };

export function choicesFor(network: StationNetwork | null): ChoiceGroup[] {
	const ready: Choice[] = [];
	const waiting: Choice[] = [];
	for (const each of network?.interfaces ?? []) {
		const into = each.addresses.length > 0 && each.up ? ready : waiting;
		into.push({ value: each.name, label: each.name, detail: describe(each) });
		// Only where a name is genuinely ambiguous. One address on a card is fully
		// named by the card, and offering it twice is two ways to say one thing.
		if (each.addresses.length > 1) {
			for (const address of each.addresses) {
				into.push({
					value: address,
					label: `${each.name} · ${address}`,
					detail: 'this address only'
				});
			}
		}
	}
	const groups: ChoiceGroup[] = [];
	if (ready.length) groups.push({ label: 'Ready', choices: ready });
	// Named for what it is rather than for what it lacks: a port with no address is
	// usually one nobody has configured yet, not one that is broken.
	if (waiting.length) groups.push({ label: 'Not configured yet', choices: waiting });
	return groups;
}

function describe(each: NetInterface): string {
	const parts: string[] = [];
	parts.push(each.addresses.length ? each.addresses.join(', ') : 'no IPv4 address');
	if (!each.up) parts.push('down');
	if (each.loopback) parts.push('loopback');
	return parts.join(' · ');
}

/**
 * Whether a station is worth showing a warning banner for.
 *
 * Any fault at all: there is no such thing as an unimportant one here, since every
 * one of them is a service that is not doing what somebody told it to.
 */
export function faults(network: StationNetwork | null): number {
	return (network?.bindings ?? []).filter((each) => each.fault).length;
}

/**
 * The sACN priority a station is claiming, in words.
 *
 * `null` where it holds no slot, which is a leader — at the top of the ladder rather
 * than in it. Written here rather than in the panel because the same sentence is
 * wanted beside a station row and beside an output.
 */
export function slotText(station: { is_leader: boolean; sacn_slot: number | null }): string {
	if (station.is_leader) return 'leader · 100';
	return station.sacn_slot === null || station.sacn_slot === undefined
		? 'no slot yet'
		: `follower · ${station.sacn_slot}`;
}
