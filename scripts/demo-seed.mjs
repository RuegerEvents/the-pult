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
		created_at: new Date().toISOString(),
		is_running: false
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

	const fixtures = ['Front left', 'Front right', 'Backlight'];
	for (const [index, name] of fixtures.entries()) {
		await create('fixtures', {
			id: id(),
			name,
			fixture_type_id: dimmer.id,
			address: { Dmx: { universe: 1, address: 1 + index } },
			position: null,
			live_values: {},
			active_preset: null
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

	await create('sequences', {
		id: id(),
		name: 'Haunt',
		cue_ids: cues.map((c) => c.id),
		active_cue_index: null
	});

	console.log(`  seeded: ${fixtures.length} fixtures, ${cues.length} cues, 1 sequence`);
	process.exit(0);
} catch (error) {
	console.error(`  seeding failed: ${error.message}`);
	process.exit(1);
}
