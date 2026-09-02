<script lang="ts">
	/**
	 * One object of the drawing, in the rig view.
	 *
	 * Its geometry is either its own or a symbol's — a truss type drawn once and
	 * instanced everywhere is why `symbols` is a collection — and either way it is
	 * loaded once per sha and cloned per object.
	 *
	 * A file the loader refuses becomes a box rather than nothing, and a box rather
	 * than a broken canvas: the view's job is to say where things are, and it can do
	 * that for an object whose mesh it could not read.
	 */
	import { T } from '@threlte/core';
	import * as THREE from 'three';

	import type { GeometryRef, NamedAsset, SceneObject, Symbol } from '$lib/generated/index.js';
	import { instance, load, type Loaded } from '$lib/geometry.js';
	import { worldTransform } from '$lib/scene.js';

	let {
		object,
		objects,
		symbols,
		names,
		onpick
	}: {
		object: SceneObject;
		objects: Map<string, SceneObject>;
		symbols: Symbol[];
		names: NamedAsset[];
		onpick?: (id: string, event: { stopPropagation: () => void }) => void;
	} = $props();

	const world = $derived(worldTransform(object.transform, object.parent, objects));
	const mirrored = $derived(world.scale.x * world.scale.y * world.scale.z < 0);

	/// Its own meshes, or the ones its symbol carries.
	const references: GeometryRef[] = $derived(
		object.geometry.length > 0
			? object.geometry
			: (symbols.find((s) => s.id === object.symbol)?.geometry ?? [])
	);

	/// Loaded and cloned, one three.js object per reference. Rebuilt only when the
	/// references change: a truss that moves does not reload its mesh.
	let drawn = $state<{ node: THREE.Object3D; reference: GeometryRef }[]>([]);
	$effect(() => {
		const wanted = references;
		const flipped = mirrored;
		let live = true;
		Promise.all(wanted.map((reference) => load(reference.asset, reference.file_name, names)))
			.then((meshes: Loaded[]) => {
				if (!live) return;
				drawn = meshes.map((mesh, index) => ({
					node: instance(mesh, flipped),
					reference: wanted[index]
				}));
			})
			.catch(() => {
				if (live) drawn = [];
			});
		return () => {
			live = false;
		};
	});

	const radians = (degrees: number) => (degrees * Math.PI) / 180;
</script>

<T.Group
	position={[world.position.x, world.position.y, world.position.z]}
	rotation={[radians(world.rotation.x), radians(world.rotation.y), radians(world.rotation.z)]}
	scale={[world.scale.x, world.scale.y, world.scale.z]}
	onclick={(event: { stopPropagation: () => void }) => onpick?.(object.id, event)}
>
	{#each drawn as mesh (mesh.reference.asset + mesh.reference.file_name)}
		<T.Group
			position={[
				mesh.reference.transform.position.x,
				mesh.reference.transform.position.y,
				mesh.reference.transform.position.z
			]}
			rotation={[
				radians(mesh.reference.transform.rotation.x),
				radians(mesh.reference.transform.rotation.y),
				radians(mesh.reference.transform.rotation.z)
			]}
			scale={[
				mesh.reference.transform.scale.x,
				mesh.reference.transform.scale.y,
				mesh.reference.transform.scale.z
			]}
		>
			<T is={mesh.node} />
		</T.Group>
	{/each}
</T.Group>
