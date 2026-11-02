<script lang="ts">
	/**
	 * A link a phone can take a picture of.
	 *
	 * Drawn here, as one SVG path, by `qrcode-generator` — MIT, no dependencies, and
	 * nothing sent anywhere: a venue's show network has no internet, and a QR service
	 * would be a request a desk at the venue could not make.
	 */

	import qrcode from 'qrcode-generator';

	let { text, size = 180 }: { text: string; size?: number } = $props();

	const drawn = $derived.by(() => {
		const code = qrcode(0, 'M');
		code.addData(text);
		code.make();
		const count = code.getModuleCount();
		let path = '';
		for (let row = 0; row < count; row++) {
			for (let col = 0; col < count; col++) {
				if (code.isDark(row, col)) path += `M${col + 2},${row + 2}h1v1h-1z`;
			}
		}
		// Two modules of quiet zone each side, which a camera needs to find the code.
		return { path, span: count + 4 };
	});
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 {drawn.span} {drawn.span}"
	shape-rendering="crispEdges"
	role="img"
	aria-label="QR code for {text}"
>
	<rect width={drawn.span} height={drawn.span} fill="#fff" />
	<path d={drawn.path} fill="#000" />
</svg>
