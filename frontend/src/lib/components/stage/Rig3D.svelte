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

	import type {
		Fixture,
		FixtureType,
		Mount,
		SceneObject,
		Show,
		StagePlan,
		Transform,
		Vec3
	} from '$lib/generated/index.js';
	import {
		aimAt,
		beamDirection,
		drawnLength,
		fixtureOutput,
		fixturePoint,
		planExtent,
		throwDistance,
		wrapDegrees,
		travelOf
	} from '$lib/stage.js';
	import {
		FIELD_OF_VIEW,
		focusShot,
		orthoFrame,
		presetShot,
		projectionFor,
		rigBounds,
		type Bounds,
		type Shot,
		type ViewPreset
	} from '$lib/camera.js';
	import { TransformControls } from 'three/examples/jsm/controls/TransformControls.js';
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
	import { clearSelection, selected, select, toggle } from '$lib/stores/selection.js';
	import {
		fixtureIsVisible,
		hiddenLayers,
		namedAssets,
		objectsById,
		symbols,
		visibleObjects
	} from '$lib/stores/scene.js';
	import { instance, load, meshSizes } from '$lib/geometry.js';
	import { CATALOGUE, piece, stockMesh } from '$lib/stock.js';
	import { IDENTITY, localOf, parentWorld, worldTransform } from '$lib/scene.js';
	import { chordsFor, mountPoint, nearestMount, type Chord } from '$lib/mount.js';
	import {
		connectorsOf,
		freeConnectors,
		placedOnConnector,
		pointToGrid,
		snapConnectors,
		SNAP_RADIUS,
		type PlacedConnector
	} from '$lib/snap.js';
	import {
		clampFixture,
		moveFixtures,
		moveObjects,
		placePiece,
		asOneAct
	} from '$lib/stores/editor.js';
	import {
		clearObjects,
		isLocked,
		layers as layerRows,
		pivot,
		pivotPoint,
		selectedObjects,
		selectObject,
		toggleObject
	} from '$lib/stores/scene.js';
	import { setValue } from '$lib/stores/programmer.js';
	import { output as showing, watching } from '$lib/stores/output.js';
	import { parameterKey } from '$lib/patch.js';
	import Quicksheet from '$lib/components/programmer/Quicksheet.svelte';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';
	import { DEFAULT_VIEW, setView, view, type Projection, type RenderMode } from '$lib/stores/view.js';

	// `camera-controls` is a library rather than a wrapper: it wants the three.js
	// pieces it uses handed to it once per process.
	CameraControls.install({ THREE });

	let {
		fixtures,
		types,
		plan,
		planUrl,
		show,
		follow = false,
		gizmoMode = 'translate'
	}: {
		fixtures: Fixture[];
		types: FixtureType[];
		plan: StagePlan | null;
		planUrl: string | null;
		show: Show | null;
		follow?: boolean;
		/** What the drawing's gizmo does. The toolbar has to show which of the three is
		 *  on, so it is the panel's state and arrives here as a prop. */
		gizmoMode?: 'translate' | 'rotate' | 'scale';
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
	// operator's hands every time a cue changed something. The buttons are how it is
	// asked for again, which is the whole of `camera-home-presets`.
	let home = $state(presetShot('front', rigBounds([], new Map())));
	let framed = false;
	$effect(() => {
		if (framed || placed.length === 0) return;
		framed = true;
		home = untrack(() =>
			presetShot('front', rigBounds(placed, $objectsById, { pieces: $visibleObjects }), aspect())
		);
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
		// Two cameras rather than two panels. A 2D view *is* this one seen straight on:
		// one editor, one gizmo, one hit test, and ortho with the three-quarter preset
		// is an axonometric nobody had to build. The near and far planes are wide open
		// on the ortho one because its distance stops meaning anything — what decides
		// how much is on screen is the frustum, which `orthoFrame` works out.
		const perspective = new THREE.PerspectiveCamera(FIELD_OF_VIEW, 1, 0.1, 400);
		perspective.position.set(...home.position);
		const ortho = new THREE.OrthographicCamera(-10, 10, 6, -6, -500, 1000);
		ortho.position.set(...home.position);
		const camera: THREE.PerspectiveCamera | THREE.OrthographicCamera = perspective;

		const controls = new CameraControls(perspective, renderer.domElement);
		controls.maxPolarAngle = Math.PI / 2 - 0.02;
		controls.minDistance = 1.5;
		controls.maxDistance = 200;

		// ── The gizmo ────────────────────────────────────────────────────────────
		//
		// three's own, attached to a pivot rather than to an object: a selection of
		// four trusses has no single transform to drive, and the pivot is where an
		// operator says the turn happens. What each frame of a drag does is apply the
		// pivot's *delta* to everything selected, which is one rule for one object and
		// for forty.
		const pivotNode = new THREE.Object3D();
		root.add(pivotNode);
		const gizmo = new TransformControls(perspective, renderer.domElement);
		gizmo.attach(pivotNode);
		gizmo.enabled = false;
		const gizmoRoot = gizmo.getHelper();
		gizmoRoot.visible = false;
		root.add(gizmoRoot);

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
			/** Whichever of the two is drawing. Written by `useProjection`. */
			camera: camera as THREE.PerspectiveCamera | THREE.OrthographicCamera,
			perspective,
			ortho,
			/** What shape the tile is. The perspective camera keeps its own; ortho has
			 *  none, and both presets and the frustum need one. */
			aspect: 16 / 9,
			projection: DEFAULT_VIEW.projection as Projection,
			/** The box the last preset framed, so a resize can refit the ortho view. */
			framed: null as Bounds | null,
			preset: 'front' as ViewPreset,
			controls,
			gizmo,
			gizmoRoot,
			pivotNode,
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
			chain: null as null | { composer: EffectComposer; bloom: UnrealBloomPass; render: RenderPass },
			timer: gpuTimer(renderer),
			/** What each fixture was drawn as last frame, so a frame that changes nothing is not drawn. */
			previous: new Float32Array(0),
			/** Fixture bodies, kept between frames and keyed by fixture id. */
			bodies: new Map<string, THREE.Mesh>(),
			/** Drawn scene objects, keyed by object id. */
			objects: new Map<string, THREE.Object3D>(),
			/** And the invisible box round each: what the pointer finds, and what a
			 *  selection is outlined with. */
			picks: new Map<string, THREE.Mesh>(),
			/** The gizmo meshes, and what each one is. */
			gizmos: new Map<THREE.Object3D, Handle>(),
			/** The `+` sprites on a selected piece's free joints. */
			handles: new Map<THREE.Object3D, PlacedConnector>(),
			handleGroup: (() => {
				const group = new THREE.Group();
				root.add(group);
				return group;
			})(),
			outlineGroup: (() => {
				const group = new THREE.Group();
				root.add(group);
				return group;
			})(),
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
			built.aspect = clientWidth / clientHeight;
			built.perspective.aspect = built.aspect;
			built.perspective.updateProjectionMatrix();
			// The ortho camera has no aspect of its own: what a tile's shape decides is
			// how wide its frustum has to be for the same box to fit.
			refitOrtho(built);
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
			built.gizmo.detach();
			built.gizmo.dispose();
			built.outlineGroup.clear();
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

		// The deck follows the plan it is showing, which is what makes an ortho plan
		// view a tracing surface: a piece dragged in lands on the grid over the paper
		// somebody drew. Its turn and its opacity are the plan's own, so a drawing that
		// was scanned at an angle lies where it was calibrated to.
		built.deck.scale.set(floor.width, floor.depth, 1);
		built.deck.position.set(
			plan ? plan.origin.x + floor.width / 2 : 0,
			0,
			plan ? plan.origin.z + floor.depth / 2 : 0
		);
		built.deck.rotation.set(-Math.PI / 2, 0, ((plan?.rotation_deg ?? 0) * Math.PI) / 180);
		built.floorMaterial.opacity = plan ? Math.min(1, Math.max(0.05, plan.opacity)) : 1;
		built.floorMaterial.transparent = !!plan && plan.opacity < 1;
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
		const render = new RenderPass(built.root, built.camera);
		composer.addPass(render);
		// Tight and above white only. The first setting tried — 0.45 strength at a
		// radius of 0.3 — spread the stage's light across the whole frame at its
		// widest mip and lifted the sky to grey, which is fog on the lens and not a
		// halo round a lamp.
		const bloom = new UnrealBloomPass(new THREE.Vector2(size.x, size.y), 0.22, 0.1, 1.3);
		composer.addPass(bloom);
		composer.addPass(new OutputPass());
		built.chain = { composer, bloom, render };
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
	// how many pixels to draw, and whether the rig is seen in perspective or straight
	// on. All this browser's, kept in `localStorage`.
	$effect(() => {
		const built = scene;
		const { workLight, resolution } = $view;
		if (built) useProjection(built, $view.projection);
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

	// A menu belongs to the joint it was opened on, so letting go of the piece puts it
	// away — otherwise it hangs over the canvas offering to build on nothing.
	$effect(() => {
		void $selectedObjects;
		jointMenu = null;
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
				built.picks.delete(id);
			}
		}
		for (const object of objects) {
			if (built.objects.has(object.id)) {
				place(built.objects.get(object.id)!, object);
				continue;
			}
			const group = new THREE.Group();
			// What a raycast finds when somebody clicks a truss. On the group and on
			// everything under it, because a hit lands on a mesh several levels down
			// inside a loaded `.glb` and walking back up to find the group is work the
			// hit test would have to do on every pointer move.
			group.userData.objectId = object.id;
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
			//
			// It is a *download* now rather than a shape built here, and that is the
			// whole of what makes an exported MVR carry a truss: `/stock/{id}.glb` is
			// generated from the same table by the station, so the bytes drawn are the
			// bytes exported. Cached per URL, so a hundred sections cost one request.
			if (references.length === 0 && object.catalogue) {
				void stockMesh(object.catalogue, object.properties).then((stock) => {
					// Against the *group*, not against a per-run flag. This effect
					// re-runs whenever the drawing changes, and a flag cleared by the
					// teardown would throw away a mesh that had not arrived yet — after
					// which the object is already in `built.objects` and is never loaded
					// again, so the truss is simply missing. The group is removed only
					// when the object goes, which is the thing actually being asked.
					if (!stock || built.objects.get(object.id) !== group) return;
					stock.userData.objectId = object.id;
					stock.traverse((node) => (node.userData.objectId = object.id));
					givePickBox(group, stock, object.id);
					group.add(stock);
					built.restyle = true;
					markDirty();
				});
			}
			const world = worldTransform(object.transform, object.parent, $objectsById);
			const mirrored = world.scale.x * world.scale.y * world.scale.z < 0;
			Promise.all(references.map((r) => load(r.asset, r.file_name, $namedAssets)))
				.then((meshes) => {
					if (built.objects.get(object.id) !== group) return;
					meshes.forEach((mesh, index) => {
						const node = instance(mesh, mirrored);
						node.traverse((child) => (child.userData.objectId = object.id));
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
						givePickBox(group, node, object.id);
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
	});

	/**
	 * A box round a drawn piece, for the pointer to find.
	 *
	 * **A truss is mostly holes.** Four chords and a zig-zag of bracing, and a click on
	 * the middle of one in plan goes straight between the chords and hits nothing — so
	 * an operator aiming at a bar has to hit a tube, which at 50 mm across on a
	 * twelve-metre frame is a few pixels. The same is true of an imported mesh, and
	 * more so of a ladder.
	 *
	 * So the raycast is given something solid to find. It is `visible = false`, which
	 * costs nothing to draw and is not skipped by the raycast — three's `Raycaster`
	 * tests an object's *layers* and never its visibility, which is the one fact this
	 * rests on. Its size comes off what was actually drawn rather than from the
	 * catalogue, so the deck's origin-at-the-top and the panel's origin-at-the-bottom
	 * need no second telling.
	 *
	 * Measured **before** the drawn mesh is put in the group, and that ordering is the
	 * whole of it: `Box3.setFromObject` answers in the object's parent's space, so
	 * measuring it once it is inside the group gives world coordinates — which, written
	 * back as a position *inside* that group, puts the box at twice the truss's own
	 * offset and leaves the bar unclickable.
	 */
	function givePickBox(group: THREE.Object3D, drawn: THREE.Object3D, id: string) {
		const bounds = new THREE.Box3().setFromObject(drawn);
		if (bounds.isEmpty()) return;
		const size = bounds.getSize(new THREE.Vector3());
		const centre = bounds.getCenter(new THREE.Vector3());
		if (!Number.isFinite(size.x) || !Number.isFinite(size.y) || !Number.isFinite(size.z)) return;
		const box = new THREE.Mesh(
			new THREE.BoxGeometry(Math.max(size.x, 0.02), Math.max(size.y, 0.02), Math.max(size.z, 0.02)),
			PICKABLE
		);
		box.position.copy(centre);
		box.visible = false;
		box.userData.objectId = id;
		group.add(box);
		// The same box is what a selection is outlined with, so knowing how far a
		// truss goes costs one lookup rather than a second measurement.
		scene?.picks.set(id, box);
	}

	/**
	 * What a pick box wears. Shared, and never drawn — the box is invisible — but a
	 * `Mesh` with no material is not raycast at all, so it needs one.
	 */
	const PICKABLE = new THREE.MeshBasicMaterial({ visible: false });

	/** What a selected piece is outlined in. One material for the whole panel. */
	const OUTLINE = new THREE.LineBasicMaterial({ color: 0x4a9eff, depthTest: false });

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

	/// What shape the tile is, which decides how far back a preset has to stand: a
	/// portrait tablet needs more room for the same rig than a wide monitor does.
	function aspect(): number {
		const built = scene;
		if (!built) return 16 / 9;
		return built.aspect > 0.01 ? built.aspect : 16 / 9;
	}

	/** Take one of the four shots, framing everything the view draws. */
	export function frame(preset: ViewPreset) {
		const built = scene;
		if (!built) return;
		const bounds = rigBounds(placed, $objectsById, { pieces: $visibleObjects });
		built.framed = bounds;
		built.preset = preset;
		// A plan and a section are drawings and are read straight on; the front and the
		// three-quarter are pictures of a room. A default, not a rule: the toggle is
		// beside the presets and stays wherever somebody puts it afterwards.
		setView({ projection: projectionFor(preset) });
		refitOrtho(built);
		take(presetShot(preset, bounds, aspect()));
	}

	/**
	 * Point the ortho camera's frustum at whatever was last framed.
	 *
	 * The perspective presets work out a *distance*; an ortho camera has no lens angle
	 * and needs a frame instead — and it needs it again on every resize, because a tile
	 * that got narrower shows less of the same box rather than the same amount smaller.
	 * The zoom `camera-controls` has put on it is kept: refitting on a resize must not
	 * throw away somebody's zoom.
	 */
	function refitOrtho(built: Scene) {
		const bounds = built.framed ?? rigBounds(placed, $objectsById, { pieces: $visibleObjects });
		const { halfWidth, halfHeight } = orthoFrame(bounds, aspect(), built.preset);
		built.ortho.left = -halfWidth;
		built.ortho.right = halfWidth;
		built.ortho.top = halfHeight;
		built.ortho.bottom = -halfHeight;
		built.ortho.updateProjectionMatrix();
	}

	/**
	 * Swap which camera is drawing.
	 *
	 * Three things have to follow it and each was a blank screen first: the orbit
	 * controls, which hold one camera and work out a dolly from its kind; the gizmo,
	 * which sizes its handles in screen terms; and the photoreal chain's render pass,
	 * which binds a camera when it is built and would otherwise go on drawing from
	 * wherever the other one was standing.
	 */
	function useProjection(built: Scene, projection: Projection) {
		if (built.projection === projection) return;
		const next = projection === 'ortho' ? built.ortho : built.perspective;
		const previous = built.camera;
		// Stand in the same place, so switching is a change of lens rather than a jump.
		next.position.copy(previous.position);
		next.quaternion.copy(previous.quaternion);
		built.projection = projection;
		built.camera = next;
		built.controls.camera = next;
		built.gizmo.camera = next;
		if (built.chain) built.chain.render.camera = next;
		refitOrtho(built);
		markDirty();
	}

	/**
	 * Frame what is selected, from where the camera already is.
	 *
	 * The more used of the two, and the one that has to leave the angle alone: an
	 * operator who picked three heads wants those three heads, not a view swung round
	 * to the front of them.
	 */
	export function frameSelection() {
		const built = scene;
		if (!built) return;
		const chosen = placed.filter((f) => $selected.has(f.id));
		if (chosen.length === 0) return;
		const from = new THREE.Vector3();
		built.camera.getWorldPosition(from);
		// The selection and nothing else: no pieces of the drawing, and no floor.
		const box = rigBounds(chosen, $objectsById, { pieces: [], margin: 0.6, toFloor: false });
		take(focusShot(box, from, aspect()));
	}

	function take(shot: Shot) {
		const [px, py, pz] = shot.position;
		const [tx, ty, tz] = shot.target;
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

	// ── Taking hold of the drawing ──────────────────────────────────────────────
	//
	// Objects have their own selection store beside the fixtures', and that is the
	// decision the whole editor rests on: a `SelectionQuery` is a question about the
	// *rig*, and `at 50` means the fixtures it answers. A truss in that scope would be
	// a truss somebody could put at fifty percent by accident.

	/** What is selected in the drawing, as rows. */
	const heldObjects = $derived(
		[...$selectedObjects]
			.map((id) => $objectsById.get(id))
			.filter((object): object is SceneObject => !!object)
	);
	/** And of those, the ones an operator may actually move. */
	const movable = $derived(heldObjects.filter((object) => !isLocked(object, $layerRows)));
	const pivotAt = $derived(pivotPoint($pivot, $selectedObjects, $objectsById));

	/**
	 * A drag in flight: where the pivot was when it started, and where everything
	 * selected was.
	 *
	 * Recorded once rather than read per frame, because what each frame applies is the
	 * pivot's **delta** — and a delta measured against last frame would accumulate
	 * rounding across a two-second drag, which shows up as a truss that ends a rotation
	 * a degree away from where the readout says.
	 */
	let dragging: {
		pivot: THREE.Matrix4;
		objects: { id: string; world: THREE.Matrix4 }[];
	} | null = null;
	let pendingMove = 0;

	const scratchDelta = new THREE.Matrix4();
	const scratchWorld = new THREE.Matrix4();
	const scratchPos = new THREE.Vector3();
	const scratchQuatB = new THREE.Quaternion();
	const scratchSize = new THREE.Vector3();
	const scratchEuler = new THREE.Euler();

	/** A three.js matrix as the placement the show stores. */
	function transformOf(matrix: THREE.Matrix4): Transform {
		matrix.decompose(scratchPos, scratchQuatB, scratchSize);
		scratchEuler.setFromQuaternion(scratchQuatB, 'XYZ');
		const degrees = 180 / Math.PI;
		return {
			position: { x: scratchPos.x, y: scratchPos.y, z: scratchPos.z },
			rotation: {
				x: scratchEuler.x * degrees,
				y: scratchEuler.y * degrees,
				z: scratchEuler.z * degrees
			},
			// three's own decomposition puts a reflection on X, which is exactly where
			// this console keeps one. A mirrored truss stays mirrored through a drag.
			scale: { x: scratchSize.x, y: scratchSize.y, z: scratchSize.z }
		};
	}

	/** And the other way, for recording where something was when a drag began. */
	function matrixOf(transform: Transform): THREE.Matrix4 {
		const radians = Math.PI / 180;
		return new THREE.Matrix4().compose(
			new THREE.Vector3(transform.position.x, transform.position.y, transform.position.z),
			new THREE.Quaternion().setFromEuler(
				new THREE.Euler(
					transform.rotation.x * radians,
					transform.rotation.y * radians,
					transform.rotation.z * radians,
					'XYZ'
				)
			),
			new THREE.Vector3(transform.scale.x, transform.scale.y, transform.scale.z)
		);
	}

	// Where the gizmo sits, and whether it is offered at all. Not while a drag is in
	// flight: the objects are moving under it, and repositioning the pivot from their
	// new centre mid-drag would have the handle running away from the pointer.
	$effect(() => {
		const built = scene;
		const at = pivotAt;
		const mode = gizmoMode;
		const grid = $view.grid;
		if (!built || dragging) return;
		const offered = movable.length > 0 && at !== null;
		built.gizmo.enabled = offered;
		built.gizmoRoot.visible = offered;
		if (!offered) {
			markDirty();
			return;
		}
		built.pivotNode.position.set(at.x, at.y, at.z);
		built.pivotNode.rotation.set(0, 0, 0);
		built.pivotNode.scale.set(1, 1, 1);
		built.pivotNode.updateMatrixWorld(true);
		built.gizmo.setMode(mode);
		// The grid is what a rig is set out in, and Alt is how somebody says they mean
		// 1.37 m. `null` is three's own way of saying "no snapping at all".
		built.gizmo.setTranslationSnap(grid > 0 ? grid : null);
		built.gizmo.setRotationSnap(grid > 0 ? Math.PI / 12 : null);
		built.gizmo.setScaleSnap(grid > 0 ? 0.1 : null);
		markDirty();
	});

	// The gizmo's own events, wired once per panel.
	$effect(() => {
		const built = scene;
		if (!built) return;
		const started = (event: { value: boolean }) => {
			// A gizmo drag and a camera drag cannot share a pointer.
			built.controls.enabled = !event.value;
			if (event.value) beginDrag(built);
			else finishDrag();
		};
		const changed = () => applyDrag(built);
		built.gizmo.addEventListener('dragging-changed', started as never);
		built.gizmo.addEventListener('objectChange', changed as never);
		return () => {
			built.gizmo.removeEventListener('dragging-changed', started as never);
			built.gizmo.removeEventListener('objectChange', changed as never);
		};
	});

	function beginDrag(built: Scene) {
		built.pivotNode.updateMatrixWorld(true);
		dragging = {
			pivot: built.pivotNode.matrixWorld.clone().invert(),
			objects: movable.map((object) => ({
				id: object.id,
				world: matrixOf(worldTransform(object.transform, object.parent, $objectsById))
			}))
		};
		// Moving a truss is one act however many frames it took, and one Ctrl-Z.
		// `crates/pult-backend/tests/counts.rs` asserts that from the other side.
		beginGesture();
	}

	function applyDrag(built: Scene) {
		if (!dragging) return;
		// Coalesced to one write per animation frame. `objectChange` fires per pointer
		// event, which on a trackpad is well over a hundred a second, and a write per
		// one of those is a socket doing nothing else for the length of the drag.
		if (pendingMove) return;
		pendingMove = requestAnimationFrame(() => {
			pendingMove = 0;
			if (!dragging) return;
			built.pivotNode.updateMatrixWorld(true);
			scratchDelta.multiplyMatrices(built.pivotNode.matrixWorld, dragging.pivot);
			void moveObjects(
				dragging.objects.map(({ id, world }) => ({
					id,
					world: transformOf(scratchWorld.multiplyMatrices(scratchDelta, world))
				}))
			);
			markDirty();
		});
	}

	function finishDrag() {
		dragging = null;
		if (pendingMove) cancelAnimationFrame(pendingMove);
		pendingMove = 0;
		endGesture();
		// The pivot stays where the operator left it rather than snapping back to the
		// selection's new centre: they put it there, and a turn is very often two turns.
		const at = scene?.pivotNode.position;
		if (at && gizmoMode === 'rotate') pivot.set({ x: at.x, y: at.y, z: at.z });
	}

	// ── Dragging a gizmo ────────────────────────────────────────────────────────

	/**
	 * Which gizmo, on which fixture — all a hover or a hit test needs to say.
	 *
	 * `pan`, `tilt` and `spot` aim a head. `slide` and `roll` are the two degrees a
	 * *clamp* has: where along the bar, and how far round the chord. A mounted fixture
	 * gets those two rather than the six a free placement would need, and every one of
	 * the other four would take the light off the truss.
	 */
	type Handle = { kind: 'pan' | 'tilt' | 'spot' | 'slide' | 'roll'; id: string };

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
	/** The right-click menu on a `+`: where it is in the panel, and which joint. */
	let jointMenu = $state<{ x: number; y: number; joint: PlacedConnector } | null>(null);
	/** What a piece dragged from the Pieces sheet would land on, while it is in flight. */
	let dropping = $state<{ x: number; y: number } | null>(null);

	const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
	const caster = new THREE.Raycaster();

	const PAN_GEOMETRY = new THREE.TorusGeometry(0.46, 0.035, 8, 48);
	const TILT_GEOMETRY = new THREE.TorusGeometry(0.36, 0.035, 8, 32, Math.PI);
	const SPOT_GEOMETRY = new THREE.CircleGeometry(0.4, 32);
	/** What a clamp's two handles are: somewhere to grab, and a ring to turn. */
	const SLIDE_GEOMETRY = new THREE.SphereGeometry(0.09, 12, 8);
	const ROLL_GEOMETRY = new THREE.TorusGeometry(0.24, 0.025, 8, 28);
	/** And the `+` on a free joint. A ball, so it reads from any angle. */
	const JOINT_GEOMETRY = new THREE.SphereGeometry(0.075, 12, 8);
	/** How far past the joint it sits, in metres. */
	const JOINT_STANDOFF = 0.18;

	// ── What a light is clamped to ──────────────────────────────────────────────

	/**
	 * The piece a fixture hangs off, and where its chords are.
	 *
	 * A catalogue piece declares them. Anything else — a truss out of somebody's
	 * drawing — gets **one**, off the mesh's bounds, and that is the smallest guess
	 * available on purpose: the console does not know whether that mesh is a box truss
	 * or a ladder, and four invented chords would put lights on corners that are not
	 * there. Only this side ever measures a mesh, which is why the browser writes every
	 * mount.
	 */
	function clampable(id: string | null): { object: SceneObject; chords: Chord[]; world: Transform } | null {
		if (!id) return null;
		const object = $objectsById.get(id);
		if (!object) return null;
		const entry = piece(object.catalogue);
		const measured = object.geometry[0] ? $meshSizes.get(object.geometry[0].asset) : undefined;
		const chords = chordsFor(
			entry,
			measured ? { x: measured.x, y: measured.y, z: measured.z } : null
		);
		if (chords.length === 0) return null;
		return { object, chords, world: worldTransform(object.transform, object.parent, $objectsById) };
	}

	/** Every piece in the drawing a light could be clamped to, with its chords. */
	const clamps = $derived(
		$visibleObjects
			.map((object) => clampable(object.id))
			.filter((each): each is NonNullable<typeof each> => !!each)
	);

	/**
	 * The nearest clamp to a point in the room, if anything is near enough.
	 *
	 * Every candidate's chords are asked in its own frame — a light is clamped to a bar
	 * whichever way the bar has been turned — so the point goes down through
	 * `localOf` and the answer comes back up through the piece's own placement.
	 */
	function nearestClamp(
		world: Vec3
	): { parent: string; mount: Mount; local: Transform } | null {
		let best: { parent: string; mount: Mount; local: Transform; away: number } | null = null;
		for (const candidate of clamps) {
			const local = localOf({ ...IDENTITY, position: world }, candidate.world);
			const { mount, distance } = nearestMount(local.position, candidate.chords);
			if (distance > SNAP_RADIUS) continue;
			if (best && distance >= best.away) continue;
			best = {
				parent: candidate.object.id,
				mount,
				local: { ...IDENTITY, position: mountPoint(mount, candidate.chords) },
				away: distance
			};
		}
		return best;
	}

	/** The chords of whatever this fixture is currently clamped to. */
	function clampOf(fixture: Fixture) {
		return fixture.mount ? clampable(fixture.parent) : null;
	}

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

			// The clamp's two degrees, on a fixture that has one. `slide` is a free
			// drag that re-clamps wherever it lands — which is also how a light comes
			// *off* a bar, by being dragged out of the snap radius — and `roll` is the
			// quarter turns a hook clamp actually has.
			const clamp = clampOf(beam.fixture);
			if (clamp) {
				const grip = add('slide', SLIDE_GEOMETRY, 0xf59e0b);
				grip.position.set(beam.at.x, beam.at.y + 0.22, beam.at.z);

				const ring = add('roll', ROLL_GEOMETRY, 0x22c55e);
				ring.position.set(beam.at.x, beam.at.y, beam.at.z);
				// About the chord, which runs along the parent's own X.
				const turn = new THREE.Euler(
					(clamp.world.rotation.x * Math.PI) / 180,
					(clamp.world.rotation.y * Math.PI) / 180,
					(clamp.world.rotation.z * Math.PI) / 180,
					'XYZ'
				);
				ring.quaternion.setFromEuler(turn);
				ring.rotateY(Math.PI / 2);
			}
		}

		// ── The `+` on every free joint ─────────────────────────────────────────
		//
		// Free is worked out from the geometry rather than from a field, so a run of
		// four sections offers a handle at each end and nowhere in the middle — and
		// goes on being right when somebody deletes a section out of the middle.
		built.handleGroup.clear();
		built.handles.clear();
		if (!dragging) {
			for (const joint of myFreeJoints) {
				const knob = new THREE.Mesh(
					JOINT_GEOMETRY,
					new THREE.MeshBasicMaterial({ color: 0x4a9eff, depthTest: false })
				);
				knob.renderOrder = 11;
				// Set out past the joint along the way it faces — 180 mm, which is
				// outside the piece. It reads as "the next one goes here" rather than
				// as a dot on the end, and it clears the move gizmo's own arm, which
				// points the same way and is scaled in screen terms.
				knob.position.set(
					joint.at.x + joint.facing.x * JOINT_STANDOFF,
					joint.at.y + joint.facing.y * JOINT_STANDOFF,
					joint.at.z + joint.facing.z * JOINT_STANDOFF
				);
				built.handleGroup.add(knob);
				built.handles.set(knob, joint);
			}
		}

		// ── The outline round what is selected ──────────────────────────────────
		//
		// A truss is a line of tubes and a selected one is a slightly bluer line of
		// tubes: nothing in the picture says where it *ends*, which is exactly what
		// somebody about to drag it needs to know. So the selection is drawn as the
		// box it occupies — the same box the pointer finds it by, so the extent is
		// measured once and the outline cannot disagree with the hit test.
		//
		// `depthTest: false`, because an outline you cannot see through the truss it
		// is round is an outline that vanishes at the angle you are working at.
		built.outlineGroup.clear();
		for (const id of $selectedObjects) {
			const box = built.picks.get(id);
			if (!box) continue;
			const edges = new THREE.LineSegments(new THREE.EdgesGeometry(box.geometry), OUTLINE);
			// Straight off the box's own world matrix rather than re-composed: the box
			// is a child of the object's group, so this follows a parented piece through
			// its whole chain with no arithmetic here at all.
			edges.matrixAutoUpdate = false;
			box.updateWorldMatrix(true, false);
			edges.matrix.copy(box.matrixWorld);
			edges.renderOrder = 9;
			built.outlineGroup.add(edges);
		}

		// Their world matrices, now rather than at the next render.
		//
		// Everything above is built fresh each tick, and a fresh `Object3D` has an
		// identity `matrixWorld` until something updates it — which is normally the
		// renderer. But this view draws only when something changed, so on a settled
		// rig no frame follows, and a raycast against a handle then finds it at the
		// origin instead of where it is drawn. Which is a handle that is visibly there
		// and cannot be pressed.
		built.gizmoGroup.updateMatrixWorld(true);
		built.handleGroup.updateMatrixWorld(true);
		built.outlineGroup.updateMatrixWorld(true);
	}

	/**
	 * The free joints of the one selected piece.
	 *
	 * Derived rather than worked out per frame, and that is not tidiness: finding a
	 * free joint means composing every visible piece's placement and comparing every
	 * joint against every other, which on a festival rig is a few thousand
	 * multiplications. Per frame it would be the frame budget; as a derived it happens
	 * when the rig changes, which is when the answer can differ.
	 *
	 * One piece only, because a `+` per end of forty selected sections is a screen full
	 * of dots rather than an offer.
	 */
	const myFreeJoints = $derived.by((): PlacedConnector[] => {
		if (movable.length !== 1) return [];
		const object = movable[0];
		const everything = $visibleObjects.flatMap((each) =>
			connectorsOf(each, worldTransform(each.transform, each.parent, $objectsById))
		);
		return freeConnectors(everything).filter((joint) => joint.object === object.id);
	});

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
			// A `+` on a free joint: ask what goes on there.
			//
			// **Before** the gizmo, because the two overlap: a joint is at the end of a
			// piece and the move gizmo's arrow points that way, and the arrow is scaled
			// in screen terms so it reaches the end at some zooms and past it at others.
			// The knob is small and the raycast against it is exact, so letting it win
			// costs the arrow only the few pixels the knob covers — and the knob is set
			// out past the joint anyway, where the next piece would go.
			const joint = jointAt(built, event);
			if (joint) {
				event.stopPropagation();
				event.preventDefault();
				openJointMenu(built, joint, event);
				return;
			}
			// The drawing's gizmo has the pointer: leave the press entirely alone.
			//
			// This runs in the *capture* phase on the element the canvas sits in, so it
			// sees every press before `TransformControls` — which listens on the canvas
			// itself — and a `stopPropagation` here means the gizmo never hears its own
			// drag. What that looked like was an arrow that highlighted, could be
			// pressed, and moved nothing; and worse, a press on an arrow with nothing
			// solid behind it fell through to "empty space", which cleared the
			// selection and took the gizmo away mid-grab.
			//
			// `axis` is the gizmo's own hover state, set from the pointermove before
			// this press, so this asks the thing that knows rather than raycasting the
			// handles a second time.
			if (built.gizmo.axis !== null) return;
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
				else {
					select(body);
					clearObjects();
				}
				return;
			}
			// Or a piece of the drawing. Second, because a light hanging under a bar is
			// in front of it and is what somebody clicking there means.
			const object = objectAt(built, event);
			if (object) {
				event.stopPropagation();
				if (event.shiftKey) toggleObject(object);
				else {
					selectObject(object);
					clearSelection();
				}
				return;
			}
			// Empty space lets go of both, which is the only gesture that can: the two
			// selections are separate and something has to be able to clear them.
			if (!event.shiftKey) {
				clearSelection();
				clearObjects();
			}
		};

		// And the right button on one does the same, since that is where a person
		// looks for a menu — and it stops the browser's own from covering this one.
		const menu = (event: MouseEvent) => {
			if (!(event.target instanceof HTMLCanvasElement)) return;
			const joint = jointAt(built, event as unknown as PointerEvent);
			if (!joint) return;
			event.preventDefault();
			event.stopPropagation();
			openJointMenu(built, joint, event);
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
		element.addEventListener('contextmenu', menu, { capture: true });
		element.addEventListener('wheel', interrupt, { passive: true });
		return () => {
			element.removeEventListener('pointerdown', press, { capture: true });
			element.removeEventListener('pointermove', move);
			element.removeEventListener('pointerdown', interrupt);
			element.removeEventListener('contextmenu', menu, { capture: true });
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

	/**
	 * Which piece of the drawing is under the pointer.
	 *
	 * Recursive, because a hit lands on a mesh several levels down inside a loaded
	 * `.glb`; the object id is stamped on every node on the way in, so the answer is a
	 * lookup rather than a walk back up the tree per pointer move.
	 *
	 * A **locked** piece still answers. Lock takes away the gizmo, not the ability to
	 * click on something and find out what it is — a piece you cannot select is a piece
	 * whose name and layer you cannot read, which is not what anybody meant by locking
	 * the house rig.
	 */
	function objectAt(built: Scene, event: PointerEvent): string | null {
		const ndc = pointerNdc(event);
		if (!ndc) return null;
		caster.setFromCamera(ndc, built.camera);
		const hit = caster.intersectObjects([...built.objects.values()], true)[0];
		return (hit?.object.userData.objectId as string) ?? null;
	}

	function jointAt(built: Scene, event: PointerEvent): PlacedConnector | null {
		const ndc = pointerNdc(event);
		if (!ndc || built.handles.size === 0) return null;
		caster.setFromCamera(ndc, built.camera);
		const hit = caster.intersectObjects([...built.handles.keys()], false)[0];
		return hit ? (built.handles.get(hit.object) ?? null) : null;
	}

	/** Whatever the joint's own piece is — the menu's first answer. */
	function sameKindAs(joint: PlacedConnector): string {
		const object = $objectsById.get(joint.object);
		return object?.catalogue ?? 'f34-2m';
	}

	/**
	 * What the menu offers: every catalogue piece with a joint of this kind, the one
	 * that is already there first.
	 *
	 * Same-kind is the catalogue's own rule and it is what keeps the list short and
	 * true — a truss end takes a truss, a corner or a plate, and a pipe end takes a
	 * pipe. Putting the piece that is already there at the top is what makes laying a
	 * run one press and a row of Enters, and it is also the answer that is right nine
	 * times in ten.
	 */
	function piecesFor(joint: PlacedConnector) {
		const same = sameKindAs(joint);
		const all = CATALOGUE.filter((entry) =>
			entry.connectors.some((connector) => connector.kind === joint.kind)
		);
		return [
			...all.filter((entry) => entry.id === same),
			...all.filter((entry) => entry.id !== same)
		];
	}

	/**
	 * Put the menu under the pointer, in the panel's own coordinates.
	 *
	 * Kept inside the tile, because a joint at the right-hand end of a run is exactly
	 * where somebody presses one and the menu opening off the edge is the menu being
	 * cut in half. Clamped rather than flipped: it stays under the pointer, which is
	 * where the eye already is.
	 */
	function openJointMenu(built: Scene, joint: PlacedConnector, event: { clientX: number; clientY: number }) {
		const rect = built.renderer.domElement.getBoundingClientRect();
		const room = { x: MENU_SIZE.width, y: MENU_SIZE.height };
		jointMenu = {
			x: Math.max(0, Math.min(event.clientX - rect.left, rect.width - room.x)),
			y: Math.max(0, Math.min(event.clientY - rect.top, rect.height - room.y)),
			joint
		};
	}

	/**
	 * About how big the menu is, for keeping it on screen.
	 *
	 * A guess rather than a measurement, and it can be: it is only ever used to stop
	 * the menu hanging off an edge, so being a few pixels out moves it a few pixels.
	 * Measuring would mean rendering it once to find out where to render it.
	 */
	const MENU_SIZE = { width: 170, height: 200 };

	async function addOnJoint(joint: PlacedConnector, catalogueId: string) {
		jointMenu = null;
		const world = placedOnConnector(catalogueId, joint);
		if (!world) return;
		const onto = $objectsById.get(joint.object);
		// Parented to whatever the piece it bolts to hangs off, so a run built one
		// section at a time stays one run: dragging the group moves the whole thing.
		const parent = onto?.parent ?? null;
		const local = localOf(world, parentWorld(parent, $objectsById));
		const id = await placePiece(catalogueId, local, { parent });
		if (id) selectObject(id);
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
		if (kind === 'slide' || kind === 'roll') {
			// Nothing to read off first: a clamp drag works from where the pointer is
			// now, against the bar it is on.
			start.from = beam.fixture.mount?.roll ?? 0;
		} else if (kind === 'spot') {
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

		// ── The clamp ───────────────────────────────────────────────────────────
		if (grab.kind === 'slide' || grab.kind === 'roll') {
			dragClamp(grab.kind, beam.fixture, beam.at, ray);
			return;
		}

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

	/**
	 * The two degrees a clamp has.
	 *
	 * `slide` is a free drag in the horizontal plane the light is already at, and every
	 * frame of it asks the whole drawing what the nearest clamp is: inside the radius
	 * the light hangs off that bar at that point along it, and outside it the light is
	 * simply somewhere, un-parented and keeping the place it had reached. Which is what
	 * makes "drag it away and it comes off" a gesture rather than a menu item.
	 *
	 * `roll` turns about the chord and lands on a quarter, because that is what a hook
	 * clamp does: hanging, standing on top, or on either face.
	 */
	function dragClamp(kind: 'slide' | 'roll', fixture: Fixture, at: Vec3, ray: Ray) {
		if (kind === 'roll') {
			const clamp = clampOf(fixture);
			if (!clamp || !fixture.mount) return;
			// Read in the plane the chord turns in, which is across the parent's own X.
			const across = localOf({ ...IDENTITY, position: at }, clamp.world);
			const point = rayOnPlane(ray, at, chordAxis(clamp.world));
			if (!point) return;
			const local = localOf({ ...IDENTITY, position: point }, clamp.world);
			const chord = clamp.chords[fixture.mount.chord % clamp.chords.length];
			const roll = quarter(local.position.y - chord.at.y, local.position.z - chord.at.z);
			if (roll === fixture.mount.roll) return;
			const mount = { ...fixture.mount, roll };
			void clampFixture(fixture.id, fixture.parent, mount, {
				...IDENTITY,
				position: mountPoint(mount, clamp.chords),
				rotation: { x: roll, y: 0, z: 0 }
			});
			void across;
			return;
		}

		// Slide: the pointer on the horizontal plane through the light.
		const point = rayOnPlane(ray, at, { x: 0, y: 1, z: 0 });
		if (!point) return;
		const wanted = { x: point.x, y: at.y, z: point.z };
		const found = nearestClamp(wanted);
		if (found) {
			if (
				fixture.parent === found.parent &&
				fixture.mount &&
				Math.abs(fixture.mount.along - found.mount.along) < 1e-4 &&
				fixture.mount.chord === found.mount.chord
			) {
				return;
			}
			// The roll an operator already chose is kept: sliding a light along a bar
			// must not stand it back up the other way.
			const mount = { ...found.mount, roll: fixture.mount?.roll ?? found.mount.roll };
			const clamp = clampable(found.parent);
			void clampFixture(fixture.id, found.parent, mount, {
				...IDENTITY,
				position: clamp ? mountPoint(mount, clamp.chords) : found.local.position,
				rotation: { x: mount.roll, y: 0, z: 0 }
			});
			return;
		}
		// Out of every radius: off the truss, and left exactly where it got to.
		if (fixture.mount === null && fixture.parent === null) {
			void moveFixtures([{ id: fixture.id, world: { ...IDENTITY, position: wanted } }]);
			return;
		}
		void clampFixture(fixture.id, null, null, { ...IDENTITY, position: wanted });
	}

	/** The direction a piece's chords run: its own X, turned into the world. */
	function chordAxis(world: Transform): Vec3 {
		const radians = Math.PI / 180;
		const axis = new THREE.Vector3(1, 0, 0).applyEuler(
			new THREE.Euler(
				world.rotation.x * radians,
				world.rotation.y * radians,
				world.rotation.z * radians,
				'XYZ'
			)
		);
		return { x: axis.x, y: axis.y, z: axis.z };
	}

	/** Which quarter turn points at `(dy, dz)`. The browser's half of `Mount::nearest`. */
	function quarter(dy: number, dz: number): number {
		const angle = (Math.atan2(-dz, -dy) * 180) / Math.PI;
		return ((Math.round(angle / 90) * 90) % 360 + 360) % 360;
	}

	// ── Dropping a piece in from the sheet ──────────────────────────────────────

	/**
	 * Where a piece dragged onto the canvas lands: the **work plane**.
	 *
	 * A pointer is a ray and a room is three-dimensional, so something has to say how
	 * far away. A horizontal plane at the sheet's own work height is the answer for
	 * every view that can see the floor; a view looking *along* the floor sees a
	 * horizontal plane edge-on and would catch nothing, so those get a vertical one at
	 * the work depth instead. Which of the two is decided by where the camera is
	 * standing rather than by the preset, because somebody may have orbited.
	 */
	function onWorkPlane(built: Scene, event: { clientX: number; clientY: number }): Vec3 | null {
		const ndc = pointerNdc(event as PointerEvent);
		if (!ndc) return null;
		caster.setFromCamera(ndc, built.camera);
		const ray: Ray = {
			origin: { ...caster.ray.origin },
			direction: { ...caster.ray.direction }
		};
		const level = Math.abs(ray.direction.y) > 0.15;
		const point = level
			? rayOnPlane(ray, { x: 0, y: $view.workHeight, z: 0 }, { x: 0, y: 1, z: 0 })
			: rayOnPlane(ray, { x: 0, y: 0, z: $view.workDepth }, { x: 0, y: 0, z: 1 });
		return point ? pointToGrid(point, $view.grid) : null;
	}

	/**
	 * A piece dropped on the canvas.
	 *
	 * Grid first, then the connectors: a section dropped near the end of a run bolts to
	 * it, and one dropped in the middle of the room lands on the half-metre. Holding
	 * Alt turns the grid off for the one time in twenty when somebody means 1.37 m.
	 */
	async function dropPiece(built: Scene, catalogueId: string, event: DragEvent) {
		const landed = event.altKey
			? rawWorkPlane(built, event)
			: onWorkPlane(built, event);
		if (!landed) return;
		let world: Transform = { ...IDENTITY, position: landed };
		const entry = piece(catalogueId);
		if (entry) {
			const mine = entry.connectors.map((connector, index) => ({
				object: '',
				index,
				at: {
					x: connector.at.x + landed.x,
					y: connector.at.y + landed.y,
					z: connector.at.z + landed.z
				},
				facing: connector.facing,
				kind: connector.kind
			}));
			const theirs = freeConnectors(
				$visibleObjects.flatMap((each) =>
					connectorsOf(each, worldTransform(each.transform, each.parent, $objectsById))
				)
			);
			world = snapConnectors(world, mine, theirs)?.transform ?? world;
		}
		const id = await placePiece(catalogueId, world);
		if (id) selectObject(id);
	}

	function rawWorkPlane(built: Scene, event: { clientX: number; clientY: number }): Vec3 | null {
		const ndc = pointerNdc(event as PointerEvent);
		if (!ndc) return null;
		caster.setFromCamera(ndc, built.camera);
		const ray: Ray = {
			origin: { ...caster.ray.origin },
			direction: { ...caster.ray.direction }
		};
		return Math.abs(ray.direction.y) > 0.15
			? rayOnPlane(ray, { x: 0, y: $view.workHeight, z: 0 }, { x: 0, y: 1, z: 0 })
			: rayOnPlane(ray, { x: 0, y: 0, z: $view.workDepth }, { x: 0, y: 0, z: 1 });
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

<!-- The canvas is also where a piece from the Pieces sheet lands. `dragover` has to
     be cancelled or the browser refuses the drop, and the piece's id travels as plain
     text because that is what a native drag carries. -->
<div
	class="viewport"
	class:dropping={dropping !== null}
	bind:this={host}
	role="presentation"
	ondragover={(event) => {
		if (!event.dataTransfer?.types.includes('text/plain')) return;
		event.preventDefault();
		event.dataTransfer.dropEffect = 'copy';
		dropping = { x: event.offsetX, y: event.offsetY };
	}}
	ondragleave={() => (dropping = null)}
	ondrop={(event) => {
		event.preventDefault();
		dropping = null;
		const id = event.dataTransfer?.getData('text/plain');
		if (id && scene) void dropPiece(scene, id, event);
	}}
></div>

{#if jointMenu}
	<!-- What goes on this joint. Same-kind only, which is the catalogue's own rule: a
	     truss end takes a truss, a corner or a plate, and a pipe end takes a pipe. The
	     piece already there is first and holds the focus, so laying a run is one press
	     and a row of Enters; Escape and the pointer leaving both put it away. -->
	<div
		class="menu"
		role="menu"
		tabindex="-1"
		style:left="{jointMenu.x}px"
		style:top="{jointMenu.y}px"
		onpointerdown={(e) => e.stopPropagation()}
		onmouseleave={() => (jointMenu = null)}
		onkeydown={(e) => e.key === 'Escape' && (jointMenu = null)}
	>
		{#each piecesFor(jointMenu.joint) as entry, n (entry.id)}
			<button
				role="menuitem"
				class:first={n === 0}
				{@attach (node: HTMLButtonElement) => {
					if (n === 0) node.focus();
				}}
				onclick={() => jointMenu && void addOnJoint(jointMenu.joint, entry.id)}
			>
				<span>{entry.title}</span>
				{#if n === 0}<span class="again">again</span>{/if}
			</button>
		{/each}
	</div>
{/if}

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
	/* A piece is in flight over the canvas: say so, because a drag that lands on
	   nothing and a drag that lands are otherwise the same picture. */
	.viewport.dropping {
		outline: 1px dashed var(--line-input, #4a9eff);
		outline-offset: -3px;
	}

	.menu {
		position: absolute;
		z-index: 30;
		display: flex;
		flex-direction: column;
		min-width: 160px;
		padding: 3px;
		border: 1px solid var(--line-strong, #333);
		border-radius: 4px;
		background: #1a1a1a;
		box-shadow: 0 6px 20px rgb(0 0 0 / 55%);
	}
	.menu button {
		display: flex;
		align-items: baseline;
		gap: 10px;
		background: none;
		border: 0;
		color: #ccc;
		padding: 5px 9px;
		font: inherit;
		font-size: 12px;
		text-align: left;
		cursor: pointer;
		border-radius: 3px;
	}
	.menu button:hover,
	.menu button:focus-visible {
		background: #2a2f3a;
		color: #fff;
		outline: none;
	}
	.menu button span:first-child { flex: 1; }
	.menu button.first { color: #fff; }
	.again { color: #666; font-size: 11px; }

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
