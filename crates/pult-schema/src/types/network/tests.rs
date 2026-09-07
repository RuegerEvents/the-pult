use super::*;

fn iface(name: &str, addresses: &[&str]) -> NetInterface {
    NetInterface {
        name: name.to_string(),
        addresses: addresses.iter().map(|a| a.to_string()).collect(),
        up: true,
        loopback: name.starts_with("lo"),
    }
}

fn machine() -> Vec<NetInterface> {
    vec![
        iface("lo0", &["127.0.0.1"]),
        iface("en0", &["192.168.1.20"]),
        // The aliased card: an Art-Net network beside a house one, which is the
        // case a name alone cannot express.
        iface("en5", &["10.0.1.7", "2.0.0.1"]),
        iface("en9", &[]),
    ]
}

fn node(byte: u8) -> NodeId {
    NodeId(Uuid::from_bytes([byte; 16]))
}

#[test]
fn a_name_resolves_to_its_first_address() {
    assert_eq!(resolve("en0", &machine()), Ok("192.168.1.20".parse().unwrap()));
    assert_eq!(resolve("en5", &machine()), Ok("10.0.1.7".parse().unwrap()));
}

#[test]
fn an_address_literal_picks_the_alias_a_name_cannot() {
    // The whole reason both forms are accepted: `en5` cannot say which of the two.
    assert_eq!(resolve("2.0.0.1", &machine()), Ok("2.0.0.1".parse().unwrap()));
}

#[test]
fn surrounding_space_is_not_a_different_interface() {
    assert_eq!(resolve("  en0 ", &machine()), Ok("192.168.1.20".parse().unwrap()));
}

#[test]
fn an_address_that_is_not_here_is_refused_rather_than_bound() {
    // Being told something wrong, not being told nothing: the caller must not fall
    // back to every interface on this.
    assert_eq!(
        resolve("10.9.9.9", &machine()),
        Err(InterfaceError::NotHere("10.9.9.9".into()))
    );
}

#[test]
fn a_name_the_machine_has_not_got_is_refused() {
    assert_eq!(resolve("eth7", &machine()), Err(InterfaceError::Unknown("eth7".into())));
}

#[test]
fn an_interface_with_no_ipv4_is_its_own_answer() {
    // Distinguishable from Unknown on purpose: the cable is in, the address is not
    // there yet, and that is a different thing to tell somebody.
    assert_eq!(resolve("en9", &machine()), Err(InterfaceError::NoAddress("en9".into())));
}

#[test]
fn ipv6_is_refused_by_name() {
    // Never accepted and bound to nothing, which is what the old code would have
    // done with it.
    assert!(matches!(resolve("::1", &machine()), Err(InterfaceError::NotIpv4(_))));
    assert!(matches!(
        resolve("fe80::1%en0", &machine()),
        Err(InterfaceError::Unknown(_)) | Err(InterfaceError::NotIpv4(_))
    ));
}

#[test]
fn loopback_can_be_named_like_anything_else() {
    // Flagged rather than excluded, unlike the throughput figures beside it: a dev
    // station binds this deliberately.
    assert_eq!(resolve("lo0", &machine()), Ok("127.0.0.1".parse().unwrap()));
    assert!(machine().iter().find(|i| i.name == "lo0").unwrap().loopback);
}

// ── the priority ladder ───────────────────────────────────────────────────────

#[test]
fn the_ladder_steps_down_to_a_floor_and_stops() {
    let slots: Vec<u8> = sacn_slots().collect();
    assert_eq!(slots, vec![90, 80, 70, 60, 50, 40, 30, 20, 10]);
    // Never zero: E1.31 reads 0 as "do not use this source", so a tenth station
    // would be silenced rather than ranked.
    assert!(!slots.contains(&0));
}

#[test]
fn the_first_follower_takes_the_top_slot() {
    assert_eq!(claim_slot(node(2), None, &[]), 90);
}

#[test]
fn a_station_defers_only_to_lower_node_ids() {
    let others = [(node(1), 90u8)];
    assert_eq!(claim_slot(node(2), None, &others), 80);
    // And the lower id is blocked by nobody, so it does not move for the higher one.
    assert_eq!(claim_slot(node(1), Some(90), &[(node(2), 90)]), 90);
}

#[test]
fn a_joining_station_does_not_renumber_anybody() {
    // The whole reason the slots are sticky. B and C are settled; D joins with the
    // highest id and takes what is free, and neither of them moves.
    let settled = [(node(2), 90u8), (node(3), 80u8)];
    assert_eq!(claim_slot(node(9), None, &settled), 70);
    assert_eq!(claim_slot(node(2), Some(90), &[(node(3), 80), (node(9), 70)]), 90);
    assert_eq!(claim_slot(node(3), Some(80), &[(node(2), 90), (node(9), 70)]), 80);
}

#[test]
fn a_remembered_slot_survives_a_restart() {
    // A console that reboots mid-show comes back at the priority the receivers last
    // heard it at, rather than at whatever gap it left behind.
    let others = [(node(1), 90u8), (node(3), 70u8)];
    assert_eq!(claim_slot(node(2), Some(80), &others), 80);
}

#[test]
fn a_collision_resolves_by_node_id_and_converges() {
    // Two stations claim 90 in the same instant. The higher id sees the other's row
    // on the next tick and moves; the lower one stays put.
    let higher = claim_slot(node(5), Some(90), &[(node(1), 90)]);
    let lower = claim_slot(node(1), Some(90), &[(node(5), 90)]);
    assert_eq!(lower, 90);
    assert_eq!(higher, 80);
    assert_ne!(lower, higher);
}

#[test]
fn a_nonsense_remembered_slot_is_ignored() {
    // 100 is the leader's and 0 is "do not use": neither is a slot to come back to.
    assert_eq!(claim_slot(node(2), Some(100), &[]), 90);
    assert_eq!(claim_slot(node(2), Some(0), &[]), 90);
}

#[test]
fn past_the_tenth_station_the_floor_is_shared() {
    let full: Vec<(NodeId, u8)> =
        sacn_slots().enumerate().map(|(i, slot)| (node(i as u8 + 1), slot)).collect();
    assert_eq!(claim_slot(node(200), None, &full), SACN_PRIORITY_FLOOR);
}

// ── what a station actually sends ─────────────────────────────────────────────

#[test]
fn auto_puts_the_leader_at_the_top() {
    assert_eq!(SacnPriority::Auto.byte_for(node(1), 90, true), 100);
    assert_eq!(SacnPriority::Auto.byte_for(node(2), 90, false), 90);
}

#[test]
fn a_manual_map_names_the_stations_it_means() {
    let mut map = BTreeMap::new();
    map.insert(node(1), 150);
    let priority = SacnPriority::Manual(map);
    // Named: the number, so this desk outranks a house console already at 100.
    assert_eq!(priority.byte_for(node(1), 90, true), 150);
    // Unnamed: the Auto number, not a bare default — being unnamed is being told
    // nothing, and two unnamed followers both at 100 is the tie the ladder exists
    // to avoid.
    assert_eq!(priority.byte_for(node(2), 90, false), 90);
    assert_eq!(priority.byte_for(node(3), 80, false), 80);
}

#[test]
fn a_manual_number_is_clamped_to_what_the_protocol_has() {
    let mut map = BTreeMap::new();
    map.insert(node(1), 255);
    assert_eq!(SacnPriority::Manual(map).byte_for(node(1), 90, false), SACN_PRIORITY_MAX);
}

#[test]
fn nothing_said_is_auto() {
    assert_eq!(SacnPriority::default(), SacnPriority::Auto);
}
