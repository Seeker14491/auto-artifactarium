//! Exercise the public protocol API without Irminsul or a player tracker.
use auto_artifactarium::position::{
    BuildMatching, DecodedEvent, Decoder, Event, Profile, captured_7_0, captured_7_1,
};

fn var(field: u32, mut value: u64) -> Vec<u8> {
    let mut bytes = vec![(field * 8) as u8];
    while value > 127 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}

fn msg(field: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = var(field, body.len() as u64);
    bytes[0] |= 2;
    bytes.extend(body);
    bytes
}

fn batch(command: u16, body: &[u8]) -> Vec<u8> {
    msg(11, &[var(15, command.into()), msg(8, body)].concat())
}

fn motion(entity: u64, position: bool) -> Vec<u8> {
    let mut y = vec![21];
    y.extend((-0.0_f32).to_le_bytes());
    let info = if position { msg(1, &y) } else { vec![] };
    let motion = [
        var(1, entity),
        msg(2, &[info, var(4, 2)].concat()),
        var(3, 5),
    ]
    .concat();
    msg(14, &[var(4, 7), msg(6, &motion)].concat())
}

fn ready() -> Decoder {
    let mut decoder = Decoder::default();
    let profile = captured_7_0();
    let build = profile.game_build.clone();
    decoder.set_profile(profile);
    decoder.receive(26573, &msg(8, build.as_bytes())).unwrap();
    decoder
}

#[test]
fn batches_emit_all_entities_in_wire_order_and_preserve_partial_motion() {
    let mut decoder = ready();
    let body = [
        batch(1185, &motion(77, true)),
        batch(7516, &batch(1185, &motion(42, true))),
        batch(1185, &motion(42, false)),
    ]
    .concat();
    let packet = decoder.receive(7516, &body).unwrap().unwrap();
    assert_eq!(packet.decoded_commands, 3);
    let events = packet.events;
    assert_eq!(events.len(), 3);
    let mut entities = Vec::new();
    for (index, event) in events.into_iter().enumerate() {
        let DecodedEvent::Movement {
            command,
            entity,
            motion,
            ..
        } = event
        else {
            panic!()
        };
        entities.push(entity);
        assert_eq!(command, 1185);
        if index == 2 {
            assert!(motion.unwrap().is_none());
        } else {
            let motion = motion.unwrap().unwrap();
            assert_eq!(motion.tick, 5);
            assert_eq!(motion.tick_rate_hz, 1000);
            assert_eq!(motion.position.y.to_bits(), (-0.0_f32).to_bits());
            assert!(motion.stationary.unwrap());
        }
    }
    assert_eq!(entities, [77, 42, 42]);
}

#[test]
fn build_rejection_gates_traffic_until_reset_or_a_matching_build() {
    let mut decoder = Decoder::default();
    let profile = captured_7_0();
    let build = profile.game_build.clone();
    decoder.set_profile(profile);
    assert!(!decoder.build_verified());
    assert!(matches!(
        decoder.receive(6000, &var(4, 9)).unwrap().unwrap().events[0],
        DecodedEvent::LocalUid(9)
    ));
    assert!(decoder.receive(1185, &[0]).unwrap().is_none());
    let error = decoder.receive(26573, &msg(8, b"unsupported")).unwrap_err();
    assert!(error.contains("received unsupported"));
    assert!(error.contains(&build));
    assert!(decoder.receive(6000, &var(4, 9)).unwrap().is_none());
    decoder.reset();
    assert!(!decoder.build_verified());
    assert!(decoder.receive(6000, &var(4, 9)).unwrap().is_some());
    decoder.receive(26573, &msg(8, build.as_bytes())).unwrap();
    assert!(decoder.build_verified());
}

#[test]
fn malformed_batches_return_no_events_and_roll_back_build_state() {
    let mut decoder = ready();
    let original_build = decoder.observed_build().unwrap().to_owned();
    let body = [batch(1185, &motion(42, true)), batch(1185, &[0])].concat();
    assert!(decoder.receive(7516, &body).is_err());
    let rejected_build = batch(26573, &msg(8, b"unsupported"));
    assert!(decoder.receive(7516, &rejected_build).is_err());
    assert!(decoder.build_verified());
    assert_eq!(decoder.observed_build(), Some(original_build.as_str()));
    assert_eq!(decoder.build_match(), Some(BuildMatching::Exact));
    let newer = b"OSRELWin7.0.0_R47805902_S47829085_D99999999";
    let body = [batch(26573, &msg(8, newer)), batch(1185, &[0])].concat();
    assert!(decoder.receive(7516, &body).is_err());
    assert_eq!(decoder.observed_build(), Some(original_build.as_str()));
    assert_eq!(decoder.build_match(), Some(BuildMatching::Exact));
    assert!(decoder.receive(1185, &motion(42, true)).unwrap().is_some());
}

#[test]
fn missing_build_has_a_distinct_diagnostic() {
    let mut decoder = ready();
    assert!(
        decoder
            .receive(26573, &[])
            .unwrap_err()
            .contains("build field is missing")
    );
    assert!(!decoder.build_verified());
}

#[test]
fn data_revisions_are_allowed_and_report_the_observed_build() {
    for build in [
        "OSRELWin7.0.0_R47805902_S47829085_D48139903",
        "OSRELWin7.0.0_R47805902_S47829085_D48215702",
        "OSRELWin7.0.0_R47805902_S47829085_D99999999",
    ] {
        let mut decoder = Decoder::default();
        decoder.set_profile(captured_7_0());
        let packet = decoder
            .receive(26573, &msg(8, build.as_bytes()))
            .unwrap()
            .unwrap();
        assert!(matches!(
            packet.events.as_slice(),
            [DecodedEvent::BuildVerified]
        ));
        assert!(decoder.build_verified());
        assert_eq!(decoder.observed_build(), Some(build));
        assert_eq!(
            decoder.build_match(),
            Some(if build == captured_7_0().game_build {
                BuildMatching::Exact
            } else {
                BuildMatching::DataRevision
            })
        );
        assert!(decoder.receive(1185, &motion(42, true)).unwrap().is_some());
        decoder.reset();
        assert_eq!(decoder.observed_build(), None);
        assert_eq!(decoder.build_match(), None);
        assert!(!decoder.build_verified());
    }
}

#[test]
fn changed_protocol_families_and_malformed_revisions_are_rejected() {
    for build in [
        "OSRELWin7.0.0_R47805903_S47829085_D48215702",
        "OSRELWin7.0.0_R47805902_S47829086_D48215702",
        "OSRELWin7.0.1_R47805902_S47829085_D48215702",
        "OSRELWin8.0.0_R47805902_S47829085_D48215702",
        "CNRELWin7.0.0_R47805902_S47829085_D48215702",
        "OSRELAndroid7.0.0_R47805902_S47829085_D48215702",
        "OSRELWin7.0.0_R47805902_S47829085_D",
        "OSRELWin7.0.0_R47805902_S47829085_D48215702_extra",
        "OSRELWin7.0.0_R47805902_S47829085_D+48215702",
        "OSRELWin7.0.0_R47805902_S47829085_D48215702_D3",
    ] {
        let mut decoder = ready();
        let error = decoder
            .receive(26573, &msg(8, build.as_bytes()))
            .unwrap_err();
        assert!(error.contains(build));
        assert!(error.contains("only numeric D revision may differ"));
        assert!(!decoder.build_verified());
        assert_eq!(decoder.observed_build(), Some(build));
        assert_eq!(decoder.build_match(), None);
        assert!(decoder.receive(1185, &motion(42, true)).unwrap().is_none());
    }
    for bytes in [vec![0xff], vec![b'x'; 129], b"build\nvalue".to_vec()] {
        let mut decoder = ready();
        assert!(decoder.receive(26573, &msg(8, &bytes)).is_err());
        assert_eq!(decoder.observed_build(), None);
        assert_eq!(decoder.build_match(), None);
    }
    let mut decoder = ready();
    let build = captured_7_0().game_build;
    let ambiguous = [msg(8, build.as_bytes()), msg(8, build.as_bytes())].concat();
    assert!(decoder.receive(26573, &ambiguous).is_err());
    assert_eq!(decoder.observed_build(), None);
}

#[test]
fn release_version_matching_accepts_numeric_revisions_and_preserves_exact_matches() {
    let profile = captured_7_1();
    assert!(matches!(
        profile.rules[0].event,
        Event::Build {
            matching: BuildMatching::ReleaseVersion,
            ..
        }
    ));
    for build in [
        profile.game_build.as_str(),
        "OSRELWin7.1.0_R48379043_S48511369_D48533839",
        "OSRELWin7.1.0_R1_S2_D3",
        "OSRELWin7.1.0_R0_S0_D0",
    ] {
        let mut decoder = Decoder::default();
        decoder.set_profile(profile.clone());
        let packet = decoder
            .receive(9282, &msg(11, build.as_bytes()))
            .unwrap()
            .unwrap();
        assert!(matches!(
            packet.events.as_slice(),
            [DecodedEvent::BuildVerified]
        ));
        assert!(decoder.build_verified());
        assert_eq!(decoder.observed_build(), Some(build));
        assert_eq!(
            decoder.build_match(),
            Some(if build == profile.game_build {
                BuildMatching::Exact
            } else {
                BuildMatching::ReleaseVersion
            })
        );
        decoder.reset();
        assert!(!decoder.build_verified());
        assert_eq!(decoder.build_match(), None);
    }
}

#[test]
fn release_version_matching_rejects_other_versions_platforms_and_malformed_revisions() {
    let profile = captured_7_1();
    let json = include_str!("../profiles/position-7.1.json");
    for build in [
        "OSRELWin7.0.0_R1_S2_D3",
        "OSRELWin7.1.1_R1_S2_D3",
        "OSRELWin7.2.0_R1_S2_D3",
        "OSRELWin7.1.00_R1_S2_D3",
        "OSRELWin7.1.0extra_R1_S2_D3",
        "CNRELWin7.1.0_R1_S2_D3",
        "OSRELAndroid7.1.0_R1_S2_D3",
    ] {
        let mut decoder = Decoder::default();
        decoder.set_profile(profile.clone());
        let error = decoder
            .receive(9282, &msg(11, build.as_bytes()))
            .unwrap_err();
        assert!(error.contains("release/platform must match"));
        assert!(!decoder.build_verified());
        assert_eq!(decoder.observed_build(), Some(build));
        assert_eq!(decoder.build_match(), None);
    }
    for build in [
        "OSRELWin7.1.0",
        "OSRELWin7.1.0_R1_S2",
        "OSRELWin7.1.0_R_S2_D3",
        "OSRELWin7.1.0_R1_S_D3",
        "OSRELWin7.1.0_R1_S2_D",
        "OSRELWin7.1.0_R-1_S2_D3",
        "OSRELWin7.1.0_R1_S+2_D3",
        "OSRELWin7.1.0_R1_S2_Dthree",
        "OSRELWin7.1.0_R1_S2_D3_extra",
        "OSRELWin7.1.0_R1_S2_D3_D4",
    ] {
        let mut decoder = Decoder::default();
        decoder.set_profile(profile.clone());
        assert!(decoder.receive(9282, &msg(11, build.as_bytes())).is_err());
        assert!(!decoder.build_verified());
        assert_eq!(decoder.build_match(), None);
        assert!(Profile::parse(&json.replace(&profile.game_build, build)).is_err());
    }
    let mut decoder = Decoder::default();
    decoder.set_profile(profile.clone());
    let body = msg(11, profile.game_build.as_bytes());
    assert!(
        decoder
            .receive(9282, &[body.clone(), body].concat())
            .is_err()
    );
    assert_eq!(decoder.observed_build(), None);
    assert!(!decoder.build_verified());
}

#[test]
fn legacy_json_defaults_to_exact_and_explicit_allowlists_still_work() {
    let json = include_str!("../profiles/position-7.0.json");
    let exact_json = json.replace(",\n      \"matching\": \"data_revision\"", "");
    let mut profile = Profile::parse(&exact_json).unwrap();
    assert!(matches!(
        profile.rules[0].event,
        Event::Build {
            matching: BuildMatching::Exact,
            ..
        }
    ));
    let build = profile.game_build.clone();
    let newer = "OSRELWin7.0.0_R47805902_S47829085_D99999999";
    let mut decoder = Decoder::default();
    decoder.set_profile(profile.clone());
    decoder.receive(26573, &msg(8, build.as_bytes())).unwrap();
    assert!(decoder.receive(26573, &msg(8, newer.as_bytes())).is_err());
    let mut additional = profile.rules[0].clone();
    if let Event::Build { expected, .. } = &mut additional.event {
        *expected = newer.into();
    }
    profile.rules.push(additional);
    decoder.set_profile(profile);
    decoder.receive(26573, &msg(8, newer.as_bytes())).unwrap();
    assert_eq!(decoder.build_match(), Some(BuildMatching::Exact));
    assert!(Profile::parse(&json.replace("data_revision", "any")).is_err());
    assert!(Profile::parse(&json.replace("D48215702", "Dgarbage")).is_err());
}

#[test]
fn shuffled_commands_and_inner_fields_need_only_a_new_json_profile() {
    let profile = Profile::parse(include_str!("fixtures/shuffled-profile.json")).unwrap();
    let mut decoder = Decoder::default();
    decoder.set_profile(profile);
    decoder
        .receive(61000, &msg(9, b"OSRELWin8.0.0_R1_S2_D3"))
        .unwrap();
    let uid = decoder.receive(61001, &var(10, 9)).unwrap().unwrap();
    assert!(matches!(uid.events.as_slice(), [DecodedEvent::LocalUid(9)]));
    let team = msg(4, &[var(11, 9), var(7, 1), var(5, 42), var(6, 3)].concat());
    let mut vector = vec![8 * 8 + 5];
    vector.extend(12.5_f32.to_le_bytes());
    let movement = msg(
        9,
        &[
            var(8, 7),
            msg(
                5,
                &[
                    var(2, 42),
                    msg(6, &[msg(4, &vector), var(3, 2)].concat()),
                    var(7, 5),
                ]
                .concat(),
            ),
        ]
        .concat(),
    );
    let nested = |command, body: &[u8]| msg(3, &[var(2, command), msg(10, body)].concat());
    let packet = decoder
        .receive(
            61003,
            &[nested(61002, &team), nested(61004, &movement)].concat(),
        )
        .unwrap()
        .unwrap();
    let [
        DecodedEvent::Team(team),
        DecodedEvent::Movement { entity, motion, .. },
    ] = packet.events.as_slice()
    else {
        panic!()
    };
    let member = team[0].as_ref().unwrap();
    assert_eq!(member.owner_uid, Some(9));
    assert_eq!(member.member.as_ref().unwrap().scene, 3);
    assert_eq!(member.member.as_ref().unwrap().entity, Some(*entity));
    let motion = motion.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(motion.position.y, 12.5);
    assert_eq!(motion.position.x, 0.);
    assert_eq!(motion.tick, 5);
    assert_eq!(motion.tick_rate_hz, 60);
    assert!(*motion.stationary.as_ref().unwrap());
}

#[test]
fn cross_scene_team_announcement_precedes_avatar_entities() {
    let mut decoder = Decoder::default();
    let profile = captured_7_1();
    let build = profile.game_build.clone();
    decoder.set_profile(profile);
    decoder.receive(9282, &msg(11, build.as_bytes())).unwrap();
    // Observed 7.1 layout: the first update after a jump has scene but no entity.
    let entry = |entity: Option<u64>| {
        let entity = entity.map_or(vec![], |entity| var(8, entity));
        msg(8, &[var(2, 1), var(6, 3), entity, var(13, 9)].concat())
    };
    let mut members = |body: &[u8]| {
        let packet = decoder.receive(29245, body).unwrap().unwrap();
        let [DecodedEvent::Team(team)] = packet.events.as_slice() else {
            panic!()
        };
        team.iter()
            .map(|entry| entry.as_ref().unwrap().member.clone())
            .collect::<Vec<_>>()
    };
    let pending = members(&entry(None));
    assert_eq!(pending[0].as_ref().unwrap().scene, 3);
    assert_eq!(pending[0].as_ref().unwrap().entity, None);
    let spawned = members(&entry(Some(42)));
    assert_eq!(spawned[0].as_ref().unwrap().entity, Some(42));
    // A transmitted zero is malformed rather than pending.
    let zero = members(&entry(Some(0)));
    assert_eq!(zero[0].as_ref().unwrap_err(), "Zero scene/entity/UID");
}

#[test]
fn empty_team_is_distinct_from_an_unknown_command() {
    let mut decoder = ready();
    assert!(decoder.receive(65535, &[0]).unwrap().is_none());
    let events = decoder.receive(7886, &[]).unwrap().unwrap().events;
    assert!(matches!(&events[..], [DecodedEvent::Team(entries)] if entries.is_empty()));
    let packet = decoder.receive(7516, &batch(65535, &[0])).unwrap().unwrap();
    assert_eq!(packet.decoded_commands, 0);
    assert!(packet.events.is_empty());
    let packet = decoder.receive(1185, &[]).unwrap().unwrap();
    assert_eq!(packet.decoded_commands, 1);
    assert!(packet.events.is_empty());
}
