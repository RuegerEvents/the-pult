import { getContext, setContext } from 'svelte';
import type { PultWsClient } from './client.js';
import type { DataRoot } from './data.js';

const CLIENT_KEY = 'pult:client';
const DATA_KEY = 'pult:data';

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
