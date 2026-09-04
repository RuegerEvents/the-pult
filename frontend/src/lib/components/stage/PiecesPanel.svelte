<script lang="ts">
	/**
	 * The pieces a console can put in a room, to drag into one.
	 *
	 * A panel rather than a sheet over the rig: it is a library, it wants the height,
	 * and a list that opened out of the rig's own toolbar pushed the picture down every
	 * time — which is the last thing a view somebody is dragging onto should do.
	 *
	 * A native drag rather than a click-then-click: the second half of the gesture is
	 * *where*, and a drag says that with the pointer instead of asking for a mode the
	 * operator has to remember they are in. Escape cancels it, which the browser gives
	 * for nothing.
	 *
	 * The **work plane** lives here rather than on the View sheet, and that is not
	 * filing: it is the answer to "how far away", which is the one question a drop
	 * cannot answer on its own. A bar goes in at six metres, and then everything else
	 * dropped goes in beside it rather than on the floor underneath it.
	 */
	import { groupedCatalogue } from '$lib/stock.js';
	import { setView, view } from '$lib/stores/view.js';

	const groups = groupedCatalogue();
</script>

<div class="pieces">
	<div class="plane">
		<label>
			<span>Work height</span>
			<input
				type="number"
				step="0.5"
				value={$view.workHeight}
				onchange={(e) => setView({ workHeight: e.currentTarget.valueAsNumber })}
			/>
			<span class="unit">m</span>
		</label>
		<label>
			<span>Work depth</span>
			<input
				type="number"
				step="0.5"
				value={$view.workDepth}
				onchange={(e) => setView({ workDepth: e.currentTarget.valueAsNumber })}
			/>
			<span class="unit">m</span>
		</label>
		<p class="note">
			Where a dropped piece lands. A view that can see the floor uses the height; one
			looking along it uses the depth, because a horizontal plane seen edge-on catches
			nothing. Then it snaps to the grid, and then to anything it can bolt to. Hold
			Alt to drop it exactly where the pointer is.
		</p>
	</div>

	{#each groups as group (group.label)}
		<section>
			<h3>{group.label}</h3>
			<ul>
				{#each group.pieces as entry (entry.id)}
					<li>
						<button
							class="piece"
							draggable="true"
							title={`${entry.title} — ${entry.size.x} × ${entry.size.y} × ${entry.size.z} m`}
							ondragstart={(event) => {
								// Plain text, because that is what a native drag carries and the
								// canvas is the only thing reading it.
								event.dataTransfer?.setData('text/plain', entry.id);
								if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copy';
							}}
						>
							<span class="name">{entry.title}</span>
							<span class="size">{entry.size.x} m</span>
						</button>
					</li>
				{/each}
			</ul>
		</section>
	{/each}
</div>

<style>
	.pieces {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		gap: 12px 22px;
		padding: 10px 12px;
	}

	.plane { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; flex-basis: 100%; }
	.plane label { display: flex; align-items: center; gap: 6px; color: #888; font-size: 12px; }
	.plane input { width: 6ch; background: #101010; border: 1px solid var(--line-strong); border-radius: 3px; color: #ddd; padding: 3px 5px; font: inherit; font-size: 12px; }
	.unit { color: #666; }
	.note { flex-basis: 100%; color: #666; font-size: 11px; line-height: 1.5; max-width: 70ch; margin: 0; }

	section { min-width: 150px; }
	h3 { margin: 0 0 4px; color: #777; font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: 0.04em; }
	ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }

	.piece {
		display: flex;
		align-items: baseline;
		gap: 8px;
		width: 100%;
		background: none;
		border: 1px solid var(--line-strong);
		border-radius: 3px;
		color: #bbb;
		padding: 4px 8px;
		font: inherit;
		font-size: 12px;
		text-align: left;
		cursor: grab;
	}
	.piece:hover { border-color: var(--line-input); color: #fff; }
	.piece:active { cursor: grabbing; }
	.name { flex: 1; }
	.size { color: #666; font-variant-numeric: tabular-nums; }
</style>
