<script lang="ts">
	/**
	 * The rig in three dimensions, drawn imperatively.
	 *
	 * ## Why there is no Threlte here any more
	 *
	 * The declarative layer cost more than it paid for in this one component. Two live
	 * defects came straight out of it: `<T.ConeGeometry args={...}>` had reactive
	 * `args`, so a geometry was rebuilt per fixture per frame whenever a throw changed
	 * — which, once the viewer began evaluating every animation frame, meant every
	 * fade; and a `<T.SpotLight>` mounted inside `{#if level > 0.01}` changed the
	 * scene's light count as a fade crossed 1%, which changes three.js's program cache
	 * key and recompiles every material in the scene, on the most ordinary thing a
	 * console does.
	 *
	 * Both are gone by construction rather than by fix. There are no reactive `args`
	 * because there is no declarative graph, and the light is mounted once for the
	 * life of the panel and driven to zero.
	 *
	 * The hard part of going imperative was already done: gizmo picking and dragging
	 * were hand-written raycasting against the DOM element before this rewrite. What
	 * Threlte actually supplied was a canvas, a render loop, hover events and an HTML
	 * overlay, and those are the four things replaced below.
	 *
	 * ## Everything here is per panel
	 *
	 * No module-level renderer, scene or controls. The workspace is a tree of tiles
	 * and two `rig` panels can be open at once; shared mutable state would have them
	 * fighting over one camera.
	 *
	 * ## What is instanced and what is not
	 *
	 * The **beams** are one `InstancedMesh` over a shared cylinder, because the beam
	 * shader wants per-instance attributes anyway. The **bodies** are individual
	 * meshes, reused across frames rather than rebuilt — whether they should be
	 * instanced too is a question about 5000 `Quaternion`s that a measurement should
	 * answer rather than taste. The **gizmos** are drawn only on what is selected, so
	 * they are never the large case.
	 */

	import { untrack } from 'svelte';
	import * as THREE from 'three';
	import CameraControls from 'camera-controls';

	import type { Fixture, FixtureType, Show, StagePlan, Vec3 } from '$lib/generated/index.js';
	import {
		aimAt,
		beamDirection,
		drawnLength,
		fixtureOutput,
		fixturePoint,
		fohCamera,
		planExtent,
		throwDistance,
		wrapDegrees,
		travelOf
	} from '$lib/stage.js';
	import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js';
	import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js';
	import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
	import { OutputPass } from 'three/examples/jsm/postprocessing/OutputPass.js';

	import { beamGeometry, beamMaterial, coneMaterial, dimKeepingHue, strobeGate, LENS_RADIUS } from '$lib/beam.js';
	import {
		bearingFromPoint,
		bearingOnFloor,
		elevationFromPoint,
		rayOnPlane,
		type Ray
	} from '$lib/puppeteer.js';
	import { selected, select, toggle } from '$lib/stores/selection.js';
	import {
		fixtureIsVisible,
		hiddenLayers,
		namedAssets,
		objectsById,
		symbols,
		visibleObjects
	} from '$lib/stores/scene.js';
	import { instance, load } from '$lib/geometry.js';
	import { stockMesh } from '$lib/stock.js';
	import { worldTransform } from '$lib/scene.js';
	import { setValue } from '$lib/stores/programmer.js';
	import { output as showing, watching } from '$lib/stores/output.js';
	import { parameterKey } from '$lib/patch.js';
	import Quicksheet from '$lib/components/programmer/Quicksheet.svelte';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { DEFAULT_VIEW, view, type RenderMode } from '$lib/stores/view.js';

	// `camera-controls` is a library rather than a wrapper: it wants the three.js
	// pieces it uses handed to it once per process.
	CameraControls.install({ THREE });

	let {
		fixtures,
		types,
		plan,
		planUrl,
		show,
		follow = false
	}: {
		fixtures: Fixture[];
		types: FixtureType[];
		plan: StagePlan | null;
		planUrl: string | null;
		show: Show | null;
		follow?: boolean;
	} = $props();

	/** The element three.js draws into. */
	let host = $state<HTMLDivElement | null>(null);
	/** Where the quicksheet has been projected to this frame, in panel pixels. */
	let sheetAt = $state<{ x: number; y: number; visible: boolean }>({
		x: 0,
		y: 0,
		visible: false
	});

	// ── What a frame of this view costs ─────────────────────────────────────────
	//
	// The station publishes what its *output* frames cost and that is a different
	// number: nothing about drawing a rig reaches a lamp. Two figures here, because
	// they answer different questions. `cpuMs` is the work this panel does to draw
	// a frame — the loop below and the call into three.js. `gpuMs` is how long the
	// GPU took over one, read back through `EXT_disjoint_timer_query_webgl2` where
	// the browser offers it, and it is the figure that says why a view reporting
	// nine milliseconds can still feel laggy: a rAF loop never waits for the GPU,
	// so the picture can be several frames behind the pointer while every CPU
	// figure looks fine. Rolling means over a second, so they read as numbers.
	//
	// And "idle" is an answer. This view draws only when something changed, so a
	// panel showing a settled rig has no frame to cost, and says so rather than
	// reporting a rate for frames it did not draw.
	let cpuMs = $state(0);
	let gpuMs = $state<number | null>(null);
	let drawing = $state(false);
	export function cost(): { cpuMs: number; gpuMs: number | null; drawing: boolean } {
		return { cpuMs, gpuMs, drawing };
	}

	/** Set by the render loop, so an effect can ask for a frame. */
	let markDirty: () => void = () => {};

	/**
	 * At most this often. A ProMotion display offers a hundred and twenty frames a
	 * second and a lighting visualiser drawn at that rate is a GPU pinned for a
	 * smoothness nobody can see in a two-second fade. Fifteen rather than sixteen
	 * and two thirds so a sixty-hertz display, whose frames arrive a hair early or
	 * late, never skips one.
	 */
	const DRAW_EVERY_MS = 15;

	/**
	 * The room's own light with the work light all the way up: enough to read every
	 * truss and the plan as if the house lights were on. The slider is a fraction of
	 * this, and 40% of it is the view as it was first drawn.
	 */
	const AMBIENT_AT_FULL = 1.25;
	const HEMISPHERE_AT_FULL = 0.75;

	const placed = $derived(
		fixtures.filter(
			(f) => fixturePoint(f, $objectsById) !== null && fixtureIsVisible(f, $hiddenLayers)
		)
	);
	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);

	// The opening view: worked out once, when there is first a rig to frame, and then
	// left alone. Recomputing it as fixtures move would take the camera out of the
	// operator's hands every time a cue changed something.
	let home = $state(fohCamera([]));
	let framed = false;
	$effect(() => {
		if (framed || placed.length === 0) return;
		framed = true;
		home = untrack(() => fohCamera(placed));
	});

	const floor = $derived(plan ? planExtent(plan) : { width: 20, depth: 14 });

	/// Everything in the scene is evaluated every frame. A superset of what is
	/// strictly visible, deliberately: a beam that stopped being evaluated because the
	/// camera turned would freeze where it was.
	$effect(() => {
		const keys = placed.flatMap((fixture) =>
			(typeOf(fixture)?.parameters ?? []).map((p) => `${fixture.id}/${parameterKey(p.kind)}`)
		);
		const registered = watching(keys);
		return () => registered.stop();
	});

	/// Everything one fixture needs drawing.
	const beams = $derived(
		placed.map((fixture) => {
			const at = fixturePoint(fixture, $objectsById)!;
			const type = typeOf(fixture);
			const direction = beamDirection(fixture, type, $showing, $objectsById);
			const length = throwDistance(at, direction);
			const output = fixtureOutput(fixture, $showing);
			const bearing = bearingOnFloor(fixture, type, $showing);
			// The beam angle the file measured, where there is one. `stage.ts` reads
			// the type's own range; the constant is the fallback and says so.
			const half = (type?.physical?.beam_angle_deg ?? 14) / 2;
			const read = (kind: Parameters<typeof parameterKey>[0]) =>
				$showing.value(fixture.id, parameterKey(kind));
			const asNumber = (v: ReturnType<typeof read>) =>
				v?.type === 'Float' ? v.value : undefined;
			return {
				fixture,
				type,
				at,
				direction,
				length,
				output,
				bearing,
				// tan of the half-angle: the whole of what a zoom costs the shader.
				spread: Math.tan((Math.max(0.5, half) * Math.PI) / 180),
				// A shutter that is closed shuts the beam off entirely; absent means
				// open, because most fixtures have no mechanical shutter at all.
				shutter: asNumber(read('Shutter')) ?? 1,
				strobe: asNumber(read('Strobe')) ?? 0,
				tiltTurn: Math.atan2(-bearing.z, bearing.x),
				canPan: !!type?.parameters.some((p) => p.kind === 'Pan'),
				canTilt: !!type?.parameters.some((p) => p.kind === 'Tilt'),
				end: [
					at.x + direction.x * length,
					at.y + direction.y * length,
					at.z + direction.z * length
				] as [number, number, number]
			};
		})
	);

	const chosen = $derived(beams.filter((beam) => $selected.has(beam.fixture.id)));
	const sheetFor = $derived(chosen.length === 1 ? chosen[0] : null);

	// ── The three.js side ───────────────────────────────────────────────────────

	type Scene = ReturnType<typeof buildScene>;
	// `$state.raw`, not `$state`: a deep proxy would keep a plain field written
	// through it — `scene.mode = …` — in the proxy's own storage, and the render loop
	// holds the object itself and would never see it. The three.js instances inside
	// escaped that because class instances are not proxied, which is why the lights
	// and the pixel ratio worked and the mode did not.
	let scene = $state.raw<Scene | null>(null);

	/// Everything the renderer owns, built once per panel and torn down with it.
	function buildScene(element: HTMLDivElement) {
		const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
		// Capped by the view setting, not by the display: every device pixel is a
		// beam-shader invocation, and a Retina display's two per CSS pixel is what
		// pinned a GPU on the festival rig.
		// Read once, untracked: this runs inside the effect that builds the scene, and
		// a reactive read here would rebuild the whole scene — renderer, camera, controls
		// and all — every time somebody moved the work light slider, which is what
		// took the camera home each time. The effect below keeps both up to date.
		renderer.setPixelRatio(Math.min(window.devicePixelRatio, untrack(() => $view.resolution)));
		renderer.setClearColor(0x101010, 1);
		element.appendChild(renderer.domElement);
		renderer.domElement.style.display = 'block';
		renderer.domElement.style.width = '100%';
		renderer.domElement.style.height = '100%';

		const root = new THREE.Scene();
		const camera = new THREE.PerspectiveCamera(50, 1, 0.1, 400);
		camera.position.set(...home.position);

		const controls = new CameraControls(camera, renderer.domElement);
		controls.maxPolarAngle = Math.PI / 2 - 0.02;
		controls.minDistance = 1.5;
		controls.maxDistance = 200;

		// Enough ambient to read the rig and the plan when nothing is on, and no more:
		// this is a view of what the fixtures are doing, so the fixtures should be the
		// light in it. The view's work light scales both — this screen's setting, not
		// the show's, since it lights no lamp and says nothing about the room.
		const ambient = new THREE.AmbientLight(0x5a6478, AMBIENT_AT_FULL * DEFAULT_VIEW.workLight);
		const hemisphere = new THREE.HemisphereLight(0xffffff, 0x101010, HEMISPHERE_AT_FULL * DEFAULT_VIEW.workLight);
		root.add(ambient);
		root.add(hemisphere);

		// ── The deck ────────────────────────────────────────────────────────────
		const floorMaterial = new THREE.MeshStandardMaterial({ color: 0x242424, roughness: 0.95 });
		const deck = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), floorMaterial);
		deck.rotation.x = -Math.PI / 2;
		deck.receiveShadow = true;
		root.add(deck);

		// ── The grid ────────────────────────────────────────────────────────────
		//
		// An infinite grid in a shader rather than a `GridHelper`. The helper is a
		// fixed extent of line segments: it aliases badly past about forty metres and
		// simply stops at its own edge, so a rig larger than the plan stands on
		// nothing. This one is a single ground-plane quad, antialiased per pixel with
		// `fwidth`, at two scales with a distance fade.
		const grid = new THREE.Mesh(
			new THREE.PlaneGeometry(1, 1),
			new THREE.ShaderMaterial({
				transparent: true,
				depthWrite: false,
				side: THREE.DoubleSide,
				// `uLinear` is one on the photoreal path, where the frame is linear light
				// and the output pass encodes it: a grey written as a screen value there
				// comes out lighter than it was drawn.
				uniforms: { uNear: { value: 0 }, uFar: { value: 160 }, uLinear: { value: 0 } },
				vertexShader: /* glsl */ `
					varying vec3 vWorld;
					void main() {
						vec4 world = modelMatrix * vec4(position, 1.0);
						vWorld = world.xyz;
						gl_Position = projectionMatrix * viewMatrix * world;
					}
				`,
				fragmentShader: /* glsl */ `
					precision highp float;
					uniform float uFar;
					uniform float uLinear;
					varying vec3 vWorld;

					// How close this pixel is to a line of the given spacing, measured
					// in pixels rather than metres — which is what keeps the line one
					// pixel wide however far away it is.
					float line(vec2 p, float spacing) {
						vec2 grid = abs(fract(p / spacing - 0.5) - 0.5) / fwidth(p / spacing);
						return 1.0 - min(min(grid.x, grid.y), 1.0);
					}

					void main() {
						vec2 p = vWorld.xz;
						float metre = line(p, 1.0) * 0.28;
						float tenth = line(p, 10.0) * 0.5;
						float strength = max(metre, tenth);
						// Fade with distance so the horizon does not turn into moiré.
						float fade = 1.0 - smoothstep(uFar * 0.35, uFar, length(p));
						strength *= fade;
						if (strength <= 0.002) discard;
						float grey = mix(0.42, pow(0.42, 2.2), uLinear);
						gl_FragColor = vec4(vec3(grey), strength);
					}
				`
			})
		);
		grid.rotation.x = -Math.PI / 2;
		grid.position.y = -0.005;
		root.add(grid);

		// ── The beams ───────────────────────────────────────────────────────────
		//
		// One instanced mesh for the whole rig. Capacity grows when the rig outgrows
		// it and never shrinks: reallocating on the way down would churn buffers to
		// save memory nobody is short of.
		const beamMat = beamMaterial();
		const coneMat = coneMaterial();
		const beamMesh = new THREE.InstancedMesh(beamGeometry(), beamMat, 1);
		beamMesh.frustumCulled = false;
		beamMesh.count = 0;
		root.add(beamMesh);

		// ── The aim lines, for the wireframe mode ────────────────────────────────
		//
		// One segment per fixture from the lens to where the beam lands, in the
		// fixture's colour and dimmed by its level — never to nothing, so an unlit
		// head still says where it points. One geometry, grown with the rig.
		const lines = new THREE.LineSegments(
			new THREE.BufferGeometry(),
			new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.9 })
		);
		lines.frustumCulled = false;
		lines.visible = false;
		root.add(lines);

		// What every truss, deck and imported mesh is drawn with in wireframe: one
		// material per panel, since two panels can be in two modes and the stock
		// materials are shared by every panel on the page.
		const wire = new THREE.MeshBasicMaterial({ color: 0x7a8290, wireframe: true });

		// ── One light, mounted for the life of the panel ─────────────────────────
		//
		// The defect this replaces: a `SpotLight` inside `{#if level > 0.01}` changed
		// the scene's light count mid-fade, which changes three.js's program cache key
		// and recompiles every material in the scene. Mounted once and driven to zero
		// instead, so the count never changes.
		//
		// One rather than one per fixture, following the brightest: a scene with five
		// thousand real lights in it does not render at all, and what the pool is for
		// is showing that the floor is being lit, not simulating the rig.
		const pool = new THREE.SpotLight(0xffffff, 0, 60, 0.22, 0.45, 1.1);
		pool.position.set(0, 6, 0);
		root.add(pool);
		root.add(pool.target);

		return {
			renderer,
			root,
			camera,
			controls,
			deck,
			floorMaterial,
			grid,
			beamMesh,
			beamMat,
			pool,
			ambient,
			hemisphere,
			coneMat,
			lines,
			wire,
			/** The mode this screen asked for, and the one the scene is currently dressed as. */
			mode: DEFAULT_VIEW.mode as RenderMode,
			dressed: null as RenderMode | null,
			/** Set when something new was added to the scene and needs dressing for the mode. */
			restyle: true,
			/** The photoreal chain, built the first time it is asked for. */
			chain: null as null | { composer: EffectComposer; bloom: UnrealBloomPass },
			timer: gpuTimer(renderer),
			/** What each fixture was drawn as last frame, so a frame that changes nothing is not drawn. */
			previous: new Float32Array(0),
			/** Fixture bodies, kept between frames and keyed by fixture id. */
			bodies: new Map<string, THREE.Mesh>(),
			/** Drawn scene objects, keyed by object id. */
			objects: new Map<string, THREE.Object3D>(),
			/** The gizmo meshes, and what each one is. */
			gizmos: new Map<THREE.Object3D, Handle>(),
			gizmoGroup: (() => {
				const group = new THREE.Group();
				root.add(group);
				return group;
			})()
		};
	}

	$effect(() => {
		const element = host;
		if (!element) return;
		const built = buildScene(element);
		scene = built;

		// Size from the element rather than the window: this is one tile of a
		// workspace, and two of them can be open at once.
		let dirty = true;
		markDirty = () => {
			dirty = true;
		};

		const resize = new ResizeObserver(() => {
			const { clientWidth, clientHeight } = element;
			if (clientWidth < 1 || clientHeight < 1) return;
			built.renderer.setSize(clientWidth, clientHeight, false);
			built.chain?.composer.setSize(clientWidth, clientHeight);
			built.camera.aspect = clientWidth / clientHeight;
			built.camera.updateProjectionMatrix();
			dirty = true;
		});
		resize.observe(element);

		const [px, py, pz] = home.position;
		const [tx, ty, tz] = home.target;
		built.controls.setLookAt(px, py, pz, tx, ty, tz, false);

		let running = true;
		let previous = performance.now();
		/** When this panel last drew, so a 120 Hz display is drawn at 60. */
		let lastDrawn = -Infinity;
		let frames = 0;
		let work = 0;
		let elapsed = 0;
		// Our own elapsed seconds rather than `THREE.Clock`, which is deprecated, and
		// rather than `performance.now()` directly: the shader wants seconds since
		// this panel opened, so two rig panels drift independently instead of both
		// hazing off one enormous number where float precision has run out.
		let seconds = 0;

		// Drawn only when there is something new to see. Three things count: the
		// camera moved, the rig changed (a level, a colour, a direction — worked out
		// by comparing against the last frame's attributes rather than trusting the
		// stores, which tick every frame whether or not anything moved), or the
		// picture animates on its own clock — a lit beam with haze in it, a strobe.
		// A settled rig with nothing lit costs nothing at all, which is the answer to
		// a theatre with no cue up holding a GPU at forty percent.
		const tick = () => {
			if (!running) return;
			requestAnimationFrame(tick);
			const now = performance.now();
			if (now - lastDrawn < DRAW_EVERY_MS) return;
			const delta = (now - previous) / 1000;
			previous = now;
			seconds += delta;
			built.timer.poll();

			const moved = built.controls.update(delta);
			const { changed, animated } = draw(built, seconds);

			elapsed += delta;
			if (elapsed >= 1) {
				cpuMs = frames > 0 ? work / frames : 0;
				drawing = frames > 0;
				gpuMs = built.timer.take();
				frames = 0;
				work = 0;
				elapsed = 0;
			}

			if (!(moved || changed || animated || dirty)) return;
			dirty = false;
			lastDrawn = now;
			built.timer.begin();
			if (built.mode === 'photoreal') chainFor(built).composer.render();
			else built.renderer.render(built.root, built.camera);
			built.timer.end();
			frames += 1;
			work += performance.now() - now;
		};
		requestAnimationFrame(tick);

		return () => {
			running = false;
			resize.disconnect();
			built.controls.dispose();
			built.beamMesh.geometry.dispose();
			built.beamMat.dispose();
			built.coneMat.dispose();
			built.lines.geometry.dispose();
			built.wire.dispose();
			built.chain?.composer.dispose();
			for (const body of built.bodies.values()) {
				body.geometry.dispose();
				(body.material as THREE.Material).dispose();
			}
			built.renderer.dispose();
			built.renderer.domElement.remove();
			scene = null;
		};
	});

	// ── Drawing one frame ───────────────────────────────────────────────────────

	// Scratch objects, allocated once. Rebuilding a Quaternion, an Euler and a Color
	// per fixture per frame is what the old drawing did, and at five thousand
	// fixtures that stops being untidy and starts being the frame budget.
	const scratchQuat = new THREE.Quaternion();
	const scratchMatrix = new THREE.Matrix4();
	const scratchScale = new THREE.Vector3(1, 1, 1);
	const scratchDirection = new THREE.Vector3();
	const scratchColour = new THREE.Color();
	const scratchDimmed = new THREE.Color();
	const scratchPosition = new THREE.Vector3();
	const DOWN = new THREE.Vector3(0, -1, 0);

	const BODY_GEOMETRY = new THREE.CylinderGeometry(0.14, 0.11, 0.34, 16);

	/** How many numbers describe one fixture's drawing, for the last-frame record. */
	const PER_FIXTURE = 10;

	/**
	 * Put the rig into the scene, and say whether that changed the picture.
	 *
	 * `changed` is worked out against what was drawn last frame rather than read
	 * off a store: the output store ticks every animation frame whether or not a
	 * value moved, so a store-driven flag would say "changed" sixty times a second
	 * on a rig that has been sitting still since the interval. `animated` is a
	 * picture that moves on its own clock — a lit beam with haze drifting through
	 * it, a strobe between flashes — and has to be drawn whether or not the rig
	 * changed.
	 */
	function draw(built: Scene, seconds: number): { changed: boolean; animated: boolean } {
		const list = beams;
		let changed = false;
		let animated = false;

		if (built.dressed !== built.mode || built.restyle) {
			dress(built);
			changed = true;
		}
		const hazy = built.mode === 'real' || built.mode === 'photoreal';

		// Haze, from the show. It reaches no lamp, and it is show data because how
		// hazy the room is is a fact about the room rather than about the screen
		// looking at it.
		const density = show?.haze_density ?? 1;
		const turbulence = show?.haze_turbulence ?? 0.25;
		const uniforms = built.beamMat.uniforms;
		if (uniforms.uHazeDensity.value !== density || uniforms.uHazeTurbulence.value !== turbulence) {
			changed = true;
		}
		uniforms.uTime.value = seconds;
		uniforms.uHazeDensity.value = density;
		uniforms.uHazeTurbulence.value = turbulence;

		// The deck follows the plan it is showing.
		built.deck.scale.set(floor.width, floor.depth, 1);
		built.deck.position.set(
			plan ? plan.origin.x + floor.width / 2 : 0,
			0,
			plan ? plan.origin.z + floor.depth / 2 : 0
		);
		const span = Math.max(floor.width, floor.depth) * 8;
		built.grid.scale.set(span, span, 1);

		// ── Beams ───────────────────────────────────────────────────────────────
		grow(built, list.length);
		const colours = built.beamMesh.geometry.getAttribute('beamColor') as THREE.InstancedBufferAttribute;
		const levels = built.beamMesh.geometry.getAttribute('beamLevel') as THREE.InstancedBufferAttribute;
		const lengths = built.beamMesh.geometry.getAttribute('beamLength') as THREE.InstancedBufferAttribute;
		const spreads = built.beamMesh.geometry.getAttribute('beamSpread') as THREE.InstancedBufferAttribute;

		// The record of last frame, one row per fixture in the order they are listed.
		// A rig that grew or shrank is a different picture by definition.
		if (built.previous.length !== list.length * PER_FIXTURE) {
			built.previous = new Float32Array(list.length * PER_FIXTURE).fill(NaN);
			changed = true;
		}
		const previous = built.previous;

		let brightest = -1;
		let brightestLevel = 0;
		// Only lit beams get an instance, packed from the front. An unlit beam used
		// to be an instance whose fragments all discarded — which still rasterises
		// the whole cone — and on a theatre rig with no cue up that was every beam.
		let lit = 0;

		for (let i = 0; i < list.length; i++) {
			const beam = list[i];
			// A shutter that is shut, and a strobe between its flashes, are both a
			// beam that is not there this instant.
			const gate = strobeGate(beam.strobe, seconds) * (beam.shutter > 0.02 ? 1 : 0);
			const level = beam.output.level * gate;
			// A strobe is drawn on its own clock, like haze: the picture moves even
			// though nothing in the rig has changed.
			if (beam.strobe > 0.002 && beam.output.level > 0.0005) animated = true;

			scratchDirection.set(beam.direction.x, beam.direction.y, beam.direction.z).normalize();
			scratchPosition.set(beam.at.x, beam.at.y, beam.at.z);
			scratchColour.setRGB(beam.output.r, beam.output.g, beam.output.b);
			const length = drawnLength(beam.length, beam.direction, beam.spread, LENS_RADIUS);

			// Against last frame. Ten numbers say everything below draws from.
			const row = i * PER_FIXTURE;
			const now = [
				scratchPosition.x, scratchPosition.y, scratchPosition.z,
				scratchDirection.x, scratchDirection.y, scratchDirection.z,
				level, scratchColour.r, scratchColour.g, scratchColour.b
			];
			for (let n = 0; n < PER_FIXTURE; n++) {
				// Through `fround`, because the record is single precision and the
				// value is double: compared raw, a number that has not moved differs
				// from its own rounding every frame, and a settled rig draws forever.
				const value = Math.fround(now[n]);
				if (previous[row + n] !== value) {
					changed = true;
					previous[row + n] = value;
				}
			}

			if (level > 0.0005) {
				scratchQuat.setFromUnitVectors(DOWN, scratchDirection);
				scratchMatrix.compose(scratchPosition, scratchQuat, scratchScale);
				built.beamMesh.setMatrixAt(lit, scratchMatrix);
				colours.setXYZ(lit, scratchColour.r, scratchColour.g, scratchColour.b);
				levels.setX(lit, level);
				// Run on past the axis's floor hit, so the floor cuts the beam rather
				// than the cone's own square end standing half in the air on a slanted
				// throw.
				lengths.setX(lit, length);
				spreads.setX(lit, beam.spread);
				lit += 1;
			}

			if (level > brightestLevel) {
				brightestLevel = level;
				brightest = i;
			}

			// ── The body ────────────────────────────────────────────────────────
			let body = built.bodies.get(beam.fixture.id);
			if (!body) {
				body = new THREE.Mesh(
					BODY_GEOMETRY,
					new THREE.MeshStandardMaterial({ roughness: 0.6, metalness: 0.3 })
				);
				body.userData.fixtureId = beam.fixture.id;
				(body.material as THREE.MeshStandardMaterial).wireframe = built.mode === 'wireframe';
				built.bodies.set(beam.fixture.id, body);
				built.root.add(body);
			}
			body.visible = true;
			body.position.copy(scratchPosition);
			// The body's wider end is the lens, and that end does face down the beam —
			// the opposite end from the one the beam is turned by.
			body.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), scratchDirection);
			const material = body.material as THREE.MeshStandardMaterial;
			material.color.set($selected.has(beam.fixture.id) ? 0x4a9eff : 0x3a3a3a);
			// Tinted by its own output, so the render says what is hanging up there.
			// The prior art this drawing came from paints its bodies pure black, and
			// then the picture cannot tell you which lamp is on.
			dimKeepingHue(scratchColour, level * 0.9, scratchDimmed);
			material.emissive.copy(scratchDimmed);
			material.emissiveIntensity = 1;
		}

		// A lit beam with haze in it drifts on the clock, so the picture is never
		// still while one is on. With the haze at nothing it is, and a lit static
		// look costs no frames either — and a cone or a line has no haze to drift.
		if (hazy && lit > 0 && density > 0.001) animated = true;

		built.beamMesh.count = lit;
		if (changed) {
			built.beamMesh.instanceMatrix.needsUpdate = true;
			colours.needsUpdate = true;
			levels.needsUpdate = true;
			lengths.needsUpdate = true;
			spreads.needsUpdate = true;
		}

		// The aim lines, in wireframe only: every fixture, lit or not.
		if (built.mode === 'wireframe' && changed) {
			const geometry = built.lines.geometry;
			let positions = geometry.getAttribute('position') as THREE.BufferAttribute | undefined;
			let tints = geometry.getAttribute('color') as THREE.BufferAttribute | undefined;
			if (!positions || positions.count < list.length * 2) {
				positions = new THREE.BufferAttribute(new Float32Array(list.length * 6), 3);
				tints = new THREE.BufferAttribute(new Float32Array(list.length * 6), 3);
				geometry.setAttribute('position', positions);
				geometry.setAttribute('color', tints);
			}
			for (let i = 0; i < list.length; i++) {
				const beam = list[i];
				positions.setXYZ(i * 2, beam.at.x, beam.at.y, beam.at.z);
				positions.setXYZ(i * 2 + 1, beam.end[0], beam.end[1], beam.end[2]);
				const glow = 0.18 + 0.82 * beam.output.level;
				tints!.setXYZ(i * 2, beam.output.r * glow, beam.output.g * glow, beam.output.b * glow);
				tints!.setXYZ(i * 2 + 1, beam.output.r * glow * 0.35, beam.output.g * glow * 0.35, beam.output.b * glow * 0.35);
			}
			positions.needsUpdate = true;
			tints!.needsUpdate = true;
			geometry.setDrawRange(0, list.length * 2);
		}

		// Bodies for fixtures that are no longer drawn: hidden rather than disposed,
		// because a layer being toggled should not churn geometry.
		const live = new Set(list.map((b) => b.fixture.id));
		for (const [id, body] of built.bodies) {
			if (!live.has(id)) body.visible = false;
		}

		// ── The pool on the floor ───────────────────────────────────────────────
		if (brightest >= 0) {
			const beam = list[brightest];
			built.pool.position.set(beam.at.x, beam.at.y, beam.at.z);
			built.pool.target.position.set(...beam.end);
			built.pool.target.updateMatrixWorld();
			built.pool.color.setRGB(beam.output.r, beam.output.g, beam.output.b);
			built.pool.distance = Math.max(1, beam.length * 2.2);
			// The pool is exactly as wide as the beam that makes it: a spotlight's
			// angle is its half-angle, and the spread is the tangent of ours.
			built.pool.angle = Math.atan(beam.spread);
		}
		// Driven to zero rather than unmounted. The light count must not change.
		built.pool.intensity = brightestLevel * 14;

		drawGizmos(built);
		projectSheet(built);
		return { changed, animated };
	}

	/**
	 * Dress the scene for the mode this screen asked for.
	 *
	 * A mode is what a screen *draws*, so everything here is a material or a
	 * visibility flag on things the scene already has: the beam mesh wears the beam
	 * shader or the flat cone one; the bodies, trusses and decks wear their own
	 * materials or the panel's wire one; the aim lines and the floor pool come and
	 * go. Nothing is rebuilt, and switching costs one frame.
	 *
	 * The originals are kept on each mesh, since a truss's material is shared by
	 * every panel on the page and must not be edited in place.
	 */
	function dress(built: Scene) {
		const mode = built.mode;
		const wireframe = mode === 'wireframe';
		built.beamMesh.material = mode === 'cones' ? built.coneMat : built.beamMat;
		built.beamMesh.visible = !wireframe;
		built.lines.visible = wireframe;
		built.deck.visible = !wireframe;
		built.pool.visible = mode === 'real' || mode === 'photoreal';
		for (const body of built.bodies.values()) {
			(body.material as THREE.MeshStandardMaterial).wireframe = wireframe;
		}
		for (const group of built.objects.values()) {
			group.traverse((node) => {
				if (!(node instanceof THREE.Mesh)) return;
				if (wireframe) {
					if (node.material !== built.wire) node.userData.solid = node.material;
					node.material = built.wire;
				} else if (node.userData.solid) {
					node.material = node.userData.solid;
				}
			});
		}
		// Tone mapping is the photoreal chain's, applied by its output pass over the
		// summed frame; on the plain path three.js would apply it per material, to a
		// picture that was never summed, and the working view would change.
		const photoreal = mode === 'photoreal';
		built.renderer.toneMapping = photoreal ? THREE.ACESFilmicToneMapping : THREE.NoToneMapping;
		built.renderer.toneMappingExposure = 1;
		// The photoreal frame is linear light. Three things written as screen values
		// on the plain path are said in linear terms here: the clear colour, the grid's
		// grey, and the beams, which are too hot as light at a value that was right
		// for the screen.
		built.renderer.setClearColor(photoreal ? new THREE.Color(0x101010).convertSRGBToLinear() : 0x101010, 1);
		(built.grid.material as THREE.ShaderMaterial).uniforms.uLinear.value = photoreal ? 1 : 0;
		built.beamMat.uniforms.uGain.value = photoreal ? 0.5 : 1;
		built.dressed = mode;
		built.restyle = false;
	}

	/**
	 * The photoreal chain: the scene into a half-float target, bloom above white,
	 * and tone mapping over the *sum*.
	 *
	 * This is the post-processing chain task 51 stayed away from, and it is here for
	 * the one thing it alone can do. Beams add, and added into an 8-bit frame that
	 * clips at one per channel, two saturated colours are white the moment they
	 * cross. In a floating-point target the sum is kept, and ACES rolls it off
	 * towards white the way a sensor does, so a blue and an amber crossing stay a
	 * colour. The bloom's threshold is one: only what is *above* white glows, which
	 * is the halo a lens gives a lamp it is looking into. Four samples of
	 * multisampling on the target, since the plain path had them on the canvas.
	 */
	function chainFor(built: Scene) {
		if (built.chain) return built.chain;
		const size = built.renderer.getSize(new THREE.Vector2());
		const ratio = built.renderer.getPixelRatio();
		const target = new THREE.WebGLRenderTarget(size.x * ratio, size.y * ratio, {
			type: THREE.HalfFloatType,
			samples: 4
		});
		const composer = new EffectComposer(built.renderer, target);
		composer.setPixelRatio(ratio);
		composer.setSize(size.x, size.y);
		composer.addPass(new RenderPass(built.root, built.camera));
		// Tight and above white only. The first setting tried — 0.45 strength at a
		// radius of 0.3 — spread the stage's light across the whole frame at its
		// widest mip and lifted the sky to grey, which is fog on the lens and not a
		// halo round a lamp.
		const bloom = new UnrealBloomPass(new THREE.Vector2(size.x, size.y), 0.22, 0.1, 1.3);
		composer.addPass(bloom);
		composer.addPass(new OutputPass());
		built.chain = { composer, bloom };
		return built.chain;
	}

	/**
	 * GPU time per frame, where the browser will say.
	 *
	 * One query in flight at a time; its answer arrives a few frames later and is
	 * folded into a running mean. `null` throughout on a browser without the
	 * extension, so the panel prints nothing rather than a zero that reads as free.
	 */
	function gpuTimer(renderer: THREE.WebGLRenderer) {
		const gl = renderer.getContext() as WebGL2RenderingContext;
		const ext = gl.getExtension('EXT_disjoint_timer_query_webgl2') as {
			TIME_ELAPSED_EXT: number;
			GPU_DISJOINT_EXT: number;
		} | null;
		let query: WebGLQuery | null = null;
		let open = false;
		let total = 0;
		let answered = 0;
		return {
			begin() {
				if (!ext || query) return;
				query = gl.createQuery();
				if (!query) return;
				gl.beginQuery(ext.TIME_ELAPSED_EXT, query);
				open = true;
			},
			end() {
				if (!ext || !open) return;
				gl.endQuery(ext.TIME_ELAPSED_EXT);
				open = false;
			},
			/** Fold in an answer that has arrived. */
			poll() {
				if (!ext || !query || open) return;
				const ready = gl.getQueryParameter(query, gl.QUERY_RESULT_AVAILABLE) as boolean;
				const disjoint = gl.getParameter(ext.GPU_DISJOINT_EXT) as boolean;
				if (!ready && !disjoint) return;
				if (ready && !disjoint) {
					total += (gl.getQueryParameter(query, gl.QUERY_RESULT) as number) / 1e6;
					answered += 1;
				}
				gl.deleteQuery(query);
				query = null;
			},
			/** The mean since last asked, or `null` if nothing answered. */
			take(): number | null {
				if (!ext) return null;
				const mean = answered > 0 ? total / answered : null;
				total = 0;
				answered = 0;
				return mean;
			}
		};
	}

	// What this screen asks of the view: how bright the room is with nothing on,
	// and how many pixels to draw. Both are this browser's, kept in `localStorage`.
	$effect(() => {
		const built = scene;
		const { workLight, resolution } = $view;
		const element = host;
		if (!built || !element) return;
		built.ambient.intensity = AMBIENT_AT_FULL * workLight;
		built.hemisphere.intensity = HEMISPHERE_AT_FULL * workLight;
		built.mode = $view.mode;
		built.renderer.setPixelRatio(Math.min(window.devicePixelRatio, resolution));
		// A new pixel ratio takes effect at the next `setSize`.
		const { clientWidth, clientHeight } = element;
		if (clientWidth > 0 && clientHeight > 0) {
			built.renderer.setSize(clientWidth, clientHeight, false);
			built.chain?.composer.setPixelRatio(built.renderer.getPixelRatio());
			built.chain?.composer.setSize(clientWidth, clientHeight);
		}
		markDirty();
	});

	// Everything else on screen that the attribute comparison in `draw` cannot see:
	// what is selected and hovered, what is being dragged, the drawing, the plan.
	// Read here so the frame after any of them changes is drawn.
	$effect(() => {
		void $selected;
		void hovered;
		void grab;
		void $visibleObjects;
		void planUrl;
		void plan;
		markDirty();
	});

	/// Make room for a rig that has grown. Attributes are reallocated together, since
	/// they are all indexed by the same instance.
	function grow(built: Scene, wanted: number) {
		const mesh = built.beamMesh;
		if (mesh.instanceMatrix.count >= wanted && mesh.geometry.getAttribute('beamColor')) return;
		const capacity = Math.max(16, 1 << Math.ceil(Math.log2(Math.max(1, wanted))));
		if (mesh.instanceMatrix.count < capacity) {
			mesh.instanceMatrix = new THREE.InstancedBufferAttribute(new Float32Array(capacity * 16), 16);
			mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
		}
		const attribute = (size: number) =>
			new THREE.InstancedBufferAttribute(new Float32Array(capacity * size), size);
		mesh.geometry.setAttribute('beamColor', attribute(3));
		mesh.geometry.setAttribute('beamLevel', attribute(1));
		mesh.geometry.setAttribute('beamLength', attribute(1));
		mesh.geometry.setAttribute('beamSpread', attribute(1));
	}

	// ── The drawing: trusses, rostra, anything somebody put in the room ─────────

	$effect(() => {
		const built = scene;
		const objects = $visibleObjects;
		if (!built) return;
		const wanted = new Set(objects.map((o) => o.id));
		for (const [id, node] of built.objects) {
			if (!wanted.has(id)) {
				built.root.remove(node);
				built.objects.delete(id);
			}
		}
		let live = true;
		for (const object of objects) {
			if (built.objects.has(object.id)) {
				place(built.objects.get(object.id)!, object);
				continue;
			}
			const group = new THREE.Group();
			built.objects.set(object.id, group);
			built.root.add(group);
			built.restyle = true;
			place(group, object);
			const references =
				object.geometry.length > 0
					? object.geometry
					: ($symbols.find((s) => s.id === object.symbol)?.geometry ?? []);
			// Nothing to load, but a name the console knows how to draw: a truss this
			// station made for itself, or one out of a drawing that arrived without
			// its meshes. An imported mesh wins where there is one — it is the truth
			// about that object, and this is only what to draw when there is nothing
			// better.
			if (references.length === 0) {
				const stock = stockMesh(object.catalogue);
				if (stock) group.add(stock);
			}
			const world = worldTransform(object.transform, object.parent, $objectsById);
			const mirrored = world.scale.x * world.scale.y * world.scale.z < 0;
			Promise.all(references.map((r) => load(r.asset, r.file_name, $namedAssets)))
				.then((meshes) => {
					if (!live) return;
					meshes.forEach((mesh, index) => {
						const node = instance(mesh, mirrored);
						const reference = references[index];
						node.position.set(
							reference.transform.position.x,
							reference.transform.position.y,
							reference.transform.position.z
						);
						node.rotation.set(
							(reference.transform.rotation.x * Math.PI) / 180,
							(reference.transform.rotation.y * Math.PI) / 180,
							(reference.transform.rotation.z * Math.PI) / 180
						);
						node.scale.set(
							reference.transform.scale.x,
							reference.transform.scale.y,
							reference.transform.scale.z
						);
						group.add(node);
					});
					built.restyle = true;
					markDirty();
				})
				// A file the loader refuses leaves the group empty rather than taking
				// the view with it: a rig that goes blank over one bad mesh is worse
				// than one with a gap in it.
				.catch(() => {});
		}
		return () => {
			live = false;
		};
	});

	function place(node: THREE.Object3D, object: { transform: unknown; parent: string | null }) {
		const world = worldTransform(
			object.transform as Parameters<typeof worldTransform>[0],
			object.parent,
			$objectsById
		);
		node.position.set(world.position.x, world.position.y, world.position.z);
		node.rotation.set(
			(world.rotation.x * Math.PI) / 180,
			(world.rotation.y * Math.PI) / 180,
			(world.rotation.z * Math.PI) / 180
		);
		node.scale.set(world.scale.x, world.scale.y, world.scale.z);
	}

	// ── The plan, as a texture for the deck ─────────────────────────────────────

	$effect(() => {
		const built = scene;
		const url = planUrl;
		if (!built) return;
		if (!url) {
			built.floorMaterial.map = null;
			built.floorMaterial.color.set(0x242424);
			built.floorMaterial.needsUpdate = true;
			return;
		}
		let live = true;
		const texture = new THREE.TextureLoader().load(url, () => {
			if (!live) texture.dispose();
		});
		texture.colorSpace = THREE.SRGBColorSpace;
		built.floorMaterial.map = texture;
		// The plan is the deck, not a lightbox: knocked back so a beam landing on it
		// is the bright thing rather than the paper.
		built.floorMaterial.color.set(0x8a8a8a);
		built.floorMaterial.needsUpdate = true;
		return () => {
			live = false;
			texture.dispose();
		};
	});

	// ── The camera ──────────────────────────────────────────────────────────────

	/** Put the camera back where the view opened. */
	export function goHome() {
		const [px, py, pz] = home.position;
		const [tx, ty, tz] = home.target;
		scene?.controls.setLookAt(px, py, pz, tx, ty, tz, true);
	}

	const focused = $derived($selected.size === 1 ? [...$selected][0] : null);
	$effect(() => {
		const id = focused;
		const built = scene;
		if (!follow || !id || !built) return;
		// Only picking a *different* fixture may move the camera. Reading the rig here
		// would re-frame on every live value, with the view lurching each time.
		untrack(() => {
			const fixture = fixtures.find((f) => f.id === id);
			const at = fixture && fixturePoint(fixture, $objectsById);
			if (!at) return;
			// Stand off along the way the camera is already looking, so framing a
			// fixture turns the view towards it rather than teleporting round to the
			// other side of it.
			const from = new THREE.Vector3();
			built.camera.getWorldPosition(from);
			const back = from.sub(new THREE.Vector3(at.x, at.y, at.z));
			if (back.lengthSq() < 1e-6) back.set(0, 1, 5);
			back.setLength(4.5);
			built.controls.setLookAt(
				at.x + back.x,
				at.y + back.y,
				at.z + back.z,
				at.x,
				at.y,
				at.z,
				true
			);
		});
	});

	// ── Dragging a gizmo ────────────────────────────────────────────────────────

	/** Which gizmo, on which fixture — all a hover or a hit test needs to say. */
	type Handle = { kind: 'pan' | 'tilt' | 'spot'; id: string };

	type Grab = Handle & {
		/** The axis's value when the gizmo was taken hold of, 0–1. */
		from: number;
		/** The last angle read under the pointer, in degrees. */
		angle: number;
		/** How far the pointer has gone round since, in degrees. */
		turned: number;
		/** For the beam spot: where the beam landed, relative to the pointer. */
		offset: { x: number; z: number };
	};
	let grab = $state<Grab | null>(null);
	let hovered = $state<Handle | null>(null);

	const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
	const caster = new THREE.Raycaster();

	const PAN_GEOMETRY = new THREE.TorusGeometry(0.46, 0.035, 8, 48);
	const TILT_GEOMETRY = new THREE.TorusGeometry(0.36, 0.035, 8, 32, Math.PI);
	const SPOT_GEOMETRY = new THREE.CircleGeometry(0.4, 32);

	/// Is this gizmo the one the pointer is on, or the one being dragged?
	const live = (kind: Handle['kind'], id: string) =>
		(grab ?? hovered)?.kind === kind && (grab ?? hovered)?.id === id;

	/**
	 * The gizmos, rebuilt per frame from what is selected.
	 *
	 * Only on what is selected, because a rig of a hundred heads wearing rings would
	 * be a ball of wire rather than a picture of a room — which is also why these are
	 * never the case instancing would be for.
	 *
	 * `depthTest: false`, so a ring is never buried inside the fixture body it belongs
	 * to. A handle you cannot see is a handle you cannot grab.
	 */
	function drawGizmos(built: Scene) {
		built.gizmoGroup.clear();
		built.gizmos.clear();
		for (const beam of chosen) {
			const add = (kind: Handle['kind'], geometry: THREE.BufferGeometry, colour: number) => {
				const mesh = new THREE.Mesh(
					geometry,
					new THREE.MeshBasicMaterial({
						color: live(kind, beam.fixture.id) ? 0xf59e0b : colour,
						depthTest: false,
						transparent: kind === 'spot',
						opacity: kind === 'spot' ? 0.85 : 1,
						side: kind === 'spot' ? THREE.DoubleSide : THREE.FrontSide
					})
				);
				mesh.renderOrder = 10;
				built.gizmoGroup.add(mesh);
				built.gizmos.set(mesh, { kind, id: beam.fixture.id });
				return mesh;
			};

			if (beam.canPan) {
				const ring = add('pan', PAN_GEOMETRY, 0x4a9eff);
				ring.position.set(beam.at.x, beam.at.y, beam.at.z);
				ring.rotation.set(-Math.PI / 2, 0, 0);
			}
			if (beam.canTilt) {
				const arc = add('tilt', TILT_GEOMETRY, 0x22c55e);
				arc.position.set(beam.at.x, beam.at.y, beam.at.z);
				arc.rotation.set(0, beam.tiltTurn, 0);
			}
			if (beam.canPan || beam.canTilt) {
				const disc = add('spot', SPOT_GEOMETRY, 0x4a9eff);
				disc.position.set(beam.end[0], 0.02, beam.end[2]);
				disc.rotation.set(-Math.PI / 2, 0, 0);
			}
		}
	}

	/// What an axis is showing now, which is where a drag of it starts from.
	///
	/// Evaluated rather than read: a head half way through a cue's move is somewhere
	/// nothing has written down, and grabbing it has to start from where it actually
	/// is rather than from where the last thing anybody stored had it.
	function axisOf(fixture: Fixture, key: 'Pan' | 'Tilt'): number {
		const value = $showing.value(fixture.id, key);
		return value?.type === 'Float' ? value.value : 0.5;
	}

	/**
	 * Taking hold of a gizmo, or picking a fixture.
	 *
	 * In the *capture* phase, and this is the whole trick: the orbit controls listen
	 * for `pointerdown` on this same element, and a gizmo drag and a camera drag
	 * cannot share a pointer. Capture runs before the event reaches the canvas and
	 * therefore before the controls hear it, so stopping it there is the one place the
	 * decision can be made cleanly.
	 */
	$effect(() => {
		const built = scene;
		const element = host;
		if (!built || !element) return;

		const press = (event: PointerEvent) => {
			if (grab || event.button !== 0) return;
			// A press that landed on a panel floating over the scene belongs to that
			// panel. The raycast cannot see the panel, so it would happily find a
			// gizmo behind it and start dragging one from under a fader.
			if (!(event.target instanceof HTMLCanvasElement)) return;
			const found = gizmoAt(built, event);
			if (found) {
				event.stopPropagation();
				event.preventDefault();
				hovered = found;
				beginGrab(built, found.kind, found.id, event);
				return;
			}
			// Not a gizmo: maybe a fixture body, which is what picking one is.
			const body = bodyAt(built, event);
			if (body) {
				event.stopPropagation();
				if (event.shiftKey) toggle(body);
				else select(body);
			}
		};

		// Hover, which Threlte's `interactivity()` used to supply. One raycast per
		// pointer move against the gizmos only, which is at most three per selected
		// fixture and nothing at all when the selection is empty.
		const move = (event: PointerEvent) => {
			if (grab) return;
			hovered = gizmoAt(built, event);
		};

		// A camera transition that is still running when somebody grabs the view is a
		// camera fighting its operator. Any pointer or wheel input calls it off.
		const interrupt = () => built.controls.stop();

		element.addEventListener('pointerdown', press, { capture: true });
		element.addEventListener('pointermove', move);
		element.addEventListener('pointerdown', interrupt);
		element.addEventListener('wheel', interrupt, { passive: true });
		return () => {
			element.removeEventListener('pointerdown', press, { capture: true });
			element.removeEventListener('pointermove', move);
			element.removeEventListener('pointerdown', interrupt);
			element.removeEventListener('wheel', interrupt);
		};
	});

	/// The pointer in clip space, which is what a raycaster wants.
	function pointerNdc(event: PointerEvent): THREE.Vector2 | null {
		if (!host) return null;
		const rect = host.getBoundingClientRect();
		if (rect.width < 1 || rect.height < 1) return null;
		return new THREE.Vector2(
			((event.clientX - rect.left) / rect.width) * 2 - 1,
			-((event.clientY - rect.top) / rect.height) * 2 + 1
		);
	}

	function gizmoAt(built: Scene, event: PointerEvent): Handle | null {
		const ndc = pointerNdc(event);
		if (!ndc || built.gizmos.size === 0) return null;
		caster.setFromCamera(ndc, built.camera);
		const hit = caster.intersectObjects([...built.gizmos.keys()], false)[0];
		return hit ? (built.gizmos.get(hit.object) ?? null) : null;
	}

	function bodyAt(built: Scene, event: PointerEvent): string | null {
		const ndc = pointerNdc(event);
		if (!ndc) return null;
		caster.setFromCamera(ndc, built.camera);
		const visible = [...built.bodies.values()].filter((b) => b.visible);
		const hit = caster.intersectObjects(visible, false)[0];
		return (hit?.object.userData.fixtureId as string) ?? null;
	}

	/// The pointer as a ray through the scene. The gizmos are surfaces in the room, so
	/// where the pointer *is* only means something once it has been turned back into
	/// one.
	function rayFrom(built: Scene, event: PointerEvent): Ray | null {
		const ndc = pointerNdc(event);
		if (!ndc) return null;
		caster.setFromCamera(ndc, built.camera);
		return { origin: { ...caster.ray.origin }, direction: { ...caster.ray.direction } };
	}

	function beginGrab(built: Scene, kind: Handle['kind'], id: string, event: PointerEvent) {
		const beam = beams.find((b) => b.fixture.id === id);
		const ray = rayFrom(built, event);
		if (!beam || !ray) return;

		const start = { kind, id, from: 0, angle: 0, turned: 0, offset: { x: 0, z: 0 } };
		if (kind === 'spot') {
			// Keep the beam where it is relative to the pointer, so grabbing the disc
			// off-centre does not tug the light sideways before the drag has begun.
			const point = rayOnPlane(ray, { x: 0, y: 0, z: 0 }, { x: 0, y: 1, z: 0 });
			if (!point) return;
			start.offset = { x: beam.end[0] - point.x, z: beam.end[2] - point.z };
		} else {
			const angle = angleUnder(kind, beam, ray);
			if (angle === null) return;
			start.angle = angle;
			start.from = axisOf(beam.fixture, kind === 'pan' ? 'Pan' : 'Tilt');
		}

		grab = start;
		// Aiming a head is one act however many frames it takes, so the whole drag
		// costs one Ctrl-Z. Bounded here rather than by an action on the canvas,
		// because a grab only counts once a gizmo is actually under the pointer —
		// spinning the camera changes nothing there is to take back.
		beginGesture();
		const onMove = (e: PointerEvent) => onDrag(built, e);
		window.addEventListener('pointermove', onMove);
		window.addEventListener(
			'pointerup',
			() => {
				grab = null;
				hovered = null;
				endGesture();
				window.removeEventListener('pointermove', onMove);
			},
			{ once: true }
		);
	}

	/// The angle the pointer is at, in the plane the grabbed axis turns in.
	function angleUnder(kind: 'pan' | 'tilt', beam: (typeof beams)[number], ray: Ray) {
		if (kind === 'pan') {
			// Pan turns about the vertical, so the drag is read off the horizontal
			// plane through the fixture.
			const point = rayOnPlane(ray, beam.at, { x: 0, y: 1, z: 0 });
			return point ? bearingFromPoint(beam.fixture, point) : null;
		}
		// Tilt nods in the vertical plane the head is panned to, so that is the plane
		// the drag is read off.
		const normal: Vec3 = { x: -beam.bearing.z, y: 0, z: beam.bearing.x };
		const point = rayOnPlane(ray, beam.at, normal);
		return point ? elevationFromPoint(beam.fixture, beam.type, point, $showing) : null;
	}

	function onDrag(built: Scene, event: PointerEvent) {
		if (!grab) return;
		const beam = beams.find((b) => b.fixture.id === grab!.id);
		const ray = rayFrom(built, event);
		if (!beam || !ray) return;

		if (grab.kind !== 'spot') {
			const angle = angleUnder(grab.kind, beam, ray);
			if (angle === null) return;
			grab.turned += wrapDegrees(angle - grab.angle);
			grab.angle = angle;

			// The head's own travel, so a drag on a 630° head and a drag on a 540° one
			// move each of them by the degrees the pointer actually turned.
			const key = grab.kind === 'pan' ? ('Pan' as const) : ('Tilt' as const);
			const travel = travelOf(typeOf(beam.fixture), key);
			setValue([beam.fixture.id], key, {
				type: 'Float',
				value: clamp01(grab.from + grab.turned / travel)
			});
			return;
		}

		// The beam spot: dragged across the floor, and both axes follow it.
		const point = rayOnPlane(ray, { x: 0, y: 0, z: 0 }, { x: 0, y: 1, z: 0 });
		if (!point) return;
		const target = { x: point.x + grab.offset.x, y: 0, z: point.z + grab.offset.z };
		const { pan, tilt } = aimAt(beam.fixture, beam.type, target, $objectsById);
		if (pan !== null) setValue([beam.fixture.id], 'Pan', { type: 'Float', value: pan });
		if (tilt !== null) setValue([beam.fixture.id], 'Tilt', { type: 'Float', value: tilt });
	}

	// ── The quicksheet, at the fixture ──────────────────────────────────────────
	//
	// What `HTML` from `@threlte/extras` used to do, which is the one piece of it with
	// no direct three.js equivalent: project a world point into the panel and put a
	// DOM element there. The spec asks for programming to happen at the light rather
	// than in a panel elsewhere, and this is that, literally.

	const sheetPoint = new THREE.Vector3();

	function projectSheet(built: Scene) {
		const at = sheetFor?.at;
		if (!at || !host) {
			if (sheetAt.visible) sheetAt = { ...sheetAt, visible: false };
			return;
		}
		sheetPoint.set(at.x, at.y, at.z).project(built.camera);
		// Behind the camera projects to a point that is nonsense on screen, so the
		// sheet hides rather than appearing mirrored on the far side of the panel.
		const behind = sheetPoint.z > 1;
		const rect = host.getBoundingClientRect();
		const x = ((sheetPoint.x + 1) / 2) * rect.width;
		const y = ((1 - sheetPoint.y) / 2) * rect.height;
		// Only when it has actually moved: this runs every frame, and assigning a
		// `$state` object per frame would re-render the sheet sixty times a second.
		if (
			sheetAt.visible === behind ||
			Math.abs(sheetAt.x - x) > 0.5 ||
			Math.abs(sheetAt.y - y) > 0.5
		) {
			sheetAt = { x, y, visible: !behind };
		}
	}
</script>

<div class="viewport" bind:this={host}></div>

{#if sheetFor && sheetAt.visible}
	<!-- Beside the fixture rather than over it: the rings and the beam spot are the
	     thing being worked on, and a panel sitting on top of them would hide exactly
	     what the sheet is for.
	     Pointer and wheel events stop here. The camera controls listen on the element
	     this sits inside, so without it every drag of a fader would also swing the
	     camera and every scroll would dolly it. -->
	<div
		class="beside"
		role="presentation"
		style:left="{sheetAt.x}px"
		style:top="{sheetAt.y}px"
		onpointerdown={(e) => e.stopPropagation()}
		onpointermove={(e) => e.stopPropagation()}
		onwheel={(e) => e.stopPropagation()}
		oncontextmenu={(e) => e.stopPropagation()}
	>
		<Quicksheet fixture={sheetFor.fixture} />
	</div>
{/if}

<style>
	.viewport {
		position: absolute;
		inset: 0;
		overflow: hidden;
	}

	.beside {
		position: absolute;
		/* Beside and vertically centred on the fixture, which is what the transform
		   did when this was an `HTML` overlay. */
		transform: translate(28px, -50%);
		pointer-events: auto;
		/* Below the console's own chrome: this panel lives *inside* the scene and must
		   not cover the store menu or any other modal. */
		z-index: 20;
	}
</style>
