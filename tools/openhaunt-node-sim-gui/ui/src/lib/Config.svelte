<script lang="ts">
	import { untrack } from 'svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { open, save } from '@tauri-apps/plugin-dialog';
	import {
		ACCESSES,
		aNewPort,
		CLASSES,
		DATA_TYPES,
		MAINS_FLAG,
		problems,
		UNITS,
		type Demo,
		type NodeConfig,
		type Snapshot
	} from './node.js';

	let { node }: { node: Snapshot } = $props();

	// The draft is the panel's own copy, so half-typed nonsense never reaches the
	// running node. `running` is what the node last told us it is, and the two
	// being different is the whole reason Apply is a button.
	// Deliberately the value it has now, not a subscription: the effect below is
	// what takes a later one, and only when the draft is not mid-edit.
	let draft = $state<NodeConfig>(untrack(() => structuredClone($state.snapshot(node.config))));
	let running = $state(untrack(() => JSON.stringify(node.config)));
	let open_ = $state(false);
	let busy = $state(false);
	let message = $state<string | null>(null);
	let trouble = $state(false);
	let presets = $state<NodeConfig[]>([]);
	let demos = $state<Demo[]>([]);

	$effect(() => {
		invoke<NodeConfig[]>('presets').then((p) => (presets = p));
		invoke<Demo[]>('demos').then((d) => (demos = d));
	});

	// A node the window did not change — restarted from a file, say — should show
	// up in the editor rather than being quietly overwritten by a stale draft.
	$effect(() => {
		const current = JSON.stringify(node.config);
		if (current !== running) {
			running = current;
			draft = structuredClone($state.snapshot(node.config));
		}
	});

	const dirty = $derived(JSON.stringify(draft) !== running);
	const wrong = $derived(problems(draft));

	function say(text: string, bad = false) {
		message = text;
		trouble = bad;
	}

	async function apply() {
		busy = true;
		try {
			const snapshot = await invoke<Snapshot>('apply', { config: $state.snapshot(draft) });
			running = JSON.stringify(snapshot.config);
			say(`running on ${snapshot.httpAddr}`);
		} catch (e) {
			say(String(e), true);
		} finally {
			busy = false;
		}
	}

	function revert() {
		draft = structuredClone($state.snapshot(node.config));
		say('back to what is running');
	}

	async function load() {
		const path = await open({
			title: 'Open a node config',
			filters: [{ name: 'Node config', extensions: ['json'] }]
		});
		if (typeof path !== 'string') return;
		try {
			draft = await invoke<NodeConfig>('load_config', { path });
			say(`loaded ${path} — Apply to run it`);
		} catch (e) {
			say(String(e), true);
		}
	}

	async function saveAs() {
		const path = await save({
			title: 'Save this node config',
			defaultPath: `${draft.serial}.json`,
			filters: [{ name: 'Node config', extensions: ['json'] }]
		});
		if (!path) return;
		try {
			await invoke('save_config', { path, config: $state.snapshot(draft) });
			say(`saved ${path}`);
		} catch (e) {
			say(String(e), true);
		}
	}

	/// Taking a preset keeps where this node is — its serial, its port, whether it
	/// advertises — and changes only what it claims to be. "Make this one a relay",
	/// not "throw this one away".
	function takePreset(config: NodeConfig) {
		draft.module = structuredClone($state.snapshot(config.module));
		draft.ports = structuredClone($state.snapshot(config.ports));
		draft.dmx = config.dmx ? structuredClone($state.snapshot(config.dmx)) : null;
		draft.name = `${config.module.name} ${draft.serial}`;
		say(`${config.module.name} — Apply to run it`);
	}

	function takeDemo(demo: Demo) {
		draft = structuredClone($state.snapshot(demo.config));
		say(`${demo.name} — Apply to run it`);
	}

	const mains = $derived((draft.module.flags & MAINS_FLAG) !== 0);

	function setMains(on: boolean) {
		draft.module.flags = on ? draft.module.flags | MAINS_FLAG : draft.module.flags & ~MAINS_FLAG;
	}

	function setDmx(on: boolean) {
		draft.dmx = on ? { protocols: ['sacn'], universes: 1 } : null;
	}

	/// An empty box means "not stated", which is a different thing from zero: a
	/// port with no `minimum` is a port that did not say, and a controller reads
	/// those differently.
	const num = (raw: string): number | undefined => (raw.trim() === '' ? undefined : Number(raw));
</script>

<section>
	<button class="disclosure" onclick={() => (open_ = !open_)} aria-expanded={open_}>
		<span class="caret" class:open={open_}>▸</span>
		<h2>Config</h2>
		{#if dirty}<span class="pill">unapplied</span>{/if}
		<span class="spacer"></span>
		{#if message}
			<span class="message" class:bad={trouble}>{message}</span>
		{/if}
	</button>

	{#if open_}
		<div class="bar">
			<button onclick={load}>Load…</button>
			<button onclick={saveAs}>Save…</button>

			<select
				value=""
				onchange={(e) => {
					const preset = presets[Number(e.currentTarget.value)];
					if (preset) takePreset(preset);
					e.currentTarget.value = '';
				}}
			>
				<option value="" disabled>Preset…</option>
				{#each presets as preset, i (preset.module.type)}
					<option value={i}>{preset.module.name}</option>
				{/each}
			</select>

			<select
				value=""
				onchange={(e) => {
					const demo = demos[Number(e.currentTarget.value)];
					if (demo) takeDemo(demo);
					e.currentTarget.value = '';
				}}
			>
				<option value="" disabled>Example…</option>
				{#each demos as demo, i (demo.name)}
					<option value={i}>{demo.name}</option>
				{/each}
			</select>

			<span class="spacer"></span>
			<button onclick={revert} disabled={!dirty}>Revert</button>
			<button class="primary" onclick={apply} disabled={busy || wrong.length > 0}>
				{busy ? 'Restarting…' : 'Apply'}
			</button>
		</div>

		{#if wrong.length > 0}
			<ul class="problems">
				{#each wrong as problem (problem)}
					<li>{problem}</li>
				{/each}
			</ul>
		{/if}

		<div class="fields">
			<label><span>Name</span><input bind:value={draft.name} /></label>
			<label><span>Serial</span><input class="mono" bind:value={draft.serial} /></label>
			<label>
				<span>Module</span>
				<input class="mono narrow" bind:value={draft.module.type} />
				<input bind:value={draft.module.name} />
			</label>
			<label><span>Rev</span><input class="narrow" bind:value={draft.module.rev} /></label>
			<label>
				<span>Caps</span>
				<input class="mono" bind:value={draft.module.caps} placeholder="dmx,rdm,sacn" />
			</label>
			<label class="check">
				<input type="checkbox" checked={mains} onchange={(e) => setMains(e.currentTarget.checked)} />
				<span>Switches mains — descriptor bit 6</span>
			</label>
			<label>
				<span>HTTP port</span>
				<input class="narrow" type="number" bind:value={draft.httpPort} />
			</label>
			<label class="check">
				<input type="checkbox" bind:checked={draft.advertise} />
				<span>Advertise over mDNS</span>
			</label>
			<label>
				<span>Auto</span>
				<input
					class="narrow"
					type="number"
					value={draft.autoMs ?? ''}
					placeholder="off"
					oninput={(e) => (draft.autoMs = num(e.currentTarget.value) ?? null)}
				/>
				<span class="unit" title="How often the node reports a reading or flips an input on its own">ms</span>
			</label>
			<label class="check">
				<input
					type="checkbox"
					checked={!!draft.dmx}
					onchange={(e) => setDmx(e.currentTarget.checked)}
				/>
				<span>Forwards a universe — makes this node a gateway</span>
			</label>
		</div>

		<div class="ports">
			<div class="row head">
				<span>#</span><span>Name</span><span>Access</span><span>Type</span><span>Unit</span>
				<span>Min</span><span>Max</span><span>Default</span><span>Class</span><span></span>
			</div>
			{#each draft.ports as port, i (i)}
				<div class="row">
					<input class="mono tiny" type="number" bind:value={port.port} />
					<input bind:value={port.name} />
					<select bind:value={port.access}>
						{#each ACCESSES as access}<option value={access}>{access}</option>{/each}
					</select>
					<select bind:value={port.dataType}>
						{#each DATA_TYPES as type}<option value={type}>{type}</option>{/each}
					</select>
					<input
						list="units"
						class="unit-input"
						value={port.unit ?? ''}
						placeholder="—"
						oninput={(e) => (port.unit = e.currentTarget.value || undefined)}
					/>
					<input
						class="tiny"
						type="number"
						value={port.minimum ?? ''}
						placeholder="—"
						oninput={(e) => (port.minimum = num(e.currentTarget.value))}
					/>
					<input
						class="tiny"
						type="number"
						value={port.maximum ?? ''}
						placeholder="—"
						oninput={(e) => (port.maximum = num(e.currentTarget.value))}
					/>
					<input
						class="tiny"
						type="number"
						value={port.default ?? ''}
						placeholder="—"
						oninput={(e) => (port.default = num(e.currentTarget.value))}
					/>
					<input
						list="classes"
						value={port.class ?? ''}
						placeholder="—"
						title="A hint. A word a controller does not know is not an error."
						oninput={(e) => (port.class = e.currentTarget.value || undefined)}
					/>
					<button
						class="drop"
						aria-label="Remove port {port.port}"
						onclick={() => (draft.ports = draft.ports.filter((_, n) => n !== i))}>×</button
					>
				</div>
			{/each}
			<datalist id="units">
				{#each UNITS as unit}<option value={unit}></option>{/each}
			</datalist>
			<datalist id="classes">
				{#each CLASSES as name}<option value={name}></option>{/each}
			</datalist>

			<button class="add" onclick={() => (draft.ports = [...draft.ports, aNewPort(draft.ports)])}>
				+ Port
			</button>
			{#if draft.ports.length === 0 && !draft.dmx}
				<p class="note">
					A node with no ports and no universe describes nothing, and a console is
					entitled to refuse to adopt it. Which is worth being able to try.
				</p>
			{/if}
		</div>
	{/if}
</section>

<style>
	section {
		border-bottom: 1px solid var(--line);
	}

	.disclosure {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 12px 20px;
		background: none;
		border: none;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
	}

	h2 {
		font-size: 0.7rem;
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--dim);
	}

	.caret {
		color: var(--dim);
		transition: transform 0.12s;
	}
	.caret.open {
		transform: rotate(90deg);
	}

	.spacer {
		flex: 1;
	}

	.pill {
		font-size: 0.62rem;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: #1a1a1a;
		background: var(--warn);
		border-radius: 3px;
		padding: 2px 6px;
	}

	.message {
		font-size: 0.72rem;
		color: var(--dim);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 40ch;
	}
	.message.bad {
		color: var(--warn);
	}

	.bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 0 20px 12px;
	}

	.problems {
		margin: 0 20px 12px;
		padding: 8px 12px;
		list-style: none;
		border: 1px solid var(--warn);
		border-radius: 5px;
		color: var(--warn);
		font-size: 0.75rem;
	}

	.fields {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
		gap: 8px 20px;
		padding: 0 20px 12px;
	}

	label {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 0.78rem;
	}

	label > span:first-child {
		min-width: 5.5rem;
		color: var(--dim);
	}

	label.check > span {
		min-width: 0;
		color: inherit;
	}

	.unit {
		color: var(--dim);
		font-size: 0.7rem;
	}

	input,
	select {
		flex: 1;
		min-width: 0;
		padding: 4px 6px;
		background: #12160f;
		border: 1px solid var(--line);
		border-radius: 4px;
		color: var(--text);
		font: inherit;
		font-size: 0.78rem;
	}

	input::placeholder {
		color: #4b5449;
	}

	input[type='checkbox'] {
		flex: 0 0 auto;
		accent-color: var(--live);
	}

	.narrow {
		flex: 0 0 7rem;
	}
	label > .narrow + input {
		flex: 1 1 8rem;
	}
	.tiny {
		flex: 0 0 4.5rem;
	}
	.unit-input {
		flex: 0 0 9rem;
	}

	.ports {
		padding: 0 20px 16px;
		overflow-x: auto;
	}

	.row {
		display: grid;
		grid-template-columns: 3.5rem minmax(8rem, 1fr) 6.5rem 6rem 9rem 4.5rem 4.5rem 4.5rem 8rem 1.5rem;
		gap: 6px;
		align-items: center;
		margin-bottom: 4px;
		min-width: 62rem;
	}

	.row.head span {
		font-size: 0.62rem;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--dim);
	}

	.row input,
	.row select {
		flex: none;
		width: 100%;
	}

	button {
		padding: 4px 10px;
		background: #20261f;
		border: 1px solid var(--line);
		border-radius: 4px;
		color: var(--text);
		font: inherit;
		font-size: 0.75rem;
		cursor: pointer;
	}

	button:hover:not(:disabled) {
		border-color: #3d463f;
	}
	button:disabled {
		opacity: 0.45;
		cursor: not-allowed;
	}

	button.primary {
		background: #1f3a22;
		border-color: var(--live);
	}

	.drop {
		padding: 2px 6px;
		background: none;
		border: none;
		color: var(--dim);
		font-size: 1rem;
		line-height: 1;
	}
	.drop:hover {
		color: var(--warn);
	}

	.add {
		margin-top: 4px;
	}

	.note {
		margin-top: 10px;
		font-size: 0.75rem;
		color: var(--dim);
		max-width: 60ch;
	}
</style>
