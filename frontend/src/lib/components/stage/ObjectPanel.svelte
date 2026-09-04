<script lang="ts">
	/**
	 * What one piece of the drawing is, in numbers.
	 *
	 * The gizmo is for the ninety percent of moves that are "about there"; this is for
	 * the other ten, which are "exactly six metres". Both write the same field, and
	 * neither is the primary one.
	 *
	 * A panel rather than a sheet on the rig's toolbar: it appeared and disappeared with
	 * the selection, and a strip of fields that shoves the picture down every time
	 * somebody clicks a truss makes the next click land somewhere else. It picks the
	 * selection up itself, so it works with no rig tile open at all — which is what an
	 * operator typing coordinates off a plan actually wants.
	 *
	 * The piece's own **properties** are rendered from what the catalogue declares
	 * rather than from a list here — a deck says it has legs and what they may be, and
	 * this offers exactly that. Which is what makes adding a property to a piece a
	 * change in one Rust file: `catalogue.rs` declares it, `stock/shapes.rs` reads it,
	 * and the control turns up here.
	 */
	import type { PropertyKind, SceneObject } from '$lib/generated/index.js';
	import { getDataContext } from '$lib/ws/context.js';
	import { canonicalProperties, piece } from '$lib/stock.js';
	import { isLocked, layers, objectsById, selectedObjects } from '$lib/stores/scene.js';
	import { asOneAct } from '$lib/stores/editor.js';

	const held = $derived(
		[...$selectedObjects]
			.map((id) => $objectsById.get(id))
			.filter((each): each is SceneObject => !!each)
	);
	const object = $derived(held[0]);

	const data = getDataContext();
	const entry = $derived(piece(object?.catalogue));
	/** Locked by its own flag, or by the layer it is on. Either way, read-only here. */
	const locked = $derived(isLocked(object, $layers));
	const properties = $derived(canonicalProperties(entry, object?.properties));

	const row = $derived(data.scene_objects.byId(object?.id ?? ''));

	/** One number of the placement, written back as a whole transform. */
	function setAxis(field: 'position' | 'rotation' | 'scale', axis: 'x' | 'y' | 'z', value: number) {
		if (!Number.isFinite(value)) return;
		void row.transform.set({ ...object.transform, [field]: { ...object.transform[field], [axis]: value } });
	}

	/**
	 * Changing what a piece was asked for.
	 *
	 * The whole map is written rather than the one key, so a change is one row and one
	 * Ctrl-Z — and the canonical form goes in, so the mesh this asks the station for is
	 * the mesh an export writes.
	 */
	function setProperty(key: string, value: number | string | boolean) {
		void asOneAct(() =>
			row.properties.set(canonicalProperties(entry, { ...properties, [key]: value }))
		);
	}

	const numberKind = (kind: PropertyKind) =>
		typeof kind === 'object' && 'Number' in kind ? kind.Number : null;
	const choiceKind = (kind: PropertyKind) =>
		typeof kind === 'object' && 'Choice' in kind ? kind.Choice : null;
</script>

{#if held.length === 0}
	<p class="empty">
		Nothing picked. Click a piece in the rig, or a row in <strong>Objects</strong>, and
		what it is turns up here.
	</p>
{:else if held.length > 1}
	<p class="empty">
		{held.length} pieces picked. This is one piece at a time — the numbers are what a
		single thing is, and there is no honest value to show for six of them at once. The
		gizmo and <strong>Line up</strong> in Rig tools are what move several.
	</p>
{:else}
<div class="sheet">
	<label class="field wide">
		<span>Name</span>
		<input value={object.name} onchange={(e) => row.name.set(e.currentTarget.value)} />
	</label>

	<label class="field">
		<span>Layer</span>
		<select
			value={object.layer ?? ''}
			onchange={(e) => row.layer.set(e.currentTarget.value || null)}
		>
			<option value="">None</option>
			{#each $layers as layer (layer.id)}
				<option value={layer.id}>{layer.name}</option>
			{/each}
		</select>
	</label>

	<label class="field">
		<input
			type="checkbox"
			checked={object.locked}
			onchange={(e) => row.locked.set(e.currentTarget.checked)}
		/>
		<span>Locked</span>
	</label>

	<p class="what">
		{entry ? entry.title : object.geometry.length > 0 ? 'Imported mesh' : object.kind}
	</p>

	{#each ['position', 'rotation', 'scale'] as const as field (field)}
		<div class="field triple">
			<span>{field === 'rotation' ? 'Rotation °' : field === 'scale' ? 'Scale' : 'Position m'}</span>
			{#each ['x', 'y', 'z'] as const as axis (axis)}
				<input
					type="number"
					step={field === 'rotation' ? 5 : field === 'scale' ? 0.1 : 0.1}
					disabled={locked}
					value={round(object.transform[field][axis])}
					onchange={(e) => setAxis(field, axis, e.currentTarget.valueAsNumber)}
				/>
			{/each}
		</div>
	{/each}

	{#each entry?.properties ?? [] as property (property.key)}
		{@const number = numberKind(property.kind)}
		{@const choice = choiceKind(property.kind)}
		<label class="field">
			<span>{property.title}</span>
			{#if number}
				<input
					type="number"
					min={number.min}
					max={number.max}
					step={number.step}
					disabled={locked}
					value={properties[property.key]}
					onchange={(e) => setProperty(property.key, e.currentTarget.valueAsNumber)}
				/>
				<span class="unit">{number.unit}</span>
			{:else if choice}
				<select
					disabled={locked}
					value={properties[property.key]}
					onchange={(e) => setProperty(property.key, e.currentTarget.value)}
				>
					{#each choice.options as option (option)}
						<option value={option}>{option}</option>
					{/each}
				</select>
			{:else}
				<input
					type="checkbox"
					disabled={locked}
					checked={properties[property.key] === true}
					onchange={(e) => setProperty(property.key, e.currentTarget.checked)}
				/>
			{/if}
		</label>
	{/each}
</div>
{/if}

<script lang="ts" module>
	/** Six decimals, so a float's own last bits do not turn up in a spin box. */
	function round(value: number): number {
		return Math.round(value * 1e6) / 1e6;
	}
</script>

<style>
	.sheet { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 18px; padding: 10px 12px; }
	.empty { color: var(--text-dim); font-size: var(--font-sm); margin: 0; padding: 16px; line-height: 1.6; max-width: 46ch; }
	.field { display: flex; align-items: center; gap: 6px; color: #888; font-size: 12px; }
	.field.wide { flex: 1 1 200px; }
	.field.wide input { flex: 1; }
	.triple input { width: 7ch; }
	input,
	select {
		background: #101010;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		color: #ddd;
		padding: 3px 5px;
		font: inherit;
		font-size: 12px;
	}
	input[type='checkbox'] { padding: 0; }
	input:disabled, select:disabled { color: #666; }
	.unit { color: #666; }
	.what { color: #666; font-size: 11px; margin: 0; }
</style>
