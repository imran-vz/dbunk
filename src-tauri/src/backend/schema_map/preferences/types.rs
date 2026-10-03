use crate::backend::schema_map::SchemaMapIdentity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_MAP_POSITIONS: usize = 512;
pub const MAX_MAP_POSITION_COORDINATE: f64 = 1_000_000.0;
pub const MAX_MAP_PREFERENCES_BYTES: usize = 128 * 1024;
pub const MAX_MAP_PREFERENCES_HEAP_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapRouting {
    #[default]
    Curve,
    Step,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapAttributes {
    #[default]
    All,
    KeysOnly,
    None,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapPrefs {
    pub routing: MapRouting,
    pub attributes: MapAttributes,
    pub show_types: bool,
    pub show_nulls: bool,
    pub show_comments: bool,
}
impl Default for MapPrefs {
    fn default() -> Self {
        Self {
            routing: MapRouting::Curve,
            attributes: MapAttributes::All,
            show_types: true,
            show_nulls: false,
            show_comments: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedPosition {
    #[serde(deserialize_with = "identity")]
    pub identity: SchemaMapIdentity,
    pub position: MapPoint,
}
fn identity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<SchemaMapIdentity, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Identity {
        database_oid: u32,
        relation_oid: u32,
    }
    let value = Identity::deserialize(deserializer)?;
    Ok(SchemaMapIdentity {
        database_oid: value.database_oid,
        relation_oid: value.relation_oid,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaMapPreferences {
    pub database_oid: u32,
    pub prefs: MapPrefs,
    #[serde(deserialize_with = "positions")]
    pub positions: Vec<SavedPosition>,
}
impl SchemaMapPreferences {
    pub fn for_database(database_oid: u32) -> Self {
        Self {
            database_oid,
            prefs: MapPrefs::default(),
            positions: Vec::new(),
        }
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.database_oid == 0 || self.positions.len() > MAX_MAP_POSITIONS {
            return None;
        }
        let mut identities = BTreeSet::new();
        for position in &self.positions {
            if position.identity.database_oid != self.database_oid
                || position.identity.relation_oid == 0
                || !identities.insert(position.identity)
                || [position.position.x, position.position.y]
                    .iter()
                    .any(|n| !n.is_finite() || n.abs() > MAX_MAP_POSITION_COORDINATE)
            {
                return None;
            }
        }
        std::mem::size_of::<Self>()
            .checked_add(
                self.positions
                    .capacity()
                    .checked_mul(std::mem::size_of::<SavedPosition>())?,
            )
            .filter(|n| *n <= MAX_MAP_PREFERENCES_HEAP_BYTES)
    }
}
fn positions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<SavedPosition>, D::Error> {
    struct Positions;
    impl<'de> serde::de::Visitor<'de> for Positions {
        type Value = Vec<SavedPosition>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 512 map positions")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(position) = seq.next_element()? {
                if result.len() == MAX_MAP_POSITIONS {
                    return Err(serde::de::Error::custom("map position limit"));
                }
                result.push(position);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Positions)
}

/// Exact typed persistence identity. Names are encoded as JSON fields and never
/// joined with punctuation or parsed as qualified SQL identifiers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SchemaMapPreferenceScope {
    Database,
    Schema { name: String },
    Relation { schema: String, table: String },
}
impl SchemaMapPreferenceScope {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let texts: &[&String] = match self {
            Self::Database => &[],
            Self::Schema { name } => &[name],
            Self::Relation { schema, table } => &[schema, table],
        };
        let mut heap = std::mem::size_of::<Self>();
        for text in texts {
            if text.is_empty() || text.len() > 63 || text.contains('\0') {
                return None;
            }
            heap = heap.checked_add(text.capacity())?;
        }
        (heap <= 4096).then_some(heap)
    }
}

/// Opaque exact observed generation, including absence. Reset writes a new
/// generation tombstone so an old absent-record revision never becomes valid.
#[derive(Clone, PartialEq, Eq)]
pub struct SchemaMapPreferencesRevision {
    pub(super) key: String,
    pub(super) generation: Option<uuid::Uuid>,
}
impl std::fmt::Debug for SchemaMapPreferencesRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaMapPreferencesRevision")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaMapPreferencesCapture {
    pub connection_id: String,
    pub scope: SchemaMapPreferenceScope,
    pub revision: SchemaMapPreferencesRevision,
    pub value: Option<SchemaMapPreferences>,
}
impl SchemaMapPreferencesCapture {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.connection_id.is_empty()
            || self.connection_id.len() > 256
            || self.connection_id.contains('\0')
            || self.revision.key.len() > 4096
        {
            return None;
        }
        let size = std::mem::size_of::<Self>()
            .checked_add(self.connection_id.capacity())?
            .checked_add(
                self.scope
                    .checked_heap_bytes()?
                    .checked_sub(std::mem::size_of::<SchemaMapPreferenceScope>())?,
            )?
            .checked_add(self.revision.key.capacity())?
            .checked_add(self.value.as_ref().map_or(Some(0), |v| {
                v.checked_heap_bytes()
                    .and_then(|n| n.checked_sub(std::mem::size_of::<SchemaMapPreferences>()))
            })?)?;
        (size <= MAX_MAP_PREFERENCES_HEAP_BYTES).then_some(size)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaMapPreferencesError {
    Cancelled,
    Invalid,
    TooLarge,
    UnsupportedVersion,
    Corrupt,
    StaleRevision,
    Storage,
    Document,
    Unavailable,
}

impl std::fmt::Display for SchemaMapPreferencesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => {
                "Map preference operation cancelled before dispatch; stored settings unchanged"
            }
            Self::Invalid => "Invalid native map preferences; nothing was saved",
            Self::TooLarge => {
                "Native map preferences exceed the storage limit; stored settings preserved"
            }
            Self::UnsupportedVersion => {
                "Native map preferences use an unsupported version; stored settings preserved"
            }
            Self::Corrupt => "Stored native map preferences are corrupt; stored settings preserved",
            Self::StaleRevision => {
                "Native map preferences changed; reload before saving or resetting"
            }
            Self::Storage => "Native map preference storage is unavailable",
            Self::Document => "The map document is closed, retired or belongs to another profile",
            Self::Unavailable => "The native profile is unavailable for map preferences",
        })
    }
}
impl std::error::Error for SchemaMapPreferencesError {}
