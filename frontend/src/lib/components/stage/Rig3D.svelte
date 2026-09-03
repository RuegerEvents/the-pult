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
	import { beamGeometry, beamMaterial, dimKeepingHue, strobeGate, LENS_RADIUS } from '$lib/beam.js';
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
	import { worldTransform } from '$lib/scene.js';
	import { setValue } from '$lib/stores/programmer.js';
	import { output as showing, watching } from '$lib/stores/output.js';
	import { parameterKey } from '$lib/patch.js';
	import Quicksheet from '$lib/components/programmer/Quicksheet.svelte';
	import { beginGesture, endGesture } from '$lib/stores/gesture.js';

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
	// number: nothing about drawing a rig reaches a lamp. A rolling mean over a
	// second, so it reads as a number rather than a flicker.
	let frameMs = $state(0);
	export function costMs(): number {
		return frameMs;
	}

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
	let scene = $state<Scene | null>(null);

	/// Everything the renderer owns, built once per panel and torn down with it.
	function buildScene(element: HTMLDivElement) {
		const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
		renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
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
		// light in it.
		root.add(new THREE.AmbientLight(0x5a6478, 0.5));
		root.add(new THREE.HemisphereLight(0xffffff, 0x101010, 0.3));

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
				uniforms: { uNear: { value: 0 }, uFar: { value: 160 } },
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
						gl_FragColor = vec4(vec3(0.42), strength);
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
		const beamMesh = new THREE.InstancedMesh(beamGeometry(), beamMat, 1);
		beamMesh.frustumCulled = false;
		beamMesh.count = 0;
		root.add(beamMesh);

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
		const resize = new ResizeObserver(() => {
			const { clientWidth, clientHeight } = element;
			if (clientWidth < 1 || clientHeight < 1) return;
			built.renderer.setSize(clientWidth, clientHeight, false);
			built.camera.aspect = clientWidth / clientHeight;
			built.camera.updateProjectionMatrix();
		});
		resize.observe(element);

		const [px, py, pz] = home.position;
		const [tx, ty, tz] = home.target;
		built.controls.setLookAt(px, py, pz, tx, ty, tz, false);

		let running = true;
		let previous = performance.now();
		let frames = 0;
		let elapsed = 0;
		// Our own elapsed seconds rather than `THREE.Clock`, which is deprecated, and
		// rather than `performance.now()` directly: the shader wants seconds since
		// this panel opened, so two rig panels drift independently instead of both
		// hazing off one enormous number where float precision has run out.
		let seconds = 0;

		const tick = () => {
			if (!running) return;
			requestAnimationFrame(tick);
			const now = performance.now();
			const delta = (now - previous) / 1000;
			previous = now;

			// The gap between frames, not the work inside one: a page served a frame
			// every 200 ms is stuttering however cheap its own work was.
			frames += 1;
			elapsed += delta;
			if (elapsed >= 1) {
				frameMs = (elapsed * 1000) / frames;
				frames = 0;
				elapsed = 0;
			}

			seconds += delta;
			built.controls.update(delta);
			draw(built, seconds);
			built.renderer.render(built.root, built.camera);
		};
		requestAnimationFrame(tick);

		return () => {
			running = false;
			resize.disconnect();
			built.controls.dispose();
			built.beamMesh.geometry.dispose();
			built.beamMat.dispose();
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

	function draw(built: Scene, seconds: number) {
		const list = beams;

		// Haze, from the show. It reaches no lamp, and it is show data because how
		// hazy the room is is a fact about the room rather than about the screen
		// looking at it.
		built.beamMat.uniforms.uTime.value = seconds;
		built.beamMat.uniforms.uHazeDensity.value = show?.haze_density ?? 0.35;
		built.beamMat.uniforms.uHazeTurbulence.value = show?.haze_turbulence ?? 0.25;

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

		let brightest = -1;
		let brightestLevel = 0;

		for (let i = 0; i < list.length; i++) {
			const beam = list[i];
			// A shutter that is shut, and a strobe between its flashes, are both a
			// beam that is not there this instant.
			const gate = strobeGate(beam.strobe, seconds) * (beam.shutter > 0.02 ? 1 : 0);
			const level = beam.output.level * gate;

			scratchDirection.set(beam.direction.x, beam.direction.y, beam.direction.z).normalize();
			scratchQuat.setFromUnitVectors(DOWN, scratchDirection);
			scratchPosition.set(beam.at.x, beam.at.y, beam.at.z);
			scratchMatrix.compose(scratchPosition, scratchQuat, scratchScale);
			built.beamMesh.setMatrixAt(i, scratchMatrix);

			scratchColour.setRGB(beam.output.r, beam.output.g, beam.output.b);
			colours.setXYZ(i, scratchColour.r, scratchColour.g, scratchColour.b);
			levels.setX(i, level);
			// Run on past the axis's floor hit, so the floor cuts the beam rather than
			// the cone's own square end standing half in the air on a slanted throw.
			lengths.setX(i, drawnLength(beam.length, beam.direction, beam.spread, LENS_RADIUS));
			spreads.setX(i, beam.spread);

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

		built.beamMesh.count = list.length;
		built.beamMesh.instanceMatrix.needsUpdate = true;
		colours.needsUpdate = true;
		levels.needsUpdate = true;
		lengths.needsUpdate = true;
		spreads.needsUpdate = true;

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
	}

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
			place(group, object);
			const references =
				object.geometry.length > 0
					? object.geometry
					: ($symbols.find((s) => s.id === object.symbol)?.geometry ?? []);
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
