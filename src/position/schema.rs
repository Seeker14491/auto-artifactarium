/// Build compatibility is explicit; older JSON profiles default to exact matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildMatching {
    #[default]
    Exact,
    /// Only the numeric D revision may vary. Release/platform, R and S must match.
    DataRevision,
}

impl BuildMatching {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::DataRevision => "data_revision",
        }
    }

    pub(crate) fn match_build(self, expected: &str, actual: &str) -> Option<Self> {
        if actual == expected {
            return Some(Self::Exact);
        }
        if self == Self::DataRevision
            && let (Some(reference), Some(observed)) =
                (build_family(expected), build_family(actual))
            && reference == observed
        {
            return Some(Self::DataRevision);
        }
        None
    }
}

fn build_family(build: &str) -> Option<&str> {
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
    .then_some(family)
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub game_build: String,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub path: Vec<u32>,
    pub equals: u64,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub command_id: u16,
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
        for rule in &p.rules {
            if rule.command_id == 0 {
                return Err("Command IDs must be nonzero".into());
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
                Event::Scene { scene } => {
                    valid_path(scene, false)?;
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
                } => {
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
                    if *matching == BuildMatching::DataRevision && build_family(expected).is_none()
                    {
                        return Err("data_revision matching requires <release/platform>_R<digits>_S<digits>_D<digits>".into());
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
