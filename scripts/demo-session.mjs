// Put the two demo stations into one session, over the ordinary WebSocket API.
//
// The same two clicks the Sessions panel makes: the first station advertises its
// show, the second waits for that advert to reach it over mDNS and joins. Run by
// scripts/demo.sh; takes the two backends' ports and the first one's sync port.
//
// When the advert never arrives — a process that may not send multicast, which on a
// Mac is a terminal never given Local Network access — the second station joins the
// first by its sync address instead (`session.joinAt`), which is what an operator
// does on a venue network whose mDNS does not reach.

import { connect, sleep } from './demo-ws.mjs';

const [LEADER_PORT = '7700', FOLLOWER_PORT = '7710', LEADER_SYNC_PORT = '7701'] = process.argv.slice(2);

const leader = connect(LEADER_PORT);
const follower = connect(FOLLOWER_PORT);

try {
	await Promise.all([leader.open, follower.open]);

	const show = await leader.get(['show']);
	if (!show) {
		// --no-seed leaves nothing to advertise. The stations still run; the
		// Sessions panel can pair them once the show has a name.
		console.log('  no show to advertise yet — pair the stations in the Sessions panel');
		process.exit(0);
	}

	const sessionId = await leader.call('session.create', { showName: show.name, showId: show.id });

	// The advert travels by mDNS, so the second station learns of it a moment
	// later rather than immediately. Joining before then is refused.
	let discovered = false;
	for (let attempt = 0; attempt < 40 && !discovered; attempt++) {
		const state = await follower.get(['session']);
		discovered = (state?.discovered ?? []).some((s) => s.session_id === sessionId);
		if (!discovered) await sleep(500);
	}
	if (discovered) {
		await follower.call('session.join', { sessionId });
		console.log(`  the second station joined ${show.name}`);
	} else {
		await follower.call('session.joinAt', { address: `127.0.0.1:${LEADER_SYNC_PORT}` });
		console.log(`  no mDNS here; the second station joined ${show.name} by its address`);
	}
	process.exit(0);
} catch (error) {
	console.error(`  pairing failed: ${error.message}`);
	process.exit(1);
}
