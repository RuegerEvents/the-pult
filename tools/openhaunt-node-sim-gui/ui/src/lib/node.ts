/**
 * A window onto a node, drawn from what the node says about itself.
 *
 * The panel used to write the module documents out a third time — which contacts
 * a 0x0002 has, what a 0x0007 measures. It no longer does: only the device knows
 * what it is, and the simulated node describes its ports exactly as it serves them
 * from `GET /api/v1/info`. Everything below reads that description, or reads a
 * payload off the wire defensively.
 */

/** One terminal, in the words `/api/v1/info` uses. */
export type PortDescription = {
	port: number;
	name: string;
	access: 'readonly' | 'readwrite';
	dataType: 'boolean' | 'number' | 'string' | 'color';
	unit?: string;
	minimum?: number;
	maximum?: number;
	default?: number;
	class?: string;
	/** What this port can trace for itself. Absent means the console sends every value. */
	effects?: PortEffects;
};

/**
 * What one port has told the console it can do without being sent every value.
 *
 * Shapes are named rather than flagged because a relay that can chop a square wave
 * has no way to trace a sine, and there is no point letting a console find that out
 * by trying.
 */
export type PortEffects = {
	shapes: string[];
	steps: boolean;
	transitions: boolean;
};

/** Every shape the protocol names, for the config editor's checkboxes. */
export const SHAPES = ['sine', 'triangle', 'square', 'saw-up', 'saw-down'];

/** The universe a gateway forwards, present only on a node that forwards one. */
export type DmxDescription = { protocols: string[]; universes: number };

/** The module descriptor, as the TXT record and `/api/v1/info` report it. */
export type ModuleDescriptor = {
	/** Written `0x0003`. An inventory key and a mains warning, not a lookup. */
	type: string;
	name: string;
	rev: string;
	flags: number;
	caps: string;
};

/**
 * Everything a node is, in one value that round-trips through a file.
 *
 * The same shape the config editor writes and the running node answers from, so
 * what is on screen is what is on the wire.
 */
export type NodeConfig = {
	name: string;
	serial: string;
	module: ModuleDescriptor;
	ports: PortDescription[];
	dmx?: DmxDescription | null;
	httpPort: number;
	advertise: boolean;
	autoMs?: number | null;
};

export type Snapshot = {
	config: NodeConfig;
	httpAddr: string;
	sacnAddr: string | null;
	adopted: boolean;
	broker: string | null;
	mqttConnected: boolean;
	outputs: Record<string, unknown>;
	/**
	 * What each port is tracing on its own, keyed by port.
	 *
	 * `outputs` is still the truth about where a port is; this says why it is
	 * moving, which is the difference between a node being driven and a node
	 * running something.
	 */
	effects: Record<string, { summary?: string }>;
	inputs: Record<string, unknown>;
	identified: number;
	startedMs: number;
};

/** One of the configs shipped with the simulator. */
export type Demo = { name: string; config: NodeConfig };

/** Descriptor bit 6: this module switches mains, and a console should say so. */
export const MAINS_FLAG = 1 << 6;

export const ACCESSES = ['readonly', 'readwrite'] as const;
export const DATA_TYPES = ['boolean', 'number', 'string', 'color'] as const;

/**
 * The `class` vocabulary a controller may recognise. Not a closed set — a node is
 * free to declare a class nobody has heard of, and a controller that does not know
 * the word treats the port as a parameter named after `name`.
 */
export const CLASSES = [
	'contact',
	'switch',
	'temperature',
	'humidity',
	'air-quality',
	'intensity',
	'color',
	'text'
] as const;

/** UDR unit names, as the ones written down so far. Free text beside them. */
export const UNITS = [
	'unitless',
	'percent',
	'degree-celsius',
	'parts-per-million',
	'metre-per-second',
	'millimetre'
] as const;

export type Frame = { universe: number; channels: number[] };

/** The ports a console reads off this node. */
export function inputPorts(node: Snapshot): PortDescription[] {
	return node.config.ports.filter((p) => p.access === 'readonly');
}

/** The ports a console drives on this node. */
export function outputPorts(node: Snapshot): PortDescription[] {
	return node.config.ports.filter((p) => p.access === 'readwrite');
}

/** A port to add to a config: the least surprising thing, on the next free number. */
export function aNewPort(ports: PortDescription[]): PortDescription {
	const next = ports.reduce((highest, p) => Math.max(highest, p.port + 1), 0);
	return { port: next, name: `Port ${next}`, access: 'readwrite', dataType: 'boolean' };
}

/**
 * Whatever is wrong with a config, in words.
 *
 * The same checks the node makes before it will run one, done here so the editor
 * can say so while it is being typed rather than only on Apply.
 */
export function problems(config: NodeConfig): string[] {
	const found: string[] = [];
	if (!config.serial.trim()) {
		found.push('a node needs a serial: it is what its topics are keyed by');
	}
	const seen = new Set<number>();
	for (const port of config.ports) {
		if (seen.has(port.port)) found.push(`two ports are numbered ${port.port}`);
		seen.add(port.port);
	}
	return found;
}

/** `{ state: true }` and the like, read defensively: this arrives off the wire. */
export function readState(value: unknown): boolean {
	return typeof value === 'object' && value !== null && (value as { state?: unknown }).state === true;
}

export function readColor(value: unknown): string | null {
	if (typeof value !== 'object' || value === null) return null;
	const { r, g, b } = value as { r?: unknown; g?: unknown; b?: unknown };
	if (typeof r !== 'number' || typeof g !== 'number' || typeof b !== 'number') return null;
	return `rgb(${clampByte(r)} ${clampByte(g)} ${clampByte(b)})`;
}

export function readText(value: unknown): string | null {
	if (typeof value !== 'object' || value === null) return null;
	const { text } = value as { text?: unknown };
	return typeof text === 'string' ? text : null;
}

function clampByte(n: number): number {
	return Math.max(0, Math.min(255, Math.round(n)));
}

/**
 * `21.5` out of `{ value: 21.5, unit: "degree-celsius", ts: … }`.
 *
 * The same key on the way out as on the way in: a number port carries `value`
 * whichever direction it flows, because the shape follows the data type.
 */
export function readNumber(value: unknown): number | null {
	if (typeof value !== 'object' || value === null) return null;
	const { value: reading } = value as { value?: unknown };
	return typeof reading === 'number' ? reading : null;
}

/** A unit as short as it should look beside a number. */
export function unitLabel(unit: string | undefined): string {
	switch (unit) {
		case undefined:
		case 'unitless':
			return '';
		case 'degree-celsius':
			return '°C';
		case 'percent':
			return '%';
		case 'parts-per-million':
			return 'ppm';
		case 'metre-per-second':
			return 'm/s';
		case 'millimetre':
			return 'mm';
		default:
			// A unit nobody has abbreviated. Shown as the node spelled it, which is
			// the only honest thing to do with a word this panel does not know.
			return ` ${unit}`;
	}
}

export function uptime(startedMs: number, now: number): string {
	const seconds = Math.max(0, Math.floor((now - startedMs) / 1000));
	const h = Math.floor(seconds / 3600);
	const m = Math.floor((seconds % 3600) / 60);
	const s = seconds % 60;
	return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m ${s}s` : `${s}s`;
}
