import { getContext, setContext } from 'svelte';
import type { PultWsClient } from './client.js';
import type { DataRoot } from './data.js';
import type { StationStore } from '$lib/stores/station.js';

const CLIENT_KEY = 'pult:client';
const DATA_KEY = 'pult:data';
const STATION_KEY = 'pult:station';

export function setClientContext(client: PultWsClient): void {
	setContext(CLIENT_KEY, client);
}

export function getClientContext(): PultWsClient {
	return getContext(CLIENT_KEY);
}

export function setDataContext(data: DataRoot): void {
	setContext(DATA_KEY, data);
}

export function getDataContext(): DataRoot {
	return getContext(DATA_KEY);
}

/**
 * What the station says it is, and — the part that matters — which show it has open.
 *
 * In context rather than a module-level store because it is *this page's* station:
 * the origin it loaded from decides which one, and a component asking a singleton
 * would be a component that could not be told.
 */
export function setStationContext(station: StationStore): void {
	setContext(STATION_KEY, station);
}

export function getStationContext(): StationStore {
	return getContext(STATION_KEY);
}
