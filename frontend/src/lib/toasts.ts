import { writable } from 'svelte/store';

export type ToastLevel = 'error' | 'warning' | 'info';

export type Toast = {
	id: string;
	message: string;
	level: ToastLevel;
};

export const toasts = writable<Toast[]>([]);

const DURATION_MS = 5000;

export function addToast(message: string, level: ToastLevel = 'error'): void {
	const id = crypto.randomUUID();
	toasts.update((t) => [...t, { id, message, level }]);
	setTimeout(() => dismissToast(id), DURATION_MS);
}

export function dismissToast(id: string): void {
	toasts.update((t) => t.filter((x) => x.id !== id));
}
