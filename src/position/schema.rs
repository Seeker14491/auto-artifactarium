/// Build compatibility is explicit; older JSON profiles default to exact matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildMatching {
    #[default]
    Exact,
    /// Only the numeric D revision may vary. Release/platform, R and S must match.
    DataRevision,
    /// Numeric R, S and D revisions may vary. Release/platform must match.
    ReleaseVersion,
}

impl BuildMatching {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::DataRevision => "data_revision",
            Self::ReleaseVersion => "release_version",
        }
    }

    pub(crate) fn match_build(self, expected: &str, actual: &str) -> Option<Self> {
        if actual == expected {
            return Some(Self::Exact);
        }
        let (reference_release, reference_family) = build_parts(expected)?;
        let (observed_release, observed_family) = build_parts(actual)?;
        match self {
            Self::DataRevision if reference_family == observed_family => Some(self),
            Self::ReleaseVersion if reference_release == observed_release => Some(self),
            _ => None,
        }
    }
}

/// Return the release/platform prefix and the prefix including R/S revisions.
fn build_parts(build: &str) -> Option<(&str, &str)> {
    fn numeric(value: &str) -> bool {
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
    }
    if build.len() > 128 {
        return None;
    }
    let (family, data) = build.rsplit_once("_D")?;
    let (release, script) = family.rsplit_once("_S")?;
    let (platform_version, resource) = release.rsplit_once("_R")?;
    (numeric(data)
        && numeric(script)
        && numeric(resource)
        && !platform_version.is_empty()
        && platform_version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.'))
    .then_some((platform_version, family))
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub game_build: String,
    pub rules: Vec<Rule>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub method: String,
    pub source: Option<String>,
    pub sha256: Option<String>,
}

/// Operations are applied in order, wrapping after each operation in u32.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(
    tag = "op",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IntegerOp {
    Xor(u32),
    Add(u32),
    Sub(u32),
}

impl IntegerOp {
    pub fn apply(&self, value: u32) -> u32 {
        match self {
            Self::Xor(key) => value ^ key,
            Self::Add(offset) => value.wrapping_add(*offset),
            Self::Sub(offset) => value.wrapping_sub(*offset),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionFields {
    pub state: Option<Vec<u32>>,
    pub velocity: Option<Vec<u32>>,
    #[serde(default)]
    pub states: std::collections::BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub path: Vec<u32>,
    pub equals: u64,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub command_id: u16,
    /// Informational name only; matching always uses command_id.
    pub message: Option<String>,
    /// Path to repeated entries (empty means the entire payload).
    pub entries: Vec<u32>,
    pub filters: Vec<Filter>,
    pub event: Event,
}

#[derive(Debug, Clone)]
pub enum Event {
    LocalUid {
        uid: Vec<u32>,
    },
    Scene {
        scene: Vec<u32>,
        decode: Vec<IntegerOp>,
    },
    ActiveEntity {
        entity: Vec<u32>,
        /// Required for entries containing more than the local player's data.
        owner_uid: Option<Vec<u32>>,
    },
    Movement {
        entity: Vec<u32>,
        position: Vec<u32>,
        tick: Vec<u32>,
        /// Verified units per second of the movement clock.
        tick_rate_hz: u32,
        axes: [u32; 3],
        stationary: Option<Filter>,
        motion: MotionFields,
    },
    /// Select local active team entry, taking its unencoded scene ID.
    LocalTeam {
        scene: Vec<u32>,
        entity: Vec<u32>,
        owner_uid: Vec<u32>,
        active: Vec<u32>,
    },
    ReplaceEntity {
        previous: Vec<u32>,
        entity: Vec<u32>,
        owner_uid: Vec<u32>,
    },
    Union {
        command: Vec<u32>,
        body: Vec<u32>,
    },
    /// Multiple guards for one command accept any one matching rule.
    Build {
        path: Vec<u32>,
        expected: String,
        matching: BuildMatching,
    },
    InvalidateReading,
    Invalidate,
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        let p = self;
        if p.name.trim().is_empty()
            || p.game_build.trim().is_empty()
            || p.rules.is_empty()
            || p.rules.len() > 64
        {
            return Err("Profile needs a name, game_build and 1–64 rules".into());
        }
        let mut kinds = [false; 3];
        if let Some(source) = &p.provenance
            && (source.method.is_empty()
                || source.method.len() > 256
                || source.source.as_ref().is_some_and(|s| s.len() > 2048)
                || source
                    .sha256
                    .as_ref()
                    .is_some_and(|s| s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit())))
        {
            return Err("Invalid profile provenance".into());
        }
        for rule in &p.rules {
            if rule.command_id == 0 {
                return Err("Command IDs must be nonzero".into());
            }
            if rule.message.as_ref().is_some_and(|name| {
                name.is_empty()
                    || name.len() > 128
                    || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            }) {
                return Err("Invalid message name".into());
            }
            valid_path(&rule.entries, true)?;
            if rule.filters.len() > 16 {
                return Err("Too many filters".into());
            }
            for filter in &rule.filters {
                valid_path(&filter.path, false)?;
            }
            match &rule.event {
                Event::LocalUid { uid } => valid_path(uid, false)?,
                Event::Scene { scene, decode } => {
                    valid_path(scene, false)?;
                    if decode.len() > 8 {
                        return Err(
                            "Scene decoding permits at most eight integer operations".into()
                        );
                    }
                    kinds[0] = true;
                }
                Event::ActiveEntity { entity, owner_uid } => {
                    valid_path(entity, false)?;
                    if let Some(owner) = owner_uid {
                        valid_path(owner, false)?;
                    }
                    kinds[1] = true;
                }
                Event::Movement {
                    entity,
                    position,
                    tick,
                    tick_rate_hz,
                    axes,
                    stationary,
                    motion,
                } => {
                    for path in [&motion.state, &motion.velocity].into_iter().flatten() {
                        valid_path(path, false)?;
                    }
                    if motion.states.len() > 256
                        || (!motion.states.is_empty() && motion.state.is_none())
                        || motion.states.values().any(|name| {
                            name.is_empty()
                                || name.len() > 128
                                || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                        })
                    {
                        return Err("Invalid motion state names".into());
                    }
                    if let Some(filter) = stationary {
                        valid_path(&filter.path, false)?;
                    }
                    valid_path(entity, false)?;
                    valid_path(position, false)?;
                    valid_path(tick, false)?;
                    if *tick_rate_hz == 0 || *tick_rate_hz > 1_000_000_000 {
                        return Err("tick_rate_hz must be between 1 and 1000000000".into());
                    }
                    valid_path(axes, false)?;
                    if axes[0] == axes[1] || axes[1] == axes[2] || axes[0] == axes[2] {
                        return Err("Axis fields must be distinct".into());
                    }
                    kinds[2] = true;
                }
                Event::LocalTeam {
                    scene,
                    entity,
                    owner_uid,
                    active,
                } => {
                    for path in [scene, entity, owner_uid, active] {
                        valid_path(path, false)?;
                    }
                    kinds[0] = true;
                    kinds[1] = true;
                }
                Event::ReplaceEntity {
                    previous,
                    entity,
                    owner_uid,
                } => {
                    for path in [previous, entity, owner_uid] {
                        valid_path(path, false)?;
                    }
                }
                Event::Union { command, body } => {
                    if !rule.filters.is_empty() {
                        return Err("Union filters are unsupported".into());
                    }
                    valid_path(command, false)?;
                    valid_path(body, false)?;
                }
                Event::Build {
                    path,
                    expected,
                    matching,
                } => {
                    if !rule.entries.is_empty() || !rule.filters.is_empty() {
                        return Err("Build guard must read directly from the body".into());
                    }
                    valid_path(path, false)?;
                    if expected.is_empty()
                        || expected.len() > 128
                        || !expected.bytes().all(|b| b.is_ascii_graphic())
                    {
                        return Err("Invalid expected build".into());
                    }
                    if *matching != BuildMatching::Exact && build_parts(expected).is_none() {
                        return Err(format!(
                            "{} matching requires <release/platform>_R<digits>_S<digits>_D<digits>",
                            matching.as_str()
                        ));
                    }
                }
                Event::Invalidate | Event::InvalidateReading => (),
            }
        }
        if !kinds.into_iter().all(|v| v) {
            return Err("Profile must define scene, active_entity and movement rules".into());
        }
        Ok(())
    }
}
fn valid_path(path: &[u32], allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && path.is_empty())
        || path.len() > 12
        || path.iter().any(|&f| f == 0 || f > 0x1fff_ffff)
    {
        return Err("Paths need 1–12 valid protobuf field numbers (entries may be empty)".into());
    }
    Ok(())
}
