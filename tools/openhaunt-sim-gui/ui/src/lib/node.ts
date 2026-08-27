/**
 * What a module has, in ports.
 *
 * Written out here from the OpenHaunt module documents rather than imported from
 * the console, for the same reason `openhaunt-sim` shares no code with it: if the
 * two ends are going to be compared, they have to have been written down twice.
 */

export type Snapshot = {
	serial: string;
	name: string;
	module: string;
	moduleName: string;
	typeId: number;
	caps: string;
	switchesMains: boolean;
	httpAddr: string;
	sacnAddr: string | null;
	advertising: boolean;
	adopted: boolean;
	broker: string | null;
	mqttConnected: boolean;
	outputs: Record<string, unknown>;
	inputs: Record<string, unknown>;
	identified: number;
	startedMs: number;
};

export type Frame = { universe: number; channels: number[] };

/** A terminal the console reads. */
export type Sensor = { port: number; label: string; unit: string; min: number; max: number };

/** How many contacts a module presents to be closed, and by which port. */
export function contacts(module: string): number[] {
	return module === 'input' ? [0, 1, 2, 3, 4, 5, 6, 7] : [];
}

/** What a module measures, and the range each reading plausibly sits in. */
export function sensors(module: string): Sensor[] {
	if (module !== 'env') return [];
	return [
		{ port: 0, label: 'Temperature', unit: '°C', min: -10, max: 50 },
		{ port: 1, label: 'Humidity', unit: '%', min: 0, max: 100 },
		{ port: 2, label: 'Air quality', unit: 'ppm', min: 0, max: 2000 }
	];
}

/** How many outputs a console can drive, and by which port. */
export function switches(module: string): number[] {
	if (module === 'relay') return [0];
	if (module === 'contact') return [0, 1, 2, 3];
	return [];
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

export function readBrightness(value: unknown): number | null {
	if (typeof value !== 'object' || value === null) return null;
	const { brightness } = value as { brightness?: unknown };
	return typeof brightness === 'number' ? brightness : null;
}

export function readText(value: unknown): string | null {
	if (typeof value !== 'object' || value === null) return null;
	const { text } = value as { text?: unknown };
	return typeof text === 'string' ? text : null;
}

function clampByte(n: number): number {
	return Math.max(0, Math.min(255, Math.round(n)));
}

/** `21.5` out of `{ value: 21.5, unit: "C", ts: … }`. */
export function readValue(value: unknown): number | null {
	if (typeof value !== 'object' || value === null) return null;
	const { value: reading } = value as { value?: unknown };
	return typeof reading === 'number' ? reading : null;
}

export function uptime(startedMs: number, now: number): string {
	const seconds = Math.max(0, Math.floor((now - startedMs) / 1000));
	const h = Math.floor(seconds / 3600);
	const m = Math.floor((seconds % 3600) / 60);
	const s = seconds % 60;
	return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m ${s}s` : `${s}s`;
}
