/**
 * A new random id, on any page.
 *
 * `crypto.randomUUID` exists only in a **secure context**, and a console reached as
 * `http://192.168.1.20:7700` is not one — `localhost` is, which is why every desk on the
 * machine running the station worked and every phone and tablet on the network did not.
 * A surface opened on a phone connected, said so, and showed no surface at all, because
 * the first id its client minted threw. `getRandomValues` has no such restriction, so a
 * version 4 id is built from it where the short way is missing.
 *
 * Every id the page makes comes from here; `uuid.test.ts` fails on a direct call.
 */
export function uuid(): string {
	if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
	return fromRandomBytes(crypto.getRandomValues(new Uint8Array(16)));
}

/** RFC 9562 version 4: the version nibble and the variant bits set over random bytes. */
export function fromRandomBytes(bytes: Uint8Array): string {
	const b = Uint8Array.from(bytes);
	b[6] = (b[6] & 0x0f) | 0x40;
	b[8] = (b[8] & 0x3f) | 0x80;
	const hex = Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
	return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/**
 * Put text on the clipboard, on any page.
 *
 * `navigator.clipboard` is secure-context only as well, so off `localhost` it is
 * `undefined` and *Copy link* did nothing. The old `execCommand` path still works there.
 */
export async function copyText(text: string): Promise<void> {
	if (navigator.clipboard?.writeText) return navigator.clipboard.writeText(text);
	const area = document.createElement('textarea');
	area.value = text;
	area.style.position = 'fixed';
	area.style.opacity = '0';
	document.body.appendChild(area);
	area.select();
	const copied = document.execCommand('copy');
	area.remove();
	if (!copied) throw new Error('This browser would not copy that.');
}
