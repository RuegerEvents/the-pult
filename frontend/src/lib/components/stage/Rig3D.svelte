<script lang="ts">
	import { untrack } from 'svelte';
	import { T, useThrelte } from '@threlte/core';
	import { CameraControls, HTML, interactivity } from '@threlte/extras';
	import type CameraControlsRef from 'camera-controls';
	import * as THREE from 'three';

	import type { Fixture, FixtureType, StagePlan, Vec3 } from '$lib/generated/index.js';
	import {
		aimAt,
		beamDirection,
		fixtureOutput,
		fixturePoint,
		fohCamera,
		planExtent,
		throwDistance,
		wrapDegrees,
		PAN_TRAVEL,
		TILT_TRAVEL
	} from '$lib/stage.js';
	import {
		bearingFromPoint,
		bearingOnFloor,
		elevationFromPoint,
		rayOnPlane,
		type Ray
	} from '$lib/puppeteer.js';
	import { selected, select, toggle } from '$lib/stores/selection.js';
	import { setValue } from '$lib/stores/programmer.js';
	import Quicksheet from '$lib/components/programmer/Quicksheet.svelte';

	let {
		fixtures,
		types,
		plan,
		planUrl,
		follow = false
	}: {
		fixtures: Fixture[];
		types: FixtureType[];
		plan: StagePlan | null;
		planUrl: string | null;
		follow?: boolean;
	} = $props();

	// Without this, nothing in the scene has ever been clickable: the plugin has to be
	// installed for `onclick` on a mesh to be anything at all.
	interactivity();

	const { camera, dom } = useThrelte();

	const placed = $derived(fixtures.filter((f) => fixturePoint(f) !== null));
	const typeOf = (fixture: Fixture) => types.find((t) => t.id === fixture.fixture_type_id);

	// The opening view: worked out once, when there is first a rig to frame, and
	// then left alone. Recomputing it as fixtures move would take the camera out of
	// the operator's hands every time a cue changed something.
	let home = $state(fohCamera([]));
	let framed = false;
	$effect(() => {
		if (framed || placed.length === 0) return;
		framed = true;
		home = untrack(() => fohCamera(placed));
	});

	let controls = $state<CameraControlsRef | undefined>(undefined);

	/// Frame the opening view once, when the controls exist and there is a rig to
	/// frame. The camera's own `position` prop is only its starting point — once
	/// `CameraControls` has hold of it, moving it is asking the controls to move it.
	let opened = false;
	$effect(() => {
		if (opened || !controls || placed.length === 0) return;
		opened = true;
		const [px, py, pz] = home.position;
		const [tx, ty, tz] = home.target;
		controls.setLookAt(px, py, pz, tx, ty, tz, false);
	});

	/** Put the camera back where the view opened. */
	export function goHome() {
		const [px, py, pz] = home.position;
		const [tx, ty, tz] = home.target;
		controls?.setLookAt(px, py, pz, tx, ty, tz, true);
	}

	/// Zoom to the fixture that was just picked, which is what the spec asks
	/// programming to begin with. Only when asked for: a camera that moves on every
	/// click is unusable for anyone working from a fixed view.
	const focused = $derived($selected.size === 1 ? [...$selected][0] : null);
	$effect(() => {
		const id = focused;
		if (!follow || !id || !controls) return;
		// Only picking a different fixture may move the camera. Reading the rig here
		// would re-frame on every live-value tick — forty times a second through a
		// fade, with the view lurching each time.
		untrack(() => {
			const fixture = fixtures.find((f) => f.id === id);
			const at = fixture && fixturePoint(fixture);
			if (!at) return;
			// Stand off along the way the camera is already looking, so framing a
			// fixture turns the view towards it rather than teleporting round to the
			// other side of it.
			const from = new THREE.Vector3();
			$camera.getWorldPosition(from);
			const back = from.sub(new THREE.Vector3(at.x, at.y, at.z));
			if (back.lengthSq() < 1e-6) back.set(0, 1, 5);
			back.setLength(4.5);
			controls?.setLookAt(at.x + back.x, at.y + back.y, at.z + back.z, at.x, at.y, at.z, true);
		});
	});

	/// The plan, as a texture for the floor. Disposed when it is replaced, because
	/// a texture per plan change is a leak nobody would notice until it mattered.
	let floorTexture = $state<THREE.Texture | null>(null);
	$effect(() => {
		const url = planUrl;
		if (!url) {
			floorTexture = null;
			return;
		}
		let live = true;
		const texture = new THREE.TextureLoader().load(url, () => {
			if (!live) texture.dispose();
		});
		texture.colorSpace = THREE.SRGBColorSpace;
		floorTexture = texture;
		return () => {
			live = false;
			texture.dispose();
		};
	});

	/// Everything one fixture needs drawing: where it is, where it points, how far
	/// the beam runs and what colour it is.
	const beams = $derived(
		placed.map((fixture) => {
			const at = fixturePoint(fixture)!;
			const type = typeOf(fixture);
			const direction = beamDirection(fixture, type);
			const length = throwDistance(at, direction);
			const output = fixtureOutput(fixture);
			// Both meshes are drawn about their own Y and centred on their middle, so
			// each has to be turned to face the beam and pushed half its length along
			// it. They are turned by *opposite* ends, though:
			//
			// A cone's apex is at +Y and its base at −Y, and light leaves a lantern at
			// a point and widens on the way to the floor — so the cone's −Y has to go
			// down the beam, putting the apex at the fixture. Aligning +Y instead
			// stands the beam on its head, narrowing towards the deck.
			//
			// The body is a slightly tapered cylinder whose wider end is the lens, and
			// that end does face down the beam.
			const along = (from: THREE.Vector3) => {
				const turn = new THREE.Quaternion().setFromUnitVectors(
					from,
					new THREE.Vector3(direction.x, direction.y, direction.z)
				);
				const euler = new THREE.Euler().setFromQuaternion(turn);
				return [euler.x, euler.y, euler.z] as [number, number, number];
			};

			const bearing = bearingOnFloor(fixture, type);
			return {
				fixture,
				type,
				at,
				length,
				output,
				colour: new THREE.Color(output.r, output.g, output.b),
				rotation: along(new THREE.Vector3(0, 1, 0)),
				coneRotation: along(new THREE.Vector3(0, -1, 0)),
				/// Which way round the tilt arc has to lie: the vertical plane the head
				/// is currently panned to.
				tiltTurn: Math.atan2(-bearing.z, bearing.x),
				bearing,
				canPan: !!type?.parameters.some((p) => p.kind === 'Pan'),
				canTilt: !!type?.parameters.some((p) => p.kind === 'Tilt'),
				midpoint: [
					at.x + (direction.x * length) / 2,
					at.y + (direction.y * length) / 2,
					at.z + (direction.z * length) / 2
				] as [number, number, number],
				// Where the beam lands, which is what the spot light has to aim at.
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

	const floor = $derived(plan ? planExtent(plan) : { width: 20, depth: 14 });

	// ── Dragging a gizmo ──────────────────────────────────────────────────────

	/**
	 * A gizmo being dragged, and what it was holding when the drag began.
	 *
	 * A ring is *turned*, not aimed. So the axis moves by however far the pointer has
	 * gone round since it took hold, starting from wherever the axis already was —
	 * rather than snapping to whatever angle the pointer first landed on, which is
	 * not what taking hold of a yoke does.
	 *
	 * `turned` is added up one move at a time, each wrapped to the short way round, so
	 * a drag that goes right past the back of the fixture keeps counting instead of
	 * flipping.
	 */
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

	const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

	/// What an axis is showing now, which is where a drag of it starts from.
	function axisOf(fixture: Fixture, key: 'Pan' | 'Tilt'): number {
		const value = fixture.live_values[key];
		return value?.type === 'Float' ? value.value : 0.5;
	}
	/// Which gizmo the pointer is over, so it can light up before it is grabbed.
	let hovered = $state<Handle | null>(null);

	/// The gizmos, so the press below knows what it hit.
	const gizmos = new Map<THREE.Object3D, Handle>();
	const caster = new THREE.Raycaster();

	const gizmo = (kind: Handle['kind'], id: string) => ({
		oncreate: (ref: THREE.Object3D) => {
			gizmos.set(ref, { kind, id });
			return () => gizmos.delete(ref);
		},
		onpointerenter: () => (hovered = { kind, id }),
		onpointerleave: () => {
			if (!grab) hovered = null;
		}
	});

	/**
	 * Taking hold of a gizmo.
	 *
	 * In the *capture* phase, and this is the whole trick: the orbit controls listen
	 * for `pointerdown` on this same element, and a gizmo drag and a camera drag
	 * cannot share a pointer. Capture runs before the event reaches the canvas and
	 * therefore before the controls hear it, so stopping it there is the one place
	 * the decision can be made cleanly — rather than trying to call the camera off
	 * after it has already started moving.
	 */
	$effect(() => {
		const element = dom as HTMLElement;
		const press = (event: PointerEvent) => {
			if (grab || event.button !== 0) return;
			// A press that landed on a panel floating over the scene belongs to that
			// panel. The raycast below cannot see the panel, so it would happily find
			// a gizmo behind it and start dragging one from under a fader.
			if (!(event.target instanceof HTMLCanvasElement)) return;
			const found = gizmoAt(event);
			if (!found) return;
			event.stopPropagation();
			event.preventDefault();
			hovered = found;
			beginGrab(found.kind, found.id, event);
		};
		element.addEventListener('pointerdown', press, { capture: true });
		return () => element.removeEventListener('pointerdown', press, { capture: true });
	});

	/// Which gizmo, if any, is under the pointer.
	function gizmoAt(event: PointerEvent): Handle | null {
		const ndc = pointerNdc(event);
		if (!ndc || gizmos.size === 0) return null;
		caster.setFromCamera(ndc, $camera);
		const hit = caster.intersectObjects([...gizmos.keys()], false)[0];
		return hit ? (gizmos.get(hit.object) ?? null) : null;
	}

	/// The pointer in clip space, which is what a raycaster wants.
	function pointerNdc(event: PointerEvent): THREE.Vector2 | null {
		const rect = (dom as HTMLElement).getBoundingClientRect();
		if (rect.width < 1 || rect.height < 1) return null;
		return new THREE.Vector2(
			((event.clientX - rect.left) / rect.width) * 2 - 1,
			-((event.clientY - rect.top) / rect.height) * 2 + 1
		);
	}

	/// The pointer as a ray through the scene. The gizmos are surfaces in the room,
	/// so where the pointer *is* only means something once it has been turned back
	/// into one.
	function rayFrom(event: PointerEvent): Ray | null {
		const ndc = pointerNdc(event);
		if (!ndc) return null;
		caster.setFromCamera(ndc, $camera);
		return { origin: { ...caster.ray.origin }, direction: { ...caster.ray.direction } };
	}

	function beginGrab(kind: Handle['kind'], id: string, event: PointerEvent) {
		const beam = beams.find((b) => b.fixture.id === id);
		const ray = rayFrom(event);
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
		window.addEventListener('pointermove', onDrag);
		window.addEventListener('pointerup', endGrab, { once: true });
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
		return point ? elevationFromPoint(beam.fixture, beam.type, point) : null;
	}

	function endGrab() {
		grab = null;
		hovered = null;
		window.removeEventListener('pointermove', onDrag);
	}

	function onDrag(event: PointerEvent) {
		if (!grab) return;
		const beam = beams.find((b) => b.fixture.id === grab!.id);
		const ray = rayFrom(event);
		if (!beam || !ray) return;

		if (grab.kind !== 'spot') {
			const angle = angleUnder(grab.kind, beam, ray);
			if (angle === null) return;
			grab.turned += wrapDegrees(angle - grab.angle);
			grab.angle = angle;

			const [key, travel] =
				grab.kind === 'pan' ? (['Pan', PAN_TRAVEL] as const) : (['Tilt', TILT_TRAVEL] as const);
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
		const { pan, tilt } = aimAt(beam.fixture, beam.type, target);
		if (pan !== null) setValue([beam.fixture.id], 'Pan', { type: 'Float', value: pan });
		if (tilt !== null) setValue([beam.fixture.id], 'Tilt', { type: 'Float', value: tilt });
	}

	$effect(() => () => endGrab());

	/// Is this gizmo the one the pointer is on, or the one being dragged?
	const live = (kind: Handle['kind'], id: string) =>
		(grab ?? hovered)?.kind === kind && (grab ?? hovered)?.id === id;

	const pick = (id: string, event: { stopPropagation: () => void; nativeEvent: PointerEvent }) => {
		event.stopPropagation();
		if (event.nativeEvent.shiftKey) toggle(id);
		else select(id);
	};
</script>

<T.PerspectiveCamera makeDefault position={home.position} fov={50} near={0.1} far={400}>
	<CameraControls
		bind:ref={controls}
		maxPolarAngle={Math.PI / 2 - 0.02}
		minDistance={1.5}
		maxDistance={200}
	/>
</T.PerspectiveCamera>

<!-- Enough ambient to read the rig and the plan when nothing is on, and no more:
     this is a view of what the fixtures are doing, so the fixtures should be the
     light in it. -->
<T.AmbientLight intensity={0.5} color="#5a6478" />
<T.HemisphereLight intensity={0.3} groundColor="#101010" />

<T.Mesh
	rotation={[-Math.PI / 2, 0, 0]}
	position={[plan ? plan.origin.x + floor.width / 2 : 0, 0, plan ? plan.origin.z + floor.depth / 2 : 0]}
	receiveShadow
>
	<T.PlaneGeometry args={[floor.width, floor.depth]} />
	{#if floorTexture}
		<!-- The plan is the deck, not a lightbox: knocked back so a beam landing on
		     it is the bright thing rather than the paper. -->
		<T.MeshStandardMaterial map={floorTexture} color="#8a8a8a" roughness={1} />
	{:else}
		<T.MeshStandardMaterial color="#242424" roughness={0.95} />
	{/if}
</T.Mesh>

<!-- A metre grid to judge distance by, sunk just below the deck so it does not
     fight the plan for the same plane, and left off under the plan itself — the
     drawing is the better reference where there is one. -->
<T.GridHelper
	position={[0, -0.005, 0]}
	args={[Math.max(floor.width, floor.depth) * 1.6, Math.round(Math.max(floor.width, floor.depth) * 1.6)]}
>
	<T.MeshBasicMaterial color="#2a2a2a" />
</T.GridHelper>

{#each beams as beam (beam.fixture.id)}
	<!-- The body: what you click, and what says where the fixture hangs. -->
	<T.Mesh
		position={[beam.at.x, beam.at.y, beam.at.z]}
		rotation={beam.rotation}
		onclick={(event: { stopPropagation: () => void; nativeEvent: PointerEvent }) =>
			pick(beam.fixture.id, event)}
	>
		<T.CylinderGeometry args={[0.14, 0.11, 0.34, 16]} />
		<T.MeshStandardMaterial
			color={$selected.has(beam.fixture.id) ? '#4a9eff' : '#3a3a3a'}
			emissive={beam.colour}
			emissiveIntensity={beam.output.level * 0.9}
			roughness={0.6}
			metalness={0.3}
		/>
	</T.Mesh>

	{#if beam.output.level > 0.01}
		<!-- The beam itself. A cone of light rather than a render of one: it says
		     where the light is going and roughly how much, which is what an
		     operator is asking. -->
		<T.Mesh position={beam.midpoint} rotation={beam.coneRotation}>
			<T.ConeGeometry args={[beam.length * 0.12, beam.length, 24, 1, true]} />
			<T.MeshBasicMaterial
				color={beam.colour}
				transparent
				opacity={Math.min(0.22, beam.output.level * 0.22)}
				side={THREE.DoubleSide}
				depthWrite={false}
				blending={THREE.AdditiveBlending}
			/>
		</T.Mesh>

		<!-- The pool it lands in, so the floor shows the state as well as the air. -->
		<T.SpotLight
			position={[beam.at.x, beam.at.y, beam.at.z]}
			target.position={beam.end}
			color={beam.colour}
			intensity={beam.output.level * 14}
			angle={0.22}
			penumbra={0.45}
			distance={beam.length * 2.2}
			decay={1.1}
		/>
	{/if}
{/each}

<!-- The gizmos. Only on what is selected, because a rig of a hundred heads wearing
     rings would be a ball of wire rather than a picture of a room. -->
{#each chosen as beam (beam.fixture.id)}
	{#if beam.canPan}
		<T.Mesh
			position={[beam.at.x, beam.at.y, beam.at.z]}
			rotation={[-Math.PI / 2, 0, 0]}
			{...gizmo('pan', beam.fixture.id)}
		>
			<T.TorusGeometry args={[0.46, 0.035, 8, 48]} />
			<T.MeshBasicMaterial color={live('pan', beam.fixture.id) ? '#f59e0b' : '#4a9eff'} />
		</T.Mesh>
	{/if}

	{#if beam.canTilt}
		<T.Mesh
			position={[beam.at.x, beam.at.y, beam.at.z]}
			rotation={[0, beam.tiltTurn, 0]}
			{...gizmo('tilt', beam.fixture.id)}
		>
			<T.TorusGeometry args={[0.36, 0.035, 8, 32, Math.PI]} />
			<T.MeshBasicMaterial color={live('tilt', beam.fixture.id) ? '#f59e0b' : '#22c55e'} />
		</T.Mesh>
	{/if}

	{#if beam.canPan || beam.canTilt}
		<!-- Where the light lands, and the handle for putting it somewhere else. -->
		<T.Mesh
			position={[beam.end[0], 0.02, beam.end[2]]}
			rotation={[-Math.PI / 2, 0, 0]}
			{...gizmo('spot', beam.fixture.id)}
		>
			<T.CircleGeometry args={[0.4, 32]} />
			<T.MeshBasicMaterial
				color={live('spot', beam.fixture.id) ? '#f59e0b' : '#4a9eff'}
				side={THREE.DoubleSide}
				transparent
				opacity={0.85}
			/>
		</T.Mesh>
	{/if}
{/each}

{#if sheetFor}
	<!-- The quicksheet, at the fixture. The spec asks for programming to happen at
	     the light rather than in a panel elsewhere, and this is that, literally. -->
	<!-- `pointerEvents="none"` on the wrapper, `auto` on the sheet itself. The wrapper
	     is laid out where the sheet would be *without* its transform, so a box the
	     height of the panel sits below the panel — invisible, and eating every click
	     that lands in it, which here is the beam spot on the floor. -->
	<!-- Kept below the console's own chrome. The default range starts near the top of
	     the stacking order, which would put a panel that lives *inside* the scene over
	     the store menu and every other modal. -->
	<HTML
		position={[sheetFor.at.x, sheetFor.at.y, sheetFor.at.z]}
		pointerEvents="none"
		zIndexRange={[20, 0]}
	>
		<!-- Beside the fixture rather than over it: the rings and the beam spot are
		     the thing being worked on, and a panel sitting on top of them would hide
		     exactly what the sheet is for.
		     Pointer and wheel events stop here. The orbit controls listen on the
		     element this panel sits inside, so without it every drag of a fader would
		     also swing the camera and every scroll would dolly it. -->
		<div
			class="beside"
			role="presentation"
			onpointerdown={(e) => e.stopPropagation()}
			onpointermove={(e) => e.stopPropagation()}
			onwheel={(e) => e.stopPropagation()}
			oncontextmenu={(e) => e.stopPropagation()}
		>
			<Quicksheet fixture={sheetFor.fixture} />
		</div>
	</HTML>
{/if}

<style>
	.beside {
		transform: translate(28px, -50%);
		pointer-events: auto;
	}
</style>
