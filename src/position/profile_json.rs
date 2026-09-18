use serde_json::{Map, Value};

use super::schema::{BuildMatching, Event, Filter, Profile, Rule};

fn object(value: Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err("Expected JSON object".into()),
    }
}
fn take<T: serde::de::DeserializeOwned>(
    map: &mut Map<String, Value>,
    key: &str,
) -> Result<T, String> {
    let value = map.remove(key).ok_or_else(|| format!("Missing {key}"))?;
    serde_json::from_value(value).map_err(|e| format!("Invalid {key}: {e}"))
}
fn no_unknown(map: Map<String, Value>) -> Result<(), String> {
    if map.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Unknown profile fields: {}",
            map.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    }
}
impl Profile {
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.len() > 65536 {
            return Err("Profile exceeds 64 KiB".into());
        }
        let mut map = object(serde_json::from_str(json).map_err(|e| e.to_string())?)?;
        let name = take(&mut map, "name")?;
        let game_build = take(&mut map, "game_build")?;
        let values: Vec<Value> = take(&mut map, "rules")?;
        no_unknown(map)?;
        let mut rules = Vec::new();
        for value in values {
            let mut map = object(value)?;
            let command_id = take(&mut map, "command_id")?;
            let entries = if map.contains_key("entries") {
                take(&mut map, "entries")?
            } else {
                Vec::new()
            };
            let filters: Vec<Value> = if map.contains_key("filters") {
                take(&mut map, "filters")?
            } else {
                Vec::new()
            };
            let filters = filters
                .into_iter()
                .map(|v| {
                    let mut map = object(v)?;
                    let filter = Filter {
                        path: take(&mut map, "path")?,
                        equals: take(&mut map, "equals")?,
                    };
                    no_unknown(map)?;
                    Ok(filter)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let event: String = take(&mut map, "event")?;
            let event = match event.as_str() {
                "local_uid" => Event::LocalUid {
                    uid: take(&mut map, "uid")?,
                },
                "scene" => Event::Scene {
                    scene: take(&mut map, "scene")?,
                },
                "active_entity" => Event::ActiveEntity {
                    entity: take(&mut map, "entity")?,
                    owner_uid: if map.contains_key("owner_uid") {
                        Some(take(&mut map, "owner_uid")?)
                    } else {
                        None
                    },
                },
                "movement" => Event::Movement {
                    entity: take(&mut map, "entity")?,
                    position: take(&mut map, "position")?,
                    tick: take(&mut map, "tick")?,
                    tick_rate_hz: take(&mut map, "tick_rate_hz")?,
                    axes: take(&mut map, "axes")?,
                    stationary: if let Some(v) = map.remove("stationary") {
                        let mut m = object(v)?;
                        let f = Filter {
                            path: take(&mut m, "path")?,
                            equals: take(&mut m, "equals")?,
                        };
                        no_unknown(m)?;
                        Some(f)
                    } else {
                        None
                    },
                },
                "local_team" => Event::LocalTeam {
                    scene: take(&mut map, "scene")?,
                    entity: take(&mut map, "entity")?,
                    owner_uid: take(&mut map, "owner_uid")?,
                    active: take(&mut map, "active")?,
                },
                "replace_entity" => Event::ReplaceEntity {
                    previous: take(&mut map, "previous")?,
                    entity: take(&mut map, "entity")?,
                    owner_uid: take(&mut map, "owner_uid")?,
                },
                "union" => Event::Union {
                    command: take(&mut map, "command")?,
                    body: take(&mut map, "body")?,
                },
                "build" => Event::Build {
                    path: take(&mut map, "path")?,
                    expected: take(&mut map, "expected")?,
                    matching: if map.contains_key("matching") {
                        match take::<String>(&mut map, "matching")?.as_str() {
                            "exact" => BuildMatching::Exact,
                            "data_revision" => BuildMatching::DataRevision,
                            other => return Err(format!("Unknown build matching policy {other}")),
                        }
                    } else {
                        BuildMatching::Exact
                    },
                },
                "invalidate_reading" => Event::InvalidateReading,
                "invalidate" => Event::Invalidate,
                _ => return Err(format!("Unknown event {event}")),
            };
            no_unknown(map)?;
            rules.push(Rule {
                command_id,
                entries,
                filters,
                event,
            });
        }
        let profile = Profile {
            name,
            game_build,
            rules,
        };
        profile.validate()?;
        Ok(profile)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_fixture_and_unknown_fields() {
        let fixture = include_str!("../../tests/fixtures/synthetic-profile.json");
        assert!(Profile::parse(fixture).is_ok());
        assert!(Profile::parse(&fixture.replace("\"scene\":", "\"typo\":")).is_err());
        assert!(Profile::parse(&fixture.replace("60001", "0")).is_err());
        assert!(Profile::parse(&fixture.replace("[1, 2, 3]", "[1, 1, 3]")).is_err());
    }

    #[test]
    fn movement_requires_a_valid_explicit_tick_rate() {
        let fixture = include_str!("../../tests/fixtures/synthetic-profile.json");
        assert!(Profile::parse(&fixture.replace("\"tick_rate_hz\": 2, ", "")).is_err());
        for rate in ["0", "-1", "1.5", "1000000001", "null"] {
            assert!(
                Profile::parse(
                    &fixture.replace("\"tick_rate_hz\": 2", &format!("\"tick_rate_hz\": {rate}"))
                )
                .is_err()
            );
        }
        assert!(
            Profile::parse(&fixture.replace("\"tick_rate_hz\": 2", "\"tick_rate_hz\": 60")).is_ok()
        );
    }
}
