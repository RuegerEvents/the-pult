import { writable } from 'svelte/store';
import { uuid } from '$lib/uuid.js';

/**
 * How loud a toast is.
 *
 * `success` is rare on purpose — a console that congratulated itself on every write
 * would be one nobody read. It exists for Save, where nothing else on screen changes
 * and there is otherwise no way to tell that anything happened.
 */
export type ToastLevel = 'error' | 'warning' | 'info' | 'success';

export type Toast = {
	id: string;
	message: string;
	level: ToastLevel;
};

export const toasts = writable<Toast[]>([]);

const DURATION_MS = 5000;

export function addToast(message: string, level: ToastLevel = 'error'): void {
	const id = uuid();
	toasts.update((t) => [...t, { id, message, level }]);
	setTimeout(() => dismissToast(id), DURATION_MS);
}

export function dismissToast(id: string): void {
	toasts.update((t) => t.filter((x) => x.id !== id));
}
