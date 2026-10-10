use auto_artifactarium::position::{
    DecodedEvent, Decoder, Event, IntegerOp, Profile, captured_7_0,
};

fn integer(mut value: u64) -> Vec<u8> {
    let mut bytes = vec![];
    while value > 127 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}
fn var(field: u32, value: u64) -> Vec<u8> {
    [integer(u64::from(field) * 8), integer(value)].concat()
}
fn msg(field: u32, body: &[u8]) -> Vec<u8> {
    [
        integer(u64::from(field) * 8 + 2),
        integer(body.len() as u64),
        body.to_vec(),
    ]
    .concat()
}
fn float(field: u32, value: f32) -> Vec<u8> {
    [
        integer(u64::from(field) * 8 + 5),
        value.to_le_bytes().to_vec(),
    ]
    .concat()
}
fn decoder(profile: Profile) -> Decoder {
    let build = profile.game_build.clone();
    let mut decoder = Decoder::default();
    decoder.set_profile(profile);
    decoder.receive(26573, &msg(8, build.as_bytes())).unwrap();
    decoder
}
fn motion(info: &[u8]) -> Vec<u8> {
    msg(
        14,
        &[
            var(4, 7),
            msg(6, &[var(1, 42), msg(2, info), var(3, 1)].concat()),
        ]
        .concat(),
    )
}

#[test]
fn metadata_preserves_bits_unknown_states_and_partial_motion() {
    let mut decoder = decoder(captured_7_0());
    for position in [false, true] {
        let pos = if position { msg(1, &[]) } else { vec![] };
        let body = motion(&[pos, msg(3, &float(2, -0.0)), var(4, 38)].concat());
        let packet = decoder.receive(1185, &body).unwrap().unwrap();
        let DecodedEvent::Movement {
            motion, metadata, ..
        } = &packet.events[0]
        else {
            panic!()
        };
        assert_eq!(motion.as_ref().unwrap().is_some(), position);
        let metadata = metadata.as_ref().unwrap();
        assert_eq!(metadata.state, Some(38));
        assert_eq!(metadata.state_name.as_deref(), Some("LAND_SPEED"));
        assert_eq!(metadata.velocity.unwrap().y.to_bits(), (-0.0f32).to_bits());
    }
    let packet = decoder
        .receive(1185, &motion(&var(4, 65000)))
        .unwrap()
        .unwrap();
    let DecodedEvent::Movement { metadata, .. } = &packet.events[0] else {
        panic!()
    };
    assert_eq!(metadata.as_ref().unwrap().state, Some(65000));
    assert!(metadata.as_ref().unwrap().state_name.is_none());
    assert!(metadata.as_ref().unwrap().velocity.is_none());
    let packet = decoder
        .receive(1185, &motion(&msg(3, &[])))
        .unwrap()
        .unwrap();
    let DecodedEvent::Movement { metadata, .. } = &packet.events[0] else {
        panic!()
    };
    assert_eq!(metadata.as_ref().unwrap().velocity.unwrap().x, 0.);
    assert!(metadata.as_ref().unwrap().state.is_none());
}

#[test]
fn malformed_optional_data_stays_deferred_until_entity_selection() {
    let mut decoder = decoder(captured_7_0());
    for velocity in [
        float(2, f32::NAN),
        float(5, 2.),
        [float(1, 1.), float(1, 2.)].concat(),
    ] {
        let packet = decoder
            .receive(1185, &motion(&[msg(1, &[]), msg(3, &velocity)].concat()))
            .unwrap()
            .unwrap();
        let DecodedEvent::Movement {
            motion, metadata, ..
        } = &packet.events[0]
        else {
            panic!()
        };
        assert!(motion.as_ref().unwrap().is_some());
        assert!(metadata.is_err());
    }
}

#[test]
fn transformed_scene_ids_wrap_in_order_and_keep_missing_distinct() {
    let mut profile = captured_7_0();
    profile.rules[2].event = Event::Scene {
        scene: vec![2],
        decode: vec![IntegerOp::Xor(11523), IntegerOp::Add(49379)],
    };
    let mut decoder = decoder(profile);
    let encoded = 3u32.wrapping_sub(49379) ^ 11523;
    let packet = decoder
        .receive(9342, &var(2, encoded.into()))
        .unwrap()
        .unwrap();
    assert!(matches!(packet.events[0], DecodedEvent::Scene(3)));
    assert!(decoder.receive(9342, &[]).is_err());
    assert!(
        decoder
            .receive(9342, &var(2, u64::from(u32::MAX) + 1))
            .is_err()
    );
    assert_eq!(IntegerOp::Add(1).apply(u32::MAX), 0);
    assert_eq!(IntegerOp::Sub(1).apply(0), u32::MAX);
    // Raw zero is valid when an explicit transform produces a nonzero scene.
    let mut profile = captured_7_0();
    profile.rules[2].event = Event::Scene {
        scene: vec![2],
        decode: vec![IntegerOp::Add(1)],
    };
    let mut zero = self::decoder(profile);
    assert!(matches!(
        zero.receive(9342, &var(2, 0)).unwrap().unwrap().events[0],
        DecodedEvent::Scene(1)
    ));
}

#[test]
fn json_extensions_are_strict_and_old_profiles_remain_valid() {
    let mut profile: serde_json::Value =
        serde_json::from_str(include_str!("../profiles/position-7.0.json")).unwrap();
    profile["rules"][2]["event"] = "scene".into();
    profile["rules"][2]["scene"] = serde_json::json!([2]);
    profile["rules"][2]["decode"] =
        serde_json::json!([{"op":"xor","value":11523},{"op":"add","value":49379}]);
    assert!(Profile::parse(&profile.to_string()).is_ok());
    for invalid in [
        serde_json::json!([{"op":"eval","value":1}]),
        serde_json::json!([{"op":"xor","value":4294967296u64}]),
        serde_json::json!([{"op":"add","value":1,"typo":true}]),
    ] {
        profile["rules"][2]["decode"] = invalid;
        assert!(Profile::parse(&profile.to_string()).is_err());
    }
    assert!(Profile::parse(include_str!("fixtures/shuffled-profile.json")).is_ok());
    let mut profile = captured_7_0();
    if let Event::Movement { motion, .. } = &mut profile.rules[7].event {
        motion.state = None;
    }
    assert!(profile.validate().is_err());
}
