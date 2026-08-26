<script lang="ts">
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import { OrbitControls } from '@threlte/extras';
	import * as THREE from 'three';

	import type { Fixture, FixtureType, StagePlan } from '$lib/generated/index.js';
	import {
		beamDirection,
		fixtureOutput,
		fixturePoint,
		fohCamera,
		planExtent,
		throwDistance
	} from '$lib/stage.js';
	import { selected, select } from '$lib/stores/selection.js';

	let {
		fixtures,
		types,
		plan,
		planUrl
	}: {
		fixtures: Fixture[];
		types: FixtureType[];
		plan: StagePlan | null;
		planUrl: string | null;
	} = $props();

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
			const direction = beamDirection(fixture, typeOf(fixture));
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

			return {
				fixture,
				at,
				length,
				output,
				colour: new THREE.Color(output.r, output.g, output.b),
				rotation: along(new THREE.Vector3(0, 1, 0)),
				coneRotation: along(new THREE.Vector3(0, -1, 0)),
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

	const floor = $derived(plan ? planExtent(plan) : { width: 20, depth: 14 });
</script>

<T.PerspectiveCamera makeDefault position={home.position} fov={50} near={0.1} far={400}>
	<OrbitControls
		target={home.target}
		enableDamping
		dampingFactor={0.12}
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
		onclick={(event: { stopPropagation: () => void }) => {
			event.stopPropagation();
			select(beam.fixture.id);
		}}
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
