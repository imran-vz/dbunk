//! Plan 031 step 4: engine picker and per-engine field sets for the native
//! connection form. Pure logic only; rendering stays in `forms.rs`.
use dbunk_lib::backend::{
    DevelopmentClickHouseConnection, DevelopmentEngineConnection, DevelopmentEnvironment,
    DevelopmentMySqlConnection, DevelopmentRedisConnection, DevelopmentSafeMode,
    DevelopmentSqliteConnection, DevelopmentSshTunnel,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Engine {
    Postgres,
    MySql,
    Sqlite,
    ClickHouse,
    Redis,
}

/// Engine-specific switches that are not text fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Toggle {
    MySqlTls,
    Https,
    RedisTls,
    RedisVerify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Toggles {
    pub(super) mysql_tls: bool,
    pub(super) https: bool,
    pub(super) redis_tls: bool,
    pub(super) redis_verify: bool,
}

impl Default for Toggles {
    fn default() -> Self {
        Self {
            mysql_tls: true,
            https: false,
            redis_tls: false,
            redis_verify: true,
        }
    }
}

impl Toggles {
    pub(super) fn from_settings(settings: &DevelopmentEngineConnection) -> Self {
        let mut toggles = Self::default();
        match settings {
            DevelopmentEngineConnection::MySQL(c) => toggles.mysql_tls = c.ssl,
            DevelopmentEngineConnection::ClickHouse(c) => toggles.https = c.use_https,
            DevelopmentEngineConnection::Redis(c) => {
                toggles.redis_tls = c.use_tls;
                toggles.redis_verify = c.verify_tls_cert;
            }
            DevelopmentEngineConnection::PostgreSQL(_) | DevelopmentEngineConnection::SQLite(_) => {
            }
        }
        toggles
    }

    pub(super) fn get(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::MySqlTls => self.mysql_tls,
            Toggle::Https => self.https,
            Toggle::RedisTls => self.redis_tls,
            Toggle::RedisVerify => self.redis_verify,
        }
    }

    pub(super) fn flip(&mut self, toggle: Toggle) {
        let value = match toggle {
            Toggle::MySqlTls => &mut self.mysql_tls,
            Toggle::Https => &mut self.https,
            Toggle::RedisTls => &mut self.redis_tls,
            Toggle::RedisVerify => &mut self.redis_verify,
        };
        *value = !*value;
    }
}

/// Text fields owned by one engine; shared keys (name, host, organization)
/// are not listed. Unknown keys are visible for every engine.
const POSTGRES_ONLY: [&str; 10] = [
    "root-cert",
    "client-cert",
    "client-key",
    "server-name",
    "statement-timeout",
    "idle-timeout",
    "connect-timeout",
    "keepalive",
    "search-path",
    "role",
];

impl Engine {
    pub(super) const ALL: [Engine; 5] = [
        Engine::Postgres,
        Engine::MySql,
        Engine::Sqlite,
        Engine::ClickHouse,
        Engine::Redis,
    ];

    /// Matches `DevelopmentConnection::engine`.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL",
            Self::Sqlite => "SQLite",
            Self::ClickHouse => "ClickHouse",
            Self::Redis => "Redis",
        }
    }

    pub(super) fn of(settings: &DevelopmentEngineConnection) -> Self {
        match settings {
            DevelopmentEngineConnection::PostgreSQL(_) => Self::Postgres,
            DevelopmentEngineConnection::MySQL(_) => Self::MySql,
            DevelopmentEngineConnection::SQLite(_) => Self::Sqlite,
            DevelopmentEngineConnection::ClickHouse(_) => Self::ClickHouse,
            DevelopmentEngineConnection::Redis(_) => Self::Redis,
        }
    }

    pub(super) fn tunnels(self) -> bool {
        self != Self::Sqlite
    }

    pub(super) fn shows(self, key: &str) -> bool {
        if key.starts_with("tunnel-") {
            return self.tunnels();
        }
        if POSTGRES_ONLY.contains(&key) {
            return self == Self::Postgres;
        }
        match key {
            "host" | "port" | "user" | "password" => self != Self::Sqlite,
            "database" => matches!(self, Self::Postgres | Self::MySql | Self::ClickHouse),
            "path" => self == Self::Sqlite,
            "url-path" => self == Self::ClickHouse,
            "db-number" => self == Self::Redis,
            _ => true,
        }
    }

    /// Defaults offered for a new connection: name, port, database, user.
    pub(super) fn defaults(self) -> [(&'static str, &'static str); 4] {
        let (name, port, database, user) = match self {
            Self::Postgres => ("Local PostgreSQL", "5432", "postgres", "postgres"),
            Self::MySql => ("Local MySQL", "3306", "", "root"),
            Self::Sqlite => ("Local SQLite", "", "", ""),
            Self::ClickHouse => ("Local ClickHouse", "8123", "default", "default"),
            Self::Redis => ("Local Redis", "6379", "", ""),
        };
        [
            ("name", name),
            ("port", port),
            ("database", database),
            ("user", user),
        ]
    }
}

/// Switching engines on a new connection replaces only values still equal
/// to the previous engine's default, so typed input is never discarded.
pub(super) fn switch_defaults(
    from: Engine,
    to: Engine,
    current: impl Fn(&str) -> String,
) -> Vec<(&'static str, String)> {
    from.defaults()
        .into_iter()
        .zip(to.defaults())
        .filter(|((key, old), _)| current(key).trim() == *old)
        .map(|(_, (key, new))| (key, new.to_owned()))
        .collect()
}

/// Initial text for engine-specific fields when editing a saved record.
pub(super) fn initial_values(
    settings: &DevelopmentEngineConnection,
) -> Vec<(&'static str, String)> {
    match settings {
        DevelopmentEngineConnection::PostgreSQL(_) => Vec::new(),
        DevelopmentEngineConnection::MySQL(c) => vec![
            ("name", c.name.clone()),
            ("host", c.host.clone()),
            ("port", c.port.to_string()),
            ("database", c.database.clone()),
            ("user", c.user.clone()),
        ],
        DevelopmentEngineConnection::SQLite(c) => {
            vec![("name", c.name.clone()), ("path", c.path.clone())]
        }
        DevelopmentEngineConnection::ClickHouse(c) => vec![
            ("name", c.name.clone()),
            ("host", c.host.clone()),
            ("port", c.port.to_string()),
            ("database", c.database.clone()),
            ("user", c.user.clone()),
            ("url-path", c.url_path.clone()),
        ],
        DevelopmentEngineConnection::Redis(c) => vec![
            ("name", c.name.clone()),
            ("host", c.host.clone()),
            ("port", c.port.to_string()),
            ("db-number", c.db_number.to_string()),
            ("user", c.user.clone()),
        ],
    }
}

/// Saved route of a non-PostgreSQL record, for the shared tunnel editor.
pub(super) fn stored_tunnel(
    settings: &DevelopmentEngineConnection,
) -> Option<DevelopmentSshTunnel> {
    match settings {
        DevelopmentEngineConnection::MySQL(c) => c.ssh_tunnel.clone(),
        DevelopmentEngineConnection::ClickHouse(c) => c.ssh_tunnel.clone(),
        DevelopmentEngineConnection::Redis(c) => c.ssh_tunnel.clone(),
        DevelopmentEngineConnection::PostgreSQL(c) => c.ssh_tunnel.clone(),
        DevelopmentEngineConnection::SQLite(_) => None,
    }
}

pub(super) struct Policy {
    pub(super) environment: DevelopmentEnvironment,
    pub(super) safe_mode: DevelopmentSafeMode,
    pub(super) read_only: bool,
}

impl Policy {
    pub(super) fn of(settings: &DevelopmentEngineConnection) -> Self {
        let (environment, safe_mode, read_only) = match settings {
            DevelopmentEngineConnection::PostgreSQL(c) => (c.environment, c.safe_mode, c.read_only),
            DevelopmentEngineConnection::MySQL(c) => (c.environment, c.safe_mode, c.read_only),
            DevelopmentEngineConnection::SQLite(c) => (c.environment, c.safe_mode, c.read_only),
            DevelopmentEngineConnection::ClickHouse(c) => (c.environment, c.safe_mode, c.read_only),
            DevelopmentEngineConnection::Redis(c) => (c.environment, c.safe_mode, c.read_only),
        };
        Self {
            environment,
            safe_mode,
            read_only,
        }
    }
}

/// Builds a non-PostgreSQL record from form text. Field limits are enforced
/// again by the backend; this only parses numbers and trims paths.
pub(super) fn input(
    engine: Engine,
    value: impl Fn(&str) -> String,
    policy: Policy,
    toggles: Toggles,
    ssh_tunnel: Option<DevelopmentSshTunnel>,
) -> Result<DevelopmentEngineConnection, String> {
    let port = || {
        value("port")
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| "Port must be between 1 and 65535".to_string())
    };
    let Policy {
        environment,
        safe_mode,
        read_only,
    } = policy;
    Ok(match engine {
        Engine::Postgres => return Err("PostgreSQL uses its dedicated form input".into()),
        Engine::MySql => DevelopmentEngineConnection::MySQL(DevelopmentMySqlConnection {
            name: value("name"),
            host: value("host").trim().into(),
            port: port()?,
            database: value("database").trim().into(),
            user: value("user"),
            environment,
            safe_mode,
            read_only,
            ssl: toggles.mysql_tls,
            ssh_tunnel,
        }),
        Engine::Sqlite => DevelopmentEngineConnection::SQLite(DevelopmentSqliteConnection {
            name: value("name"),
            path: value("path").trim().into(),
            environment,
            safe_mode,
            read_only,
        }),
        Engine::ClickHouse => {
            DevelopmentEngineConnection::ClickHouse(DevelopmentClickHouseConnection {
                name: value("name"),
                host: value("host").trim().into(),
                port: port()?,
                database: value("database").trim().into(),
                user: value("user"),
                environment,
                safe_mode,
                read_only,
                use_https: toggles.https,
                url_path: value("url-path").trim().into(),
                ssh_tunnel,
            })
        }
        Engine::Redis => DevelopmentEngineConnection::Redis(DevelopmentRedisConnection {
            name: value("name"),
            host: value("host").trim().into(),
            port: port()?,
            db_number: value("db-number")
                .trim()
                .parse()
                .map_err(|_| "Redis database number must be between 0 and 255")?,
            user: value("user"),
            environment,
            safe_mode,
            read_only,
            use_tls: toggles.redis_tls,
            verify_tls_cert: toggles.redis_verify,
            ssh_tunnel,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn policy() -> Policy {
        Policy {
            environment: DevelopmentEnvironment::Production,
            safe_mode: DevelopmentSafeMode::Strict,
            read_only: true,
        }
    }

    fn values(pairs: &[(&str, &str)]) -> impl Fn(&str) -> String {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned().unwrap_or_default()
    }

    #[test]
    fn field_sets_follow_the_selected_engine() {
        assert!(Engine::Postgres.shows("root-cert"));
        assert!(!Engine::MySql.shows("root-cert"));
        assert!(Engine::Sqlite.shows("path"));
        for key in [
            "host",
            "port",
            "user",
            "password",
            "database",
            "tunnel-jump",
        ] {
            assert!(!Engine::Sqlite.shows(key), "{key}");
        }
        assert!(Engine::Redis.shows("db-number"));
        assert!(!Engine::Redis.shows("database"));
        assert!(Engine::ClickHouse.shows("url-path"));
        assert!(!Engine::MySql.shows("url-path"));
        for engine in Engine::ALL {
            for key in ["name", "project", "folder", "color"] {
                assert!(engine.shows(key), "{engine:?} {key}");
            }
        }
    }

    #[test]
    fn switching_engines_replaces_only_untouched_defaults() {
        let current = values(&[
            ("name", "Local PostgreSQL"),
            ("port", "5432"),
            ("database", "billing"),
            ("user", "postgres"),
        ]);
        let changes = switch_defaults(Engine::Postgres, Engine::MySql, current);
        assert_eq!(
            changes,
            [
                ("name", "Local MySQL".to_owned()),
                ("port", "3306".to_owned()),
                ("user", "root".to_owned()),
            ]
        );
    }

    #[test]
    fn inputs_parse_per_engine_and_carry_policy_toggles_and_route() {
        let toggles = Toggles {
            mysql_tls: false,
            https: true,
            redis_tls: true,
            redis_verify: false,
        };
        let redis = input(
            Engine::Redis,
            values(&[
                ("name", "Cache"),
                ("host", " cache.local "),
                ("port", "6380"),
                ("db-number", "4"),
            ]),
            policy(),
            toggles,
            Some(DevelopmentSshTunnel::new("edge")),
        )
        .unwrap();
        let DevelopmentEngineConnection::Redis(redis) = redis else {
            panic!("Redis input")
        };
        assert_eq!(
            (redis.host.as_str(), redis.port, redis.db_number),
            ("cache.local", 6380, 4)
        );
        assert!(redis.use_tls && !redis.verify_tls_cert && redis.read_only);
        assert_eq!(redis.ssh_tunnel.as_ref().unwrap().bastion_id, "edge");
        assert_eq!(
            Toggles::from_settings(&DevelopmentEngineConnection::Redis(redis.clone())),
            Toggles {
                redis_tls: true,
                redis_verify: false,
                ..Toggles::default()
            }
        );
        let clickhouse = input(
            Engine::ClickHouse,
            values(&[
                ("name", "Events"),
                ("host", "ch"),
                ("port", "8443"),
                ("url-path", " /ch "),
            ]),
            policy(),
            toggles,
            None,
        )
        .unwrap();
        let DevelopmentEngineConnection::ClickHouse(clickhouse) = clickhouse else {
            panic!("ClickHouse input")
        };
        assert!(clickhouse.use_https);
        assert_eq!(clickhouse.url_path, "/ch");
        let sqlite = input(
            Engine::Sqlite,
            values(&[("name", "File"), ("path", " /data/app.db ")]),
            policy(),
            toggles,
            Some(DevelopmentSshTunnel::new("ignored")),
        )
        .unwrap();
        let DevelopmentEngineConnection::SQLite(sqlite) = sqlite else {
            panic!("SQLite input")
        };
        assert_eq!(sqlite.path, "/data/app.db");
        for (engine, pairs) in [
            (Engine::MySql, vec![("port", "0")]),
            (Engine::ClickHouse, vec![("port", "x")]),
            (Engine::Redis, vec![("port", "6379"), ("db-number", "256")]),
        ] {
            assert!(input(engine, values(&pairs), policy(), toggles, None).is_err());
        }
        assert!(input(Engine::Postgres, values(&[]), policy(), toggles, None).is_err());
    }

    #[test]
    fn edit_values_round_trip_through_input() {
        let saved = DevelopmentEngineConnection::MySQL(DevelopmentMySqlConnection {
            name: "Orders".into(),
            host: "db".into(),
            port: 3307,
            database: "orders".into(),
            user: "app".into(),
            environment: DevelopmentEnvironment::Production,
            safe_mode: DevelopmentSafeMode::Strict,
            read_only: true,
            ssl: false,
            ssh_tunnel: None,
        });
        let initial = initial_values(&saved);
        let pairs = initial
            .iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect::<Vec<_>>();
        let rebuilt = input(
            Engine::of(&saved),
            values(&pairs),
            policy(),
            Toggles::from_settings(&saved),
            stored_tunnel(&saved),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(rebuilt).unwrap(),
            serde_json::to_value(saved).unwrap()
        );
    }
}
