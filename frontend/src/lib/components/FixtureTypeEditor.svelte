<script lang="ts">
	/**
	 * The show's fixture types, as a list; one is edited in a dialog on top of it.
	 *
	 * The list used to unfold a type in place, and one with forty parameters pushed
	 * every other type off the panel. Now each row says what a type is — where it came
	 * from, how many channels, how much of the rig is made of it — and Edit opens
	 * `FixtureTypeDialog` with Parameters, Modes and Color.
	 */
	import { focusOnMount } from '$lib/actions.js';
	import { onMount } from 'svelte';
	import Download from '@lucide/svelte/icons/download';
	import Pencil from '@lucide/svelte/icons/pencil';
	import Trash from '@lucide/svelte/icons/trash-2';
	import { getDataContext } from '$lib/ws/context.js';
	import type { FixtureType } from '$lib/generated/index.js';
	import { defaultValueFor } from '$lib/patch.js';
	import { canEdit } from '$lib/stores/showLock.js';
	import { backendOrigin } from '$lib/ws/endpoint.js';
	import { userId } from '$lib/stores/user.js';
	import FixtureTypeShare from './FixtureTypeShare.svelte';
	import FixtureTypeDialog from './fixturetypes/FixtureTypeDialog.svelte';
	import Dialog from './Dialog.svelte';
	import Help from './Help.svelte';
	import { collection } from '$lib/stores/show.js';
	import { uuid } from '$lib/uuid.js';

	const backend = () => backendOrigin(window.location);
	const data = getDataContext();

	let types = $state<FixtureType[]>([]);
	/// What the last import said, shown until the next one or until it is dismissed.
	let report = $state<{ ok: boolean; text: string; warnings: string[] } | null>(null);
	let importing = $state(false);
	let fileInput = $state<HTMLInputElement | null>(null);
	/// Which half of this panel is showing: the show's own types, or the Share.
	let tab = $state<'types' | 'share'>('types');
	/// The type being edited, by id, so the dialog follows every write to it.
	let editing = $state<string | null>(null);
	const edited = $derived(editing ? (types.find((t) => t.id === editing) ?? null) : null);
	let newName = $state('');
	let creating = $state(false);

	/**
	 * Send a `.gdtf` to the station and say what it made of it.
	 *
	 * The file goes as raw bytes with the operator's id in a header, because an HTTP
	 * request carries no `Identify` the way the socket does — and an import an
	 * operator cannot undo would be the worst thing on this panel.
	 */
	async function importGdtf(file: File) {
		importing = true;
		report = null;
		try {
			const answer = await fetch(`${backend()}/api/import/gdtf`, {
				method: 'POST',
				headers: {
					'content-type': 'application/vnd.gdtf+zip',
					'x-pult-user': $userId
				},
				body: await file.arrayBuffer()
			});
			if (!answer.ok) {
				report = { ok: false, text: (await answer.text()) || answer.statusText, warnings: [] };
				return;
			}
			const body = await answer.json();
			const warnings: string[] = body.warnings ?? [];
			report = {
				ok: true,
				text: body.replaced
					? `Updated ${file.name} — every fixture patched to it follows.`
					: `Imported ${file.name}.`,
				warnings
			};
		} catch (error) {
			report = { ok: false, text: String(error), warnings: [] };
		} finally {
			importing = false;
			if (fileInput) fileInput.value = '';
		}
	}

	/// A type as a file to save. An imported one comes back as the archive it arrived
	/// in; one made here comes back as a generated file another console can open.
	function exportGdtf(type: FixtureType) {
		window.open(`${backend()}/api/export/gdtf/${type.id}`, '_blank');
	}

	/// Where a type came from, as a badge, with the GDTF revision where the file has one.
	function sourceOf(type: FixtureType): { key: 'manual' | 'node' | 'gdtf'; text: string } {
		if (type.source === 'Manual') return { key: 'manual', text: 'Made here' };
		if (type.source === 'Node') return { key: 'node', text: 'Node' };
		const revision = type.source.Gdtf.revision;
		return { key: 'gdtf', text: revision ? `GDTF · ${revision}` : 'GDTF' };
	}

	async function createType() {
		const name = newName.trim();
		if (!name) return;
		const id = uuid();
		await data.fixture_types.create({
			id,
			name,
			manufacturer: 'Generic',
			short_name: '',
			long_name: name,
			description: '',
			channel_count: 1,
			parameters: [
				{
					kind: 'Intensity',
					direction: 'Output',
					binding: null,
					default_value: defaultValueFor('Intensity'),
					physical: null,
					slots: [],
					feature_group: null,
					emitters: [],
					per_cell: false
				}
			],
			// A type made here names no mode: where its channels go follows from the
			// order of its parameters, and the station computes the layout from them.
			// Modes are what an imported GDTF file brings.
			dmx_modes: [],
			physical: {
				weight_kg: null,
				power_w: null,
				dimensions_m: null,
				connectors: [],
				leg_height_m: null,
				operating_temperature: null,
				beam_angle_deg: null
			},
			geometry: [],
			source: 'Manual',
			// Nothing to draw a plan head from yet — no geometry, no thumbnail — so
			// `Auto` will land on the generic symbol until somebody gives it either.
			plan_symbol: 'Auto',
			thumbnail: null,
			cells: 0
		});
		newName = '';
		creating = false;
		// Straight into it: a type with one dimmer is a start, not a finish.
		editing = id;
	}

	onMount(() => data.fixture_types.subscribeDeep((v) => { types = v; }));

	const fixtures = collection('fixtures');
	/**
	 * A type nothing is patched from deletes in place, and Ctrl-Z brings it back. One
	 * the rig uses asks first: every fixture of it loses what its channels mean the
	 * moment it goes, which is a whole rig going dark from one ×.
	 */
	let deleting = $state<FixtureType | null>(null);
	const usedBy = (type: FixtureType) => $fixtures.filter((f) => f.fixture_type_id === type.id).length;

	function askDelete(type: FixtureType) {
		if (usedBy(type) === 0) void data.fixture_types.byId(type.id).delete();
		else deleting = type;
	}
</script>

<section class="block">
	<header class="block-head">
		<h2>Fixture types</h2>
		<nav class="tabs">
			<button class="tab" class:on={tab === 'types'} onclick={() => (tab = 'types')}>In this show</button>
			<button class="tab" class:on={tab === 'share'} onclick={() => (tab = 'share')}>From GDTF Share</button>
		</nav>
		{#if $canEdit && tab === 'types'}
			<button class="btn btn-ghost" disabled={importing} onclick={() => fileInput?.click()}
				>{importing ? 'Importing…' : 'Import GDTF'}</button
			>
			<button class="btn btn-ghost" onclick={() => (creating = !creating)}>{creating ? 'Cancel' : '+ Type'}</button>
		{/if}
	</header>

	<!-- Hidden rather than styled: the button above is the control, and a file input
	     cannot be made to look like the rest of this panel. -->
	<input
		class="file"
		type="file"
		accept=".gdtf,application/vnd.gdtf+zip"
		bind:this={fileInput}
		onchange={(e) => {
			const file = e.currentTarget.files?.[0];
			if (file) importGdtf(file);
		}}
	/>

	{#if report}
		<div class="report" class:bad={!report.ok}>
			<div>
				<p>{report.text}</p>
				{#if report.warnings.length > 0}
					<!-- Warnings, not errors: a Share file with a dangling reference is still
					     a fixture somebody has to patch tonight. -->
					<p class="worth">
						{report.warnings.length} thing{report.warnings.length === 1 ? '' : 's'} worth knowing
						<Help label="What the import noticed">
							{#each report.warnings as warning, w (w)}<span class="warning">{warning}</span>{/each}
						</Help>
					</p>
				{/if}
			</div>
			<button class="btn btn-ghost btn-icon" title="Dismiss" aria-label="Dismiss" onclick={() => (report = null)}>×</button>
		</div>
	{/if}

	{#if tab === 'share'}
		<FixtureTypeShare />
	{:else}
		{#if creating && $canEdit}
			<form
				class="new-row"
				onsubmit={(e) => {
					e.preventDefault();
					createType();
				}}
			>
				<input class="input" placeholder="Type name, e.g. Source Four" bind:value={newName} use:focusOnMount />
				<button class="btn btn-primary" type="submit">Create</button>
			</form>
		{/if}

		{#if types.length === 0}
			<p class="empty">No fixture types yet. A fixture needs one to know what its channels do.</p>
		{:else}
			<table class="tbl">
				<thead>
					<tr><th>Name</th><th>Manufacturer</th><th>Source</th><th class="num">Channels</th><th class="num">Patched</th><th></th></tr>
				</thead>
				<tbody>
					{#each types as type (type.id)}
						{@const source = sourceOf(type)}
						{@const count = usedBy(type)}
						<tr ondblclick={() => (editing = type.id)}>
							<td>
								<button class="name" onclick={() => (editing = type.id)}>{type.name}</button>
							</td>
							<td class="dim">{type.manufacturer}</td>
							<td><span class="badge {source.key}">{source.text}</span></td>
							<td class="num">{type.channel_count}{#if type.dmx_modes.length > 1}<span class="dim"> · {type.dmx_modes.length} modes</span>{/if}</td>
							<td class="num" class:dim={count === 0}>{count}</td>
							<td class="end">
								<button
									class="btn btn-ghost btn-icon"
									title="Edit {type.name}"
									aria-label="Edit {type.name}"
									onclick={() => (editing = type.id)}><Pencil size={14} /></button
								>
								<button
									class="btn btn-ghost btn-icon"
									title="Export {type.name} as GDTF"
									aria-label="Export {type.name} as GDTF"
									onclick={() => exportGdtf(type)}><Download size={14} /></button
								>
								{#if $canEdit}
									<button
										class="btn btn-ghost btn-icon danger"
										title="Delete {type.name}"
										aria-label="Delete {type.name}"
										onclick={() => askDelete(type)}><Trash size={14} /></button
									>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	{/if}
</section>

{#if edited}
	<FixtureTypeDialog type={edited} onclose={() => (editing = null)} />
{/if}

{#if deleting}
	{@const count = usedBy(deleting)}
	<Dialog title="Delete a fixture type" onclose={() => (deleting = null)}>
		<p class="ask">
			<b>{deleting.name}</b> is what {count} patched fixture{count === 1 ? '' : 's'}
			{count === 1 ? 'is' : 'are'} made of. Deleting it leaves {count === 1 ? 'that fixture' : 'them'}
			with no idea what {count === 1 ? 'its' : 'their'} channels do, and the rig stops sending
			to {count === 1 ? 'it' : 'them'}. Ctrl-Z brings it back.
		</p>
		{#snippet footer()}
			<button class="btn btn-ghost" onclick={() => (deleting = null)}>Cancel</button>
			<button
				class="btn btn-danger"
				onclick={() => {
					const id = deleting!.id;
					deleting = null;
					void data.fixture_types.byId(id).delete();
				}}>Delete it</button
			>
		{/snippet}
	</Dialog>
{/if}

<style>
	.ask {
		padding: 14px;
		font-size: var(--font-sm);
		line-height: 1.5;
	}
	.tabs {
		display: flex;
		gap: 2px;
		margin-left: auto;
	}
	.tab {
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--text-dim);
		font: inherit;
		font-size: 12px;
		padding: 2px 8px 3px;
		cursor: pointer;
	}
	.tab.on {
		color: var(--text);
		border-bottom-color: var(--accent, #4a9eff);
	}

	/* The button in the header is the control; the input itself is never seen. */
	.file {
		display: none;
	}

	.report {
		display: grid;
		grid-template-columns: 1fr auto;
		align-items: start;
		gap: 8px;
		margin: 8px 0;
		padding: 8px 12px;
		border-radius: 4px;
		background: var(--bg-raised);
		border-left: 3px solid var(--good, #4caf50);
		font-size: 12px;
	}
	.report.bad {
		border-left-color: var(--bad, #e5534b);
	}
	.report p {
		margin: 0;
	}
	.worth {
		color: var(--text-dim);
		margin-top: 4px !important;
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.warning {
		display: block;
	}
	.warning + .warning {
		margin-top: 6px;
	}

	.block {
		margin-bottom: 24px;
	}
	.block-head {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-bottom: 8px;
	}
	h2 {
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
	}
	.tbl {
		width: 100%;
		border-collapse: collapse;
		font-size: var(--font-sm);
	}
	.tbl th {
		text-align: left;
		font-weight: 500;
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--text-faint);
		padding: 0 10px 6px 0;
	}
	.tbl td {
		padding: 4px 10px 4px 0;
		border-top: 1px solid var(--line);
		vertical-align: middle;
	}
	.tbl .num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.tbl .end {
		text-align: right;
		padding-right: 0;
		white-space: nowrap;
	}
	.name {
		background: none;
		border: none;
		padding: 4px 0;
		color: var(--text);
		font: inherit;
		font-weight: 500;
		cursor: pointer;
		text-align: left;
	}
	.name:hover {
		color: var(--accent);
	}
	.dim {
		color: var(--text-faint);
	}
	.badge {
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 2px 7px;
		border-radius: 9px;
		background: var(--bg-raised);
		color: var(--text-dim);
		white-space: nowrap;
	}
	.badge.gdtf {
		color: var(--accent);
	}
	.badge.node {
		color: var(--good, #4ade80);
	}
	.danger:hover {
		color: var(--bad);
	}
	.new-row {
		display: flex;
		gap: 6px;
		margin-bottom: 8px;
	}
	.empty {
		color: var(--text-faint);
		font-size: 13px;
		padding: 8px 0;
	}
</style>
