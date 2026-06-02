use crate::{SshTunnelConfig, StoredConnection};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct SshSessionKey {
    bastion_ids: Vec<String>,
    compression: bool,
    keepalive_interval_seconds: Option<u32>,
    keepalive_want_reply: bool,
    proxy_command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SshRoute {
    pub(super) bastion_ids: Vec<String>,
    pub(super) session_key: SshSessionKey,
    pub(super) compression: bool,
    pub(super) keepalive_interval_seconds: Option<u32>,
    pub(super) keepalive_want_reply: bool,
    pub(super) proxy_command: Option<String>,
}

impl SshRoute {
    pub(super) fn from_config(config: &SshTunnelConfig) -> Result<Self, String> {
        let config = config.normalized();
        validate_tunnel_config(&config)?;
        let final_bastion_id = config
            .bastion_server_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "SSH tunnel is enabled but no Bastion Server is selected".to_string())?;
        let mut bastion_ids = config.jump_chain.clone();
        bastion_ids.push(final_bastion_id.to_string());
        let proxy_command = config.proxy_command.clone();
        let session_key = SshSessionKey {
            bastion_ids: bastion_ids.clone(),
            compression: config.compression,
            keepalive_interval_seconds: config.keepalive_interval_seconds,
            keepalive_want_reply: config.keepalive_want_reply,
            proxy_command: proxy_command.clone(),
        };
        Ok(Self {
            bastion_ids,
            session_key,
            compression: config.compression,
            keepalive_interval_seconds: config.keepalive_interval_seconds,
            keepalive_want_reply: config.keepalive_want_reply,
            proxy_command,
        })
    }
}

pub fn validate_connection_tunnel(connection: &StoredConnection) -> Result<(), String> {
    if let Some(config) = connection.ssh_tunnel() {
        let config = config.normalized();
        validate_tunnel_config(&config)?;
    }
    Ok(())
}

fn validate_tunnel_config(config: &SshTunnelConfig) -> Result<(), String> {
    if !config.enabled {
        return Ok(());
    }
    let final_bastion_id = config
        .bastion_server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "SSH tunnel is enabled but no Bastion Server is selected".to_string())?;
    if matches!(config.local_port, Some(0)) {
        return Err("SSH tunnel local port must be between 1 and 65535".to_string());
    }
    if let Some(interval) = config.keepalive_interval_seconds {
        if !(2..=3600).contains(&interval) {
            return Err("SSH keepalive interval must be between 2 and 3600 seconds".to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    for bastion_id in config
        .jump_chain
        .iter()
        .map(|bastion_id| bastion_id.trim())
        .filter(|bastion_id| !bastion_id.is_empty())
    {
        if bastion_id == final_bastion_id {
            return Err("SSH jump chain cannot include the selected Bastion Server".to_string());
        }
        if !seen.insert(bastion_id.to_string()) {
            return Err("SSH jump chain cannot include duplicate Bastion Servers".to_string());
        }
    }
    if config
        .proxy_command
        .as_deref()
        .is_some_and(|command| command.trim().is_empty())
    {
        return Err("SSH proxy command cannot be blank".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel() -> SshTunnelConfig {
        SshTunnelConfig {
            enabled: true,
            bastion_server_id: Some("final".to_string()),
            ..SshTunnelConfig::default()
        }
    }

    #[test]
    fn route_key_includes_advanced_options() {
        let mut first = tunnel();
        first.jump_chain = vec!["jump-1".to_string()];
        first.compression = true;
        first.keepalive_interval_seconds = Some(30);
        first.proxy_command = Some("ssh -W %h:%p edge".to_string());

        let mut second = first.clone();
        second.keepalive_interval_seconds = Some(60);

        let first_route = SshRoute::from_config(&first).expect("first route");
        let second_route = SshRoute::from_config(&second).expect("second route");

        assert_ne!(first_route.session_key, second_route.session_key);
        assert_eq!(
            first_route.bastion_ids,
            vec!["jump-1".to_string(), "final".to_string()]
        );
    }

    #[test]
    fn route_key_preserves_bastion_id_boundaries() {
        let mut first = tunnel();
        first.bastion_server_id = Some("b>c".to_string());
        first.jump_chain = vec!["a".to_string()];

        let mut second = tunnel();
        second.bastion_server_id = Some("c".to_string());
        second.jump_chain = vec!["a>b".to_string()];

        let first_route = SshRoute::from_config(&first).expect("first route");
        let second_route = SshRoute::from_config(&second).expect("second route");

        assert_ne!(first_route.session_key, second_route.session_key);
        assert_eq!(
            first_route.bastion_ids,
            vec!["a".to_string(), "b>c".to_string()]
        );
        assert_eq!(
            second_route.bastion_ids,
            vec!["a>b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn route_normalizes_tunnel_config_before_building_route() {
        let mut config = tunnel();
        config.bastion_server_id = Some(" final ".to_string());
        config.jump_chain = vec![" jump-1 ".to_string(), String::new()];
        config.proxy_command = Some(" ssh -W %h:%p edge ".to_string());

        let route = SshRoute::from_config(&config).expect("route");

        assert_eq!(
            route.bastion_ids,
            vec!["jump-1".to_string(), "final".to_string()]
        );
        assert_eq!(route.proxy_command.as_deref(), Some("ssh -W %h:%p edge"));
    }

    #[test]
    fn route_rejects_jump_chain_loops() {
        let mut config = tunnel();
        config.jump_chain = vec!["jump-1".to_string(), "final".to_string()];

        let error = SshRoute::from_config(&config).expect_err("loop rejected");

        assert!(error.contains("selected Bastion Server"));
    }

    #[test]
    fn route_rejects_duplicate_jump_hops() {
        let mut config = tunnel();
        config.jump_chain = vec!["jump-1".to_string(), "jump-1".to_string()];

        let error = SshRoute::from_config(&config).expect_err("duplicate rejected");

        assert!(error.contains("duplicate"));
    }
}
