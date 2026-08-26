/**
 * Getting a drawing into the console.
 *
 * A designer is handed a PDF, so a PDF has to work — but a PDF in the render path
 * would mean shipping a document engine into both stage views. Page one is
 * rasterised here instead, and everything downstream only ever sees an image.
 */

/** What the console will store, and what a PDF becomes on the way in. */
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp'];

export type UploadedPlan = {
	sha256: string;
	width_px: number;
	height_px: number;
};

/** Render page one of a PDF to a PNG at roughly 150 dpi. */
async function rasterisePdf(file: File): Promise<Blob> {
	// Loaded on demand: a document engine has no business in the main bundle for
	// the sake of a file most shows will upload once.
	const pdfjs = await import('pdfjs-dist');
	pdfjs.GlobalWorkerOptions.workerSrc = (
		await import('pdfjs-dist/build/pdf.worker.mjs?url')
	).default;

	const doc = await pdfjs.getDocument({ data: await file.arrayBuffer() }).promise;
	const page = await doc.getPage(1);
	// 150 dpi against the PDF's own 72, capped so a huge sheet does not become a
	// texture no browser will accept.
	const natural = page.getViewport({ scale: 1 });
	const scale = Math.min(150 / 72, 8000 / Math.max(natural.width, natural.height));
	const viewport = page.getViewport({ scale });

	const canvas = document.createElement('canvas');
	canvas.width = Math.ceil(viewport.width);
	canvas.height = Math.ceil(viewport.height);
	const context = canvas.getContext('2d');
	if (!context) throw new Error('this browser would not give us a canvas to draw on');
	// A plan is usually black on nothing, and nothing renders as transparent.
	context.fillStyle = '#ffffff';
	context.fillRect(0, 0, canvas.width, canvas.height);
	await page.render({ canvas, canvasContext: context, viewport }).promise;

	return new Promise((resolve, reject) =>
		canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error('could not encode the page'))), 'image/png')
	);
}

/** How big an image is, which is what turns pixels into metres later. */
function measure(blob: Blob): Promise<{ width: number; height: number }> {
	return new Promise((resolve, reject) => {
		const url = URL.createObjectURL(blob);
		const image = new Image();
		image.onload = () => {
			URL.revokeObjectURL(url);
			resolve({ width: image.naturalWidth, height: image.naturalHeight });
		};
		image.onerror = () => {
			URL.revokeObjectURL(url);
			reject(new Error('that file is not an image this console can read'));
		};
		image.src = url;
	});
}

/**
 * Put a plan in the asset store and say what it is.
 *
 * The console addresses assets by their contents, so uploading the same drawing
 * twice costs one round trip and no storage.
 */
export async function uploadPlan(file: File, endpoint: string): Promise<UploadedPlan> {
	const isPdf = file.type === 'application/pdf' || file.name.toLowerCase().endsWith('.pdf');
	const blob = isPdf ? await rasterisePdf(file) : file;
	const mime = isPdf ? 'image/png' : file.type;

	if (!IMAGE_TYPES.includes(mime)) {
		throw new Error(`${mime || 'that file'} is not a plan this console can read`);
	}

	const { width, height } = await measure(blob);

	const response = await fetch(endpoint, {
		method: 'POST',
		headers: { 'content-type': mime },
		body: blob
	});
	if (!response.ok) throw new Error(await response.text());

	const { sha256 } = await response.json();
	return { sha256, width_px: width, height_px: height };
}

/**
 * A scale that makes a new plan a sensible size before anyone calibrates it.
 *
 * Guessing a 12 m wide stage is wrong for every specific room and right about the
 * order of magnitude, which is the difference between a plan you can see and one
 * that is a speck or fills the county.
 */
export const guessScale = (widthPx: number) => 12 / Math.max(widthPx, 1);
