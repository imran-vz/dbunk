//! `VERSION()` parsing, the editability gate and the feature flags planners
//! consult. MySQL 8.0+ and MariaDB 10.6+ are editable; older servers are
//! captured read-only.
use crate::schema_edit::{ServerFacts, ServerFeatures, ServerFlavor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Version {
    pub flavor: ServerFlavor,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `8.0.36`, `8.4.0-commercial`, `10.6.12-MariaDB-1:10.6.12+maria~ubu2004`
    /// and the replication-hack prefix `5.5.5-10.6.12-MariaDB`.
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        let mariadb = lower.contains("mariadb");
        let mut rest = lower.as_str();
        if mariadb {
            if let Some(stripped) = rest.strip_prefix("5.5.5-") {
                rest = stripped;
            }
        }
        let numeric = rest
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .next()
            .unwrap_or_default();
        let mut parts = numeric.split('.');
        let major = parts.next()?.parse::<u32>().ok()?;
        let minor = parts.next()?.parse::<u32>().ok()?;
        let patch = parts
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(0);
        Some(Self {
            flavor: if mariadb {
                ServerFlavor::MariaDb
            } else {
                ServerFlavor::MySql
            },
            major,
            minor,
            patch,
        })
    }

    pub fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }

    pub fn is_mariadb(&self) -> bool {
        self.flavor == ServerFlavor::MariaDb
    }

    pub fn features(&self) -> ServerFeatures {
        match self.flavor {
            ServerFlavor::MariaDb => ServerFeatures {
                rename_column: self.at_least(10, 5, 2),
                drop_column: true,
                instant_add_column: false,
                instant_add_column_anywhere: false,
                enforced_check: self.at_least(10, 2, 1),
                column_reorder: true,
                transactional_ddl: false,
            },
            _ => ServerFeatures {
                rename_column: self.at_least(8, 0, 0),
                drop_column: true,
                instant_add_column: self.at_least(8, 0, 12),
                instant_add_column_anywhere: self.at_least(8, 0, 29),
                enforced_check: self.at_least(8, 0, 16),
                column_reorder: true,
                transactional_ddl: false,
            },
        }
    }

    /// The reason this server is read-only, if it is.
    pub fn gate(&self) -> Option<String> {
        match self.flavor {
            ServerFlavor::MariaDb if !self.at_least(10, 6, 0) => Some(format!(
                "MariaDB {}.{}.{} is older than 10.6; editing a table's structure needs MariaDB 10.6 or newer",
                self.major, self.minor, self.patch
            )),
            ServerFlavor::MySql if !self.at_least(8, 0, 0) => Some(format!(
                "MySQL {}.{}.{} is older than 8.0; editing a table's structure needs MySQL 8.0 or newer",
                self.major, self.minor, self.patch
            )),
            _ => None,
        }
    }

    /// `information_schema.CHECK_CONSTRAINTS` exists.
    pub fn has_check_catalog(&self) -> bool {
        match self.flavor {
            ServerFlavor::MariaDb => self.at_least(10, 2, 22),
            _ => self.at_least(8, 0, 16),
        }
    }

    /// MariaDB `CHECK_CONSTRAINTS.LEVEL` (column or table check).
    pub fn has_check_level(&self) -> bool {
        self.is_mariadb() && self.at_least(10, 5, 10)
    }

    /// MySQL `STATISTICS.EXPRESSION` (functional key parts).
    pub fn has_index_expression(&self) -> bool {
        !self.is_mariadb() && self.at_least(8, 0, 13)
    }

    /// MySQL `STATISTICS.IS_VISIBLE`.
    pub fn has_index_visibility(&self) -> bool {
        !self.is_mariadb() && self.at_least(8, 0, 0)
    }

    /// MariaDB 10.6 `STATISTICS.IGNORED`.
    pub fn has_ignored_index(&self) -> bool {
        self.is_mariadb() && self.at_least(10, 6, 0)
    }

    /// MySQL `TABLE_CONSTRAINTS.ENFORCED`.
    pub fn has_enforced_column(&self) -> bool {
        !self.is_mariadb() && self.at_least(8, 0, 16)
    }

    /// `COLUMNS.GENERATION_EXPRESSION`.
    pub fn has_generation_expression(&self) -> bool {
        match self.flavor {
            ServerFlavor::MariaDb => self.at_least(10, 2, 0),
            _ => self.at_least(5, 7, 0),
        }
    }

    /// MySQL `information_schema.VIEW_TABLE_USAGE`.
    pub fn has_view_table_usage(&self) -> bool {
        !self.is_mariadb() && self.at_least(8, 0, 13)
    }
}

/// Server facts for a baseline, plus the reason the server is read-only.
pub(super) fn facts(text: &str) -> (ServerFacts, Option<Version>, Option<String>) {
    match Version::parse(text) {
        Some(version) => (
            ServerFacts {
                flavor: version.flavor,
                version: text.to_owned(),
                features: version.features(),
            },
            Some(version),
            version.gate(),
        ),
        None => (
            ServerFacts {
                flavor: if text.to_ascii_lowercase().contains("mariadb") {
                    ServerFlavor::MariaDb
                } else {
                    ServerFlavor::MySql
                },
                version: text.to_owned(),
                features: ServerFeatures::default(),
            },
            None,
            Some(format!(
                "The server version {text:?} is not recognised; editing needs MySQL 8.0+ or MariaDB 10.6+"
            )),
        ),
    }
}
