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
 * A tube drawn flat is a tube: the eye sees a surface. What turns it into air with
 * light in it is that the *middle* of the tube is bright and the *edge* is nothing,
 * and the term that does that has to come from the tube's own surface normal
 * against the view — not from the beam's axis, which is the same for every pixel
 * across the beam and so cannot make one side of it differ from the other. The
 * normal is worked out in the vertex shader for the cone it just made, because
 * the geometry's own normals are of a cylinder that is no longer there — and
 * worked out there rather than from screen-space derivatives in the fragment,
 * because a derivative is flat per triangle and draws the tube as strips.
 *
 * Three terms multiply:
 *
 * - **silhouette**: how squarely the surface faces the camera, raised to a power
 *   that *falls with how end-on the beam is seen*. Side-on, the power is high and
 *   the edges fade to nothing over most of the width. Down the barrel the power
 *   goes to zero and the whole disc lights up, which is the flare a lamp gives when
 *   it catches your eye. One term, both effects.
 * - **attenuation** along the throw, in metres, and steeper for a wider beam: a
 *   wash thins out where a beam carries. Not inverse-square, which goes to nothing
 *   far too fast to read on a screen.
 * - **haze**, below.
 *
 * Both faces of the tube are drawn and added, so the core is the front and back
 * walls together and the edge is neither. All additive blending in one fragment
 * shader with no post-processing chain — the prior art this technique came from
 * credits a post-processing library in its README and has none in its source, and
 * the cheaper lesson is the one worth taking. The fragment's alpha is one: with
 * additive blending the source colour is scaled by its alpha, and writing the
 * strength there as well squared everything and made every beam a ghost.
 *
 * ## Haze
 *
 * Turbulence — the absolute value of signed noise, summed over four octaves — which
 * is what gives haze its streaks and folds; smooth noise gives blobs. Sampled in
 * world space with **time as the third axis**, so it drifts rather than being a
 * texture stuck to the beam. It never darkens a beam below what the beam is worth
 * without it: the haze is *in* the light, and adds structure rather than taking
 * light away.
 *
 * ## The beam starts at the lens, not at a point
 *
 * A cone from a point reads as a pin. Light leaves a lantern across the width of
 * its lens, so the radius at the origin is the lens and widens from there.
 *
 * ## Colour is scaled in HSV, value only
 *
 * A dim beam keeps its hue. Scaling RGB towards black crushes a saturated blue
 * towards grey on the way down, which is exactly backwards from what a real dimmer
 * does and makes every fade end looking wrong.
 */

import * as THREE from 'three';

/** How many sides the cylinder has. Enough to read as round, few enough to instance. */
const RADIAL_SEGMENTS = 32;
/** Rings along the length, so the falloff and haze have somewhere to be sampled. */
const HEIGHT_SEGMENTS = 24;

/** Radius of the lens the beam leaves, in metres. The width of the beam at the lantern. */
export const LENS_RADIUS = 0.1;

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

	uniform float uLensRadius;

	varying vec3 vColor;
	varying float vLevel;
	varying float vAlong;      // 0 at the lantern, 1 at the far end
	varying float vLength;     // the throw, in metres
	varying float vSpread;
	varying vec3 vWorld;
	varying vec3 vAxis;        // the beam's direction, in world space
	varying vec3 vNormal;      // the cone's surface normal, in world space

	void main() {
		// The geometry runs 0 to -1 in y. Turn that into a fraction along the beam.
		float along = -position.y;
		vAlong = along;
		vLength = beamLength;
		vSpread = beamSpread;

		// The cone, made here rather than in a geometry. The radius starts at the
		// lens and grows by tan(angle) per metre of throw, which is the entire trick:
		// changing a zoom changes an attribute, and no buffer is rebuilt.
		float radius = uLensRadius + along * beamSpread * beamLength;
		vec3 shaped = vec3(position.x * radius, position.y * beamLength, position.z * radius);

		vec4 world = instanceMatrix * vec4(shaped, 1.0);
		vWorld = world.xyz;

		// The cone's own normal, worked out rather than read off the geometry — the
		// geometry is a cylinder, and this vertex is no longer on it. A cone whose
		// radius grows by beamSpread per metre has a normal leaning back along the
		// axis by the same amount. Interpolated across the face, so the surface
		// reads as round rather than as the thirty-two flat strips it is made of.
		vec3 radial = normalize(vec3(position.x, 0.0, position.z));
		vNormal = normalize((instanceMatrix * vec4(radial.x, beamSpread, radial.z, 0.0)).xyz);

		// The beam's own axis in world space: local -Y through the instance rotation.
		vAxis = normalize((instanceMatrix * vec4(0.0, -1.0, 0.0, 0.0)).xyz);

		vColor = beamColor;
		vLevel = beamLevel;
		gl_Position = projectionMatrix * modelViewMatrix * world;
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
	varying float vLength;
	varying float vSpread;
	varying vec3 vWorld;
	varying vec3 vAxis;
	varying vec3 vNormal;

	// ── Turbulence, four octaves of value noise ──────────────────────────────
	// Sampled in world space with time as the third axis, so the haze drifts through
	// the room rather than travelling with the beam. Turbulence rather than plain
	// noise: the absolute value of a signed field folds it into streaks, which is
	// what haze in a beam looks like, where smooth noise is blobs.
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

	float turbulence(vec3 p) {
		float total = 0.0;
		float amplitude = 1.0;
		for (int octave = 0; octave < 4; octave++) {
			total += abs(noise(p) * 2.0 - 1.0) * amplitude;
			p *= 2.03;
			amplitude *= 0.5;
		}
		return total;
	}

	void main() {
		if (vLevel <= 0.0005) discard;

		vec3 normal = normalize(vNormal);
		vec3 toEye = normalize(cameraPosition - vWorld);
		vec3 axis = normalize(vAxis);

		// How side-on the beam is seen: 1 across it, 0 straight down it.
		float sideOn = 1.0 - abs(dot(axis, toEye));

		// 1. Silhouette. The middle of the tube faces the camera; the edge is
		//    perpendicular to it and goes to nothing. The power falls with sideOn so
		//    that looking down the barrel lights the whole disc: that is the flare.
		float facing = abs(dot(normal, toEye));
		float silhouette = pow(facing, 4.0 * sideOn);

		// 2. Attenuation along the throw, in metres. Steeper for a wider beam, so a
		//    wash thins where a beam carries — the spread is tan of the half-angle.
		float metres = vAlong * vLength;
		float attenuation = 1.3 / (1.0 + 0.5 * metres + 0.9 * vSpread * metres * metres);

		float intensity = silhouette * attenuation;

		// 3. The haze the beam is passing through. Never below the beam's own
		//    intensity: haze is in the light and adds folds to it rather than taking
		//    light away. Density is how much of that structure shows; turbulence is
		//    how fast the field moves through the room.
		vec3 samplePoint = vWorld * 0.45 + vec3(0.0, 0.0, uTime * uHazeTurbulence * 0.3);
		float folds = max(turbulence(samplePoint) * 0.9, intensity);
		float haze = mix(1.0, folds, clamp(uHazeDensity, 0.0, 1.0));

		float strength = vLevel * intensity * haze;

		// Fade out over the last stretch above the deck rather than clipping through
		// it. A beam that ends in a hard disc where it meets the floor reads as a
		// modelling error, which is what it is.
		strength *= smoothstep(0.0, 0.45, vWorld.y - uFloorY);

		if (strength <= 0.001) discard;

		// Alpha is one: additive blending scales the colour by it, and putting the
		// strength there as well squares every beam into a ghost.
		gl_FragColor = vec4(vColor * strength, 1.0);
	}
`;

/** The uniforms a beam material carries, so a caller can drive them per frame. */
export type BeamUniforms = {
	uTime: { value: number };
	uHazeDensity: { value: number };
	uHazeTurbulence: { value: number };
	uFloorY: { value: number };
	uLensRadius: { value: number };
};

/**
 * The material every beam shares.
 *
 * Additive and depth-write-off, so beams cross one another without either winning,
 * and `DoubleSide` because both walls of the tube are part of the picture — the
 * core is the front and the back added together — and because the camera goes
 * inside a beam whenever somebody flies the view through the rig.
 */
export function beamMaterial(): THREE.ShaderMaterial & { uniforms: BeamUniforms } {
	return new THREE.ShaderMaterial({
		vertexShader: VERTEX,
		fragmentShader: FRAGMENT,
		uniforms: {
			uTime: { value: 0 },
			uHazeDensity: { value: 0.35 },
			uHazeTurbulence: { value: 0.25 },
			uFloorY: { value: 0 },
			uLensRadius: { value: LENS_RADIUS }
		},
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		side: THREE.DoubleSide,
		toneMapped: false
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
