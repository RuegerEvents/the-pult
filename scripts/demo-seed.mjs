// Put something in a fresh demo show, over the ordinary WebSocket API.
//
// Nothing here is privileged — it is the same protocol the frontend speaks, so if
// this drifts out of date it will fail loudly rather than quietly doing the wrong
// thing. Run by scripts/demo.sh; takes the backend's port as its only argument.

import { connect } from './demo-ws.mjs';

const station = connect(process.argv[2] ?? '7700');
const { get, set, create } = station;

const id = () => crypto.randomUUID();

await station.open.catch((error) => {
	console.error(`  ${error.message}`);
	process.exit(1);
});

try {
	if (await get(['show'])) {
		console.log('  this show already has something in it; leaving it alone');
		process.exit(0);
	}

	await set(['show'], {
		id: id(),
		name: 'Demo',
		created_at: new Date().toISOString()
	});

	// One ordinary DMX fixture type, so the Patch tab has something in it and an
	// Art-Net output has something to send.
	const dimmer = {
		id: id(),
		name: 'Dimmer',
		manufacturer: 'Generic',
		channel_count: 1,
		parameters: [
			{
				kind: 'Intensity',
				direction: 'Output',
				binding: { Dmx: { channel: 1 } },
				default_value: { type: 'Float', value: 0 }
			}
		]
	};
	await create('fixture_types', dimmer);

	// And a moving head, so there is something to puppeteer. A colour takes the
	// three channels from the one it is bound to, so pan starts at 5.
	const spot = {
		id: id(),
		name: 'Spot',
		manufacturer: 'Generic',
		channel_count: 6,
		parameters: [
			{
				kind: 'Intensity',
				direction: 'Output',
				binding: { Dmx: { channel: 1 } },
				default_value: { type: 'Float', value: 0 }
			},
			{
				kind: 'ColorRgb',
				direction: 'Output',
				binding: { Dmx: { channel: 2 } },
				default_value: { type: 'Color', value: { r: 1, g: 1, b: 1 } }
			},
			{
				kind: 'Pan',
				direction: 'Output',
				binding: { Dmx: { channel: 5 } },
				default_value: { type: 'Float', value: 0.5 }
			},
			{
				kind: 'Tilt',
				direction: 'Output',
				binding: { Dmx: { channel: 6 } },
				default_value: { type: 'Float', value: 0.5 }
			}
		]
	};
	await create('fixture_types', spot);

	// Hung where the names say, in metres: X to the right as seen from front of
	// house, Y up, Z downstage towards the audience. Placed rather than null so the
	// Stage tab opens with a rig in it rather than three chips in a tray.
	const fixtures = [
		['Front left', { x: -3, y: 4.5, z: 2 }],
		['Front right', { x: 3, y: 4.5, z: 2 }],
		['Backlight', { x: 0, y: 5, z: -3 }]
	];
	for (const [index, [name, at]] of fixtures.entries()) {
		await create('fixtures', {
			id: id(),
			name,
			fixture_type_id: dimmer.id,
			address: { Dmx: { universe: 1, address: 1 + index } },
			position: { Point: at },
			live_values: {},
		});
	}

	// Hung axially rather than as points: a moving head needs a rest direction for
	// pan and tilt to be angles away from, and these two face downstage and down.
	const heads = [
		['Head left', { x: -2.5, y: 5, z: -1 }],
		['Head right', { x: 2.5, y: 5, z: -1 }]
	];
	for (const [index, [name, at]] of heads.entries()) {
		await create('fixtures', {
			id: id(),
			name,
			fixture_type_id: spot.id,
			address: { Dmx: { universe: 1, address: 11 + index * spot.channel_count } },
			position: { Axial: { position: at, direction: { x: 0, y: -0.8, z: 0.6 } } },
			live_values: {},
		});
	}

	const patched = await get(['fixtures']);

	// Two cues that actually move something, so Go does something visible and a
	// trigger wired to a contact has somewhere to go.
	const capture = (fixture, level) => ({
		fixture_id: fixture.id,
		parameter_kind: 'Intensity',
		value: { type: 'Float', value: level },
		fade_in_ms: 0,
		fade_out_ms: 0,
		delay_in_ms: 0
	});

	const cues = [
		{
			id: id(),
			name: 'House',
			number: 1,
			captures: patched.map((f) => capture(f, 0.2)),
			follow_mode: 'Manual',
			fade_in_ms: 2000,
			fade_out_ms: 2000,
			is_active: false
		},
		{
			id: id(),
			name: 'Scare',
			number: 2,
			captures: patched.map((f) => capture(f, 1.0)),
			follow_mode: 'Manual',
			fade_in_ms: 150,
			fade_out_ms: 1500,
			is_active: false
		}
	];
	for (const cue of cues) await create('cues', cue);

	const sequence = {
		id: id(),
		name: 'Haunt',
		cue_ids: cues.map((c) => c.id),
		active_cue_index: null
	};
	await create('sequences', sequence);

	// Two graphs for the Flows tab. The first is a chain anyone can set off by
	// hand; the second is the thing a one-row-per-rule trigger could never say.
	//
	// Nodes carry their own coordinates rather than being laid out from an index,
	// because an `And` has two feeders and they cannot share a row.
	const drawFlow = async (name, placed, wires) => {
		const flow = { id: id(), name, enabled: true };
		await create('flows', flow);
		const ids = [];
		for (const [kind, x, y] of placed) {
			const node = {
				id: id(),
				flow_id: flow.id,
				kind,
				x,
				y,
				active: false,
				last_fired_at: null
			};
			ids.push(node.id);
			await create('flow_nodes', node);
		}
		for (const [from, fromPort, to, toPort] of wires) {
			await create('flow_edges', {
				id: id(),
				flow_id: flow.id,
				from_node: ids[from],
				from_port: fromPort,
				to_node: ids[to],
				to_port: toPort
			});
		}
	};

	const watch = (fixture) => ({
		Source: { Parameter: { fixture_id: fixture.id, parameter: 'Intensity' } }
	});

	await drawFlow(
		'Panic button',
		[
			['Button', 40, 60],
			[{ Delay: { ms: 1500 } }, 280, 60],
			[{ Action: { GoNext: { sequence_id: sequence.id } } }, 520, 60]
		],
		[
			[0, 0, 1, 0],
			[1, 0, 2, 0]
		]
	);

	await drawFlow(
		'Both fronts up',
		[
			[watch(patched[0]), 40, 40],
			[watch(patched[1]), 40, 180],
			['And', 300, 100],
			[{ Condition: 'RisingEdge' }, 540, 100],
			[{ Action: { GoToCue: { sequence_id: sequence.id, cue_id: cues[1].id } } }, 780, 100]
		],
		// Both watches into the two inputs of the And, then edge-detect the gate:
		// it fires when the second one comes up, not when either does.
		[
			[0, 0, 2, 0],
			[1, 0, 2, 1],
			[2, 0, 3, 0],
			[3, 0, 4, 0]
		]
	);

	console.log(
		`  seeded: ${patched.length} fixtures, ${cues.length} cues, 1 sequence, 2 flows`
	);
	process.exit(0);
} catch (error) {
	console.error(`  seeding failed: ${error.message}`);
	process.exit(1);
}
