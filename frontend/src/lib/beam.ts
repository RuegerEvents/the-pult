/**
 * A beam that reads as light, rather than a cone that says where a light points.
 *
 * One instanced open-ended cylinder for the whole rig, and every beam is a set of
 * per-instance attributes on it. **The beam is not geometry**: the cone shape is
 * vertex displacement in the shader, so a zoom costs one float in a buffer and
 * nothing is rebuilt. That is the whole reason this replaced `ConeGeometry` — the old
 * drawing had reactive `args`, so dragging a beam spot allocated a fresh cone per
 * fixture per frame, and since the viewer began evaluating every animation frame, a
 * plain fade did it too.
 *
 * ## What makes it look like light
 *
 * Four terms multiplied, none of which is optional:
 *
 * - **side-on**: how perpendicular the view is to the beam's axis. A beam seen from
 *   the side is a bright streak; seen end-on it is nearly nothing.
 * - **down the barrel**: how close the camera is to looking into the lens, which is
 *   what makes a light flare when you catch its eye.
 * - **falloff**: inverse-square-ish along the length, so the far end is dimmer.
 * - **silhouette**: a power term on the edge, which is what stops a cylinder reading
 *   as a tube. Without this one it looks like a drinking straw.
 *
 * All additive blending in one fragment shader. There is no `EffectComposer` and no
 * post-processing chain — the prior art this technique came from credits a
 * post-processing library in its README and has none in its source, and the cheaper
 * lesson is the one worth taking.
 *
 * ## Haze
 *
 * Sampled in world space with **time as the third axis**, so it drifts rather than
 * being a texture stuck to the beam. Value noise rather than simplex: three octaves
 * of it is a dozen lines that can be read and checked, where simplex is fifty that
 * mostly get copied, and for modulating haze density the difference is not visible.
 *
 * ## Colour is scaled in HSV, value only
 *
 * A dim beam keeps its hue. Scaling RGB towards black crushes a saturated blue
 * towards grey on the way down, which is exactly backwards from what a real dimmer
 * does and makes every fade end looking wrong.
 */

import * as THREE from 'three';

/** How many sides the cylinder has. Enough to read as round, few enough to instance. */
const RADIAL_SEGMENTS = 20;
/** Rings along the length, so the falloff and haze have somewhere to be sampled. */
const HEIGHT_SEGMENTS = 24;

/**
 * The one geometry every beam shares.
 *
 * A unit cylinder, open at both ends, which the vertex shader turns into whatever
 * cone each instance needs. Local space runs down **−Y** from the origin, because the
 * origin is the lantern: light leaves a fixture at a point and widens on the way to
 * the floor, and building it the other way up narrows the beam towards the deck.
 */
export function beamGeometry(): THREE.CylinderGeometry {
	const geometry = new THREE.CylinderGeometry(
		1,
		1,
		1,
		RADIAL_SEGMENTS,
		HEIGHT_SEGMENTS,
		true // open ended: a beam has no cap
	);
	// Move it so y runs 0 (at the lantern) to -1 (at the far end), which is what the
	// vertex shader below assumes.
	geometry.translate(0, -0.5, 0);
	return geometry;
}

const VERTEX = /* glsl */ `
	// Per-instance. instanceMatrix is supplied by three.js and carries the
	// fixture's position and the rotation that aims the beam.
	attribute vec3 beamColor;
	attribute float beamLevel;
	attribute float beamLength;
	// tan of the half-angle. The whole of what a zoom costs.
	attribute float beamSpread;

	varying vec3 vColor;
	varying float vLevel;
	varying float vAlong;      // 0 at the lantern, 1 at the far end
	varying vec3 vWorld;
	varying vec3 vAxis;        // the beam's direction, in world space
	varying vec3 vToEye;

	void main() {
		// The geometry runs 0 to -1 in y. Turn that into a fraction along the beam.
		float along = -position.y;
		vAlong = along;

		// The cone, made here rather than in a geometry. The far ring is scaled by
		// tan(angle) times the throw, which is the entire trick: changing a zoom
		// changes an attribute, and no buffer is rebuilt.
		vec3 shaped = vec3(
			position.x * along * beamSpread * beamLength,
			position.y * beamLength,
			position.z * along * beamSpread * beamLength
		);

		vec4 world = instanceMatrix * vec4(shaped, 1.0);
		vWorld = world.xyz;

		// The beam's own axis in world space: local -Y through the instance rotation.
		vAxis = normalize((instanceMatrix * vec4(0.0, -1.0, 0.0, 0.0)).xyz);

		vec4 viewPosition = modelViewMatrix * world;
		vToEye = normalize(cameraPosition - world.xyz);

		vColor = beamColor;
		vLevel = beamLevel;
		gl_Position = projectionMatrix * viewPosition;
	}
`;

const FRAGMENT = /* glsl */ `
	precision highp float;

	uniform float uTime;
	uniform float uHazeDensity;
	uniform float uHazeTurbulence;
	uniform float uFloorY;

	varying vec3 vColor;
	varying float vLevel;
	varying float vAlong;
	varying vec3 vWorld;
	varying vec3 vAxis;
	varying vec3 vToEye;

	// ── Value noise, three octaves ────────────────────────────────────────────
	// Sampled in world space with time as the third axis, so the haze drifts through
	// the room rather than travelling with the beam.
	float hash(vec3 p) {
		p = fract(p * 0.3183099 + vec3(0.1, 0.2, 0.3));
		p *= 17.0;
		return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
	}

	float noise(vec3 x) {
		vec3 i = floor(x);
		vec3 f = fract(x);
		f = f * f * (3.0 - 2.0 * f);
		return mix(
			mix(mix(hash(i + vec3(0,0,0)), hash(i + vec3(1,0,0)), f.x),
			    mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),
			mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x),
			    mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y),
			f.z);
	}

	float fbm(vec3 p) {
		float total = 0.0;
		float amplitude = 0.5;
		for (int octave = 0; octave < 3; octave++) {
			total += noise(p) * amplitude;
			p *= 2.03;
			amplitude *= 0.5;
		}
		return total;
	}

	void main() {
		if (vLevel <= 0.0005) discard;

		// 1. Side-on. A beam seen across its axis is bright; seen end-on it is not.
		float sideOn = 1.0 - abs(dot(normalize(vAxis), normalize(vToEye)));

		// 2. Down the barrel: the opposite term, and the one that makes a lamp flare
		//    when the camera catches its eye. Deliberately narrow.
		float barrel = pow(max(0.0, dot(normalize(vAxis), -normalize(vToEye))), 24.0);

		// 3. Falloff along the throw. Not true inverse-square — that goes to nothing
		//    far too fast to read on a screen — but the same shape.
		float falloff = 1.0 / (1.0 + 2.2 * vAlong * vAlong);

		// 4. Silhouette. Without the power term a cylinder reads as a tube: the edges
		//    are as bright as the middle and the eye sees a surface rather than air.
		float silhouette = pow(sideOn, 2.5);

		// The haze the beam is passing through. Density scales the whole thing;
		// turbulence is how fast the field moves through the room.
		vec3 samplePoint = vWorld * 0.6 + vec3(0.0, 0.0, uTime * uHazeTurbulence * 0.35);
		float haze = mix(0.75, fbm(samplePoint) * 1.6, clamp(uHazeDensity, 0.0, 1.0));

		float strength = vLevel * silhouette * falloff * haze + barrel * vLevel * 0.5;

		// Fade out over the last stretch above the deck rather than clipping through
		// it. A beam that ends in a hard disc where it meets the floor reads as a
		// modelling error, which is what it is.
		float aboveFloor = smoothstep(0.0, 0.35, vWorld.y - uFloorY);
		strength *= mix(0.25, 1.0, aboveFloor);

		if (strength <= 0.001) discard;

		gl_FragColor = vec4(vColor * strength, strength);
	}
`;

/** The uniforms a beam material carries, so a caller can drive them per frame. */
export type BeamUniforms = {
	uTime: { value: number };
	uHazeDensity: { value: number };
	uHazeTurbulence: { value: number };
	uFloorY: { value: number };
};

/**
 * The material every beam shares.
 *
 * Additive and depth-write-off, so beams cross one another without either winning,
 * and `DoubleSide` because the camera goes inside a beam whenever somebody flies the
 * view through the rig.
 */
export function beamMaterial(): THREE.ShaderMaterial & { uniforms: BeamUniforms } {
	return new THREE.ShaderMaterial({
		vertexShader: VERTEX,
		fragmentShader: FRAGMENT,
		uniforms: {
			uTime: { value: 0 },
			uHazeDensity: { value: 0.35 },
			uHazeTurbulence: { value: 0.25 },
			uFloorY: { value: 0 }
		},
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		side: THREE.DoubleSide
	}) as THREE.ShaderMaterial & { uniforms: BeamUniforms };
}

/**
 * Scale a colour's *value* only, keeping its hue and saturation.
 *
 * The reason this is not `colour.multiplyScalar(level)`: scaling RGB drags a
 * saturated colour towards grey as it dims, so a deep blue fading out goes pale
 * before it goes dark. A real dimmer does not do that, and neither does this.
 */
export function dimKeepingHue(colour: THREE.Color, level: number, into: THREE.Color): THREE.Color {
	const hsl = { h: 0, s: 0, l: 0 };
	colour.getHSL(hsl);
	into.setHSL(hsl.h, hsl.s, hsl.l * Math.max(0, Math.min(1, level)));
	return into;
}

/**
 * What a strobing parameter is showing at this instant.
 *
 * A square wave against the animation clock, and it lives **here rather than in
 * `pult-render`** on purpose. A strobe channel carries a *rate*: the console sends
 * the byte and the fixture does the flashing, so there is nothing for the evaluator
 * to work out and no corpus case to keep two compilations agreeing about. What the
 * visualiser has to do is synthesise the flash so an operator can see what they set,
 * which is a fact about the picture and not about the rig.
 *
 * `rate` is the parameter's own 0-1. Zero means not strobing, and the beam is simply
 * on — never off, because a fixture at zero strobe is a fixture with an open shutter.
 */
export function strobeGate(rate: number, seconds: number): number {
	if (!(rate > 0.002)) return 1;
	// 1 to 25 Hz, which is about the range a real shutter covers before it stops
	// reading as separate flashes.
	const hz = 1 + rate * 24;
	// Duty cycle under a half, because a strobe is a series of stabs rather than a
	// square-on square-off flicker, and an even split reads as a flicker.
	return (seconds * hz) % 1 < 0.38 ? 1 : 0;
}
