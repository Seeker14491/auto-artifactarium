use std::sync::Arc;

use super::schema::{BuildMatching, Event, MotionFields, Profile};
use super::wire::{Position, messages, scalar, vector};

pub type DecodeResult<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct OwnedEntity {
    pub owner_uid: Option<u64>,
    pub entity: DecodeResult<u32>,
}

#[derive(Debug, Clone)]
pub struct TeamMember {
    pub scene: u32,
    /// None until the avatar spawns: cross-scene jumps announce the team first.
    pub entity: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TeamEntry {
    pub owner_uid: Option<u64>,
    pub active: DecodeResult<bool>,
    pub member: DecodeResult<TeamMember>,
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub previous: Option<u64>,
    pub next: DecodeResult<OwnedEntity>,
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub position: Position,
    pub tick: u64,
    pub tick_rate_hz: u32,
    /// Evaluated by the consumer only for a selected, accepted motion sample.
    pub stationary: DecodeResult<bool>,
}

/// Optional transmitted data. Missing fields remain absent, including on partial motion.
#[derive(Debug, Clone, Default)]
pub struct MotionMetadata {
    pub state: Option<u32>,
    pub state_name: Option<String>,
    pub velocity: Option<Position>,
}

/// Typed protocol events. No account selection, timestamps, or measurement state.
#[derive(Debug, Clone)]
pub enum DecodedEvent {
    BuildVerified,
    LocalUid(u32),
    Scene(u32),
    ActiveEntities {
        check_owner: bool,
        entries: Vec<DecodeResult<OwnedEntity>>,
    },
    Team(Vec<DecodeResult<TeamEntry>>),
    Replacements(Vec<DecodeResult<Replacement>>),
    Movement {
        command: u16,
        entity: u32,
        /// None means a partial motion update with no transmitted position.
        motion: DecodeResult<Option<Motion>>,
        metadata: DecodeResult<MotionMetadata>,
    },
    InvalidateReading,
    Invalidate,
}

#[derive(Debug, Clone, Default)]
pub struct DecodedPacket {
    pub events: Vec<DecodedEvent>,
    /// Successfully decoded leaf commands, excluding batch wrappers. A command
    /// can be valid yet contain no matching entries; unknown commands count as 0.
    pub decoded_commands: usize,
}

/// Keeps only profile/build negotiation state. Clone before applying events if
/// the consuming tracker can reject a packet, then commit both states together.
#[derive(Default, Clone)]
pub struct Decoder {
    profile: Option<Arc<Profile>>,
    build_verified: bool,
    build_rejected: bool,
    observed_build: Option<String>,
    build_match: Option<BuildMatching>,
}

impl Decoder {
    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = Some(Arc::new(profile));
        self.reset();
    }

    pub fn reset(&mut self) {
        self.build_rejected = false;
        self.observed_build = None;
        self.build_match = None;
        self.build_verified = !self.profile.as_ref().is_some_and(|p| {
            p.rules
                .iter()
                .any(|r| matches!(r.event, Event::Build { .. }))
        });
    }

    pub fn build_verified(&self) -> bool {
        self.build_verified
    }

    pub fn observed_build(&self) -> Option<&str> {
        self.observed_build.as_deref()
    }

    /// Exact reference build or a D-only match; None until a guard succeeds.
    pub fn build_match(&self) -> Option<BuildMatching> {
        self.build_match
    }

    /// Returns None for unknown commands or traffic gated by build validation.
    /// A recognized packet is decoded before its events are returned. Malformed
    /// batches return no partial events and roll back build negotiation changes.
    pub fn receive(&mut self, command: u16, body: &[u8]) -> DecodeResult<Option<DecodedPacket>> {
        self.receive_depth(command, body, 0)
    }

    fn receive_depth(
        &mut self,
        command: u16,
        body: &[u8],
        depth: usize,
    ) -> DecodeResult<Option<DecodedPacket>> {
        if depth > 4 {
            return Err("Nested command depth exceeded".into());
        }
        let Some(profile) = &self.profile else {
            return Ok(None);
        };
        let rules: Vec<_> = profile
            .rules
            .iter()
            .filter(|r| r.command_id == command)
            .collect();
        if rules.is_empty() {
            return Ok(None);
        }
        // Each rule declares its compatibility policy. No unrestricted wildcards.
        let mut build_failures = Vec::new();
        for rule in &rules {
            if let Event::Build {
                path,
                expected,
                matching,
            } = &rule.event
            {
                let builds = messages(body, path);
                let actual = builds
                    .as_ref()
                    .ok()
                    .filter(|values| values.len() == 1)
                    .and_then(|values| std::str::from_utf8(values[0]).ok())
                    .filter(|value| {
                        !value.is_empty()
                            && value.len() <= 128
                            && value.bytes().all(|b| b.is_ascii_graphic())
                    });
                self.observed_build = actual.map(str::to_owned);
                self.build_match = actual.and_then(|actual| matching.match_build(expected, actual));
                if self.build_match.is_some() {
                    self.build_verified = true;
                    self.build_rejected = false;
                    return Ok(Some(DecodedPacket {
                        events: vec![DecodedEvent::BuildVerified],
                        decoded_commands: 1,
                    }));
                }
                let received = match builds {
                    Ok(values) if values.len() == 1 => std::str::from_utf8(values[0])
                        .ok()
                        .filter(|v| v.len() <= 128 && v.chars().all(|c| c.is_ascii_graphic()))
                        .map(|v| format!("received {v}"))
                        .unwrap_or_else(|| "received an invalid build value".into()),
                    Ok(values) if values.is_empty() => "build field is missing".into(),
                    Ok(_) => "received multiple build values".into(),
                    Err(_) => "build packet is malformed".into(),
                };
                let policy = match matching {
                    BuildMatching::Exact => "exact match",
                    BuildMatching::DataRevision => {
                        "only numeric D revision may differ; release/platform, R and S must match"
                    }
                    BuildMatching::ReleaseVersion => {
                        "numeric R, S and D revisions may differ; release/platform must match"
                    }
                };
                build_failures.push(format!("{received}; expected {expected} ({policy})"));
            }
        }
        if !build_failures.is_empty() {
            self.build_verified = false;
            self.build_rejected = true;
            return Err(format!(
                "Unsupported or missing game build: {}",
                build_failures.join(" or ")
            ));
        }
        if self.build_rejected
            || (!self.build_verified
                && !rules
                    .iter()
                    .all(|r| matches!(r.event, Event::LocalUid { .. })))
        {
            return Ok(None);
        }
        for rule in &rules {
            if let Event::Union {
                command: command_path,
                body: body_path,
            } = &rule.event
            {
                let mut nested = Vec::new();
                for entry in messages(body, &rule.entries)? {
                    let id = u16::try_from(required_u32(entry, command_path)?)
                        .map_err(|_| "Command ID overflow")?;
                    let bodies = messages(entry, body_path)?;
                    if bodies.len() != 1 {
                        return Err("Ambiguous nested command body".into());
                    }
                    nested.push((id, bodies[0]));
                }
                let mut transaction = self.clone();
                let mut packet = DecodedPacket::default();
                for (id, payload) in nested {
                    if let Some(decoded) = transaction.receive_depth(id, payload, depth + 1)? {
                        packet.events.extend(decoded.events);
                        packet.decoded_commands += decoded.decoded_commands;
                    }
                }
                *self = transaction;
                return Ok(Some(packet));
            }
        }
        let result = (|| {
            let mut events = Vec::new();
            for rule in rules {
                let mut identities = Vec::new();
                let mut team = Vec::new();
                let mut replacements = Vec::new();
                for entry in messages(body, &rule.entries)? {
                    let mut matches = true;
                    for filter in &rule.filters {
                        if scalar(entry, &filter.path)?.unwrap_or(0) != filter.equals {
                            matches = false;
                            break;
                        }
                    }
                    if !matches {
                        continue;
                    }
                    match &rule.event {
                        Event::LocalUid { uid } => {
                            events.push(DecodedEvent::LocalUid(required_u32(entry, uid)?))
                        }
                        Event::Scene { scene, decode } => {
                            let raw =
                                scalar(entry, scene)?.ok_or("Required scene integer missing")?;
                            let mut value =
                                u32::try_from(raw).map_err(|_| "Scene integer exceeds u32")?;
                            for op in decode {
                                value = op.apply(value);
                            }
                            if value == 0 {
                                return Err("Zero scene/entity/UID".into());
                            }
                            events.push(DecodedEvent::Scene(value))
                        }
                        Event::ActiveEntity { entity, owner_uid } => {
                            identities.push(owned_entity(entry, entity, owner_uid.as_deref()));
                        }
                        Event::LocalTeam {
                            scene,
                            entity,
                            owner_uid,
                            active,
                        } => {
                            team.push((|| {
                                Ok(TeamEntry {
                                    owner_uid: scalar(entry, owner_uid)?,
                                    active: scalar(entry, active).map(|v| v.unwrap_or(0) == 1),
                                    member: (|| {
                                        Ok(TeamMember {
                                            scene: required_u32(entry, scene)?,
                                            entity: optional_u32(entry, entity)?,
                                        })
                                    })(),
                                })
                            })());
                        }
                        Event::ReplaceEntity {
                            previous,
                            entity,
                            owner_uid,
                        } => {
                            replacements.push(scalar(entry, previous).map(|previous| {
                                Replacement {
                                    previous,
                                    next: owned_entity(entry, entity, Some(owner_uid)),
                                }
                            }));
                        }
                        Event::Movement {
                            entity,
                            position,
                            tick,
                            tick_rate_hz,
                            axes,
                            stationary,
                            motion: motion_fields,
                        } => {
                            events.push(DecodedEvent::Movement {
                                command,
                                entity: required_u32(entry, entity)?,
                                metadata: motion_metadata(entry, motion_fields, *axes),
                                motion: (|| {
                                    let positions = messages(entry, position)?;
                                    if positions.is_empty() {
                                        return Ok(None);
                                    }
                                    if positions.len() != 1 {
                                        return Err("Expected exactly one position message".into());
                                    }
                                    Ok(Some(Motion {
                                        position: vector(positions[0], *axes)?,
                                        tick: scalar(entry, tick)?.unwrap_or(0),
                                        tick_rate_hz: *tick_rate_hz,
                                        stationary: stationary.as_ref().map_or(Ok(true), |f| {
                                            scalar(entry, &f.path)
                                                .map(|v| v.unwrap_or(0) == f.equals)
                                        }),
                                    }))
                                })(),
                            });
                        }
                        Event::Invalidate => events.push(DecodedEvent::Invalidate),
                        Event::InvalidateReading => events.push(DecodedEvent::InvalidateReading),
                        Event::Build { .. } | Event::Union { .. } => unreachable!(),
                    }
                }
                // Preserve empty lists and rule boundaries for identity invalidation
                // and ambiguity checks by the consuming player tracker.
                match &rule.event {
                    Event::ActiveEntity { owner_uid, .. } => {
                        events.push(DecodedEvent::ActiveEntities {
                            check_owner: owner_uid.is_some(),
                            entries: identities,
                        })
                    }
                    Event::LocalTeam { .. } => events.push(DecodedEvent::Team(team)),
                    Event::ReplaceEntity { .. } => {
                        events.push(DecodedEvent::Replacements(replacements))
                    }
                    _ => (),
                }
            }
            Ok(Some(DecodedPacket {
                events,
                decoded_commands: 1,
            }))
        })();
        result.map_err(|error: String| format!("Command {command}: {error}"))
    }
}

fn motion_metadata(
    bytes: &[u8],
    mapping: &MotionFields,
    axes: [u32; 3],
) -> DecodeResult<MotionMetadata> {
    let state = mapping
        .state
        .as_ref()
        .map(|path| scalar(bytes, path))
        .transpose()?
        .flatten()
        .map(u32::try_from)
        .transpose()
        .map_err(|_| "Motion state exceeds u32")?;
    let velocity = if let Some(path) = &mapping.velocity {
        let values = messages(bytes, path)?;
        match values.as_slice() {
            [] => None,
            [value] => Some(vector(value, axes)?),
            _ => return Err("Expected at most one velocity message".into()),
        }
    } else {
        None
    };
    Ok(MotionMetadata {
        state,
        state_name: state.and_then(|id| mapping.states.get(&id).cloned()),
        velocity,
    })
}

fn required_u32(bytes: &[u8], path: &[u32]) -> DecodeResult<u32> {
    optional_u32(bytes, path)?.ok_or_else(|| "Required integer missing".into())
}

/// Absent is allowed; a transmitted zero or oversized value is still malformed.
fn optional_u32(bytes: &[u8], path: &[u32]) -> DecodeResult<Option<u32>> {
    let Some(value) = scalar(bytes, path)? else {
        return Ok(None);
    };
    let value = u32::try_from(value).map_err(|_| "Integer exceeds u32")?;
    if value == 0 {
        return Err("Zero scene/entity/UID".into());
    }
    Ok(Some(value))
}

fn owned_entity(bytes: &[u8], entity: &[u32], owner: Option<&[u32]>) -> DecodeResult<OwnedEntity> {
    Ok(OwnedEntity {
        owner_uid: match owner {
            Some(path) => scalar(bytes, path)?,
            None => None,
        },
        entity: required_u32(bytes, entity),
    })
}
