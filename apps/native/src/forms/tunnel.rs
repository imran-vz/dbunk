//! C08.a SSH route fields in the connection form. Only general PostgreSQL
//! profiles offer routes. Options the form does not edit are carried from the
//! loaded record unchanged.
use super::*;

pub(super) const FIELDS: [(&str, &str); 4] = [
    ("tunnel-jump", "Jump hops (Bastion names, in order)"),
    (
        "tunnel-local-port",
        "Local forward port (blank = automatic)",
    ),
    ("tunnel-keepalive", "SSH keepalive (seconds)"),
    ("tunnel-proxy", "SSH proxy command"),
];

pub(super) struct State {
    pub(super) enabled: bool,
    pub(super) bastion: Option<String>,
    base: Option<DevelopmentSshTunnel>,
    pub(super) choices: Vec<(String, String)>,
    pub(super) loaded: bool,
    initial_jump: String,
    _load: Option<Task<()>>,
}

impl State {
    pub(super) fn new(stored: Option<DevelopmentSshTunnel>) -> Self {
        Self {
            enabled: stored.is_some(),
            bastion: stored.as_ref().map(|tunnel| tunnel.bastion_id.clone()),
            initial_jump: stored
                .as_ref()
                .map(|tunnel| tunnel.jump_chain.join(", "))
                .unwrap_or_default(),
            base: stored,
            choices: Vec::new(),
            loaded: false,
            _load: None,
        }
    }

    pub(super) fn initial(&self, key: &str) -> String {
        let base = self.base.as_ref();
        match key {
            "tunnel-jump" => self.initial_jump.clone(),
            "tunnel-local-port" => base
                .and_then(|tunnel| tunnel.local_port)
                .map(|port| port.to_string())
                .unwrap_or_default(),
            "tunnel-keepalive" => number(base.and_then(|tunnel| tunnel.keepalive_interval_seconds)),
            "tunnel-proxy" => base
                .and_then(|tunnel| tunnel.proxy_command.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub(super) fn choice_name(&self, id: &str) -> Option<&str> {
        self.choices
            .iter()
            .find(|(choice, _)| choice == id)
            .map(|(_, name)| name.as_str())
    }
}

/// Each hop is an exact Bastion ID or a unique Bastion name.
pub(super) fn resolve_hops(
    text: &str,
    choices: &[(String, String)],
) -> Result<Vec<String>, String> {
    text.split(',')
        .map(str::trim)
        .filter(|hop| !hop.is_empty())
        .map(|hop| {
            if choices.iter().any(|(id, _)| id == hop) {
                return Ok(hop.to_owned());
            }
            let mut named = choices.iter().filter(|(_, name)| name == hop);
            match (named.next(), named.next()) {
                (Some((id, _)), None) => Ok(id.clone()),
                (Some(_), Some(_)) => Err(format!(
                    "More than one Bastion Server is named {hop}; rename one first"
                )),
                (None, _) => Err(format!("No Bastion Server named {hop}")),
            }
        })
        .collect()
}

impl Form {
    /// Starts the bounded choice load; the stored record stays authoritative
    /// until the user changes the route.
    pub(super) fn load_tunnel_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.host.backend.clone();
        let work = self
            .host
            .runtime
            .spawn(async move { backend.development_bastions().await });
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|_| Err("Bastion Servers could not load".into()));
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(tunnel) = this.tunnel.as_mut() else {
                    return;
                };
                tunnel.loaded = true;
                match result {
                    Ok(list) => {
                        tunnel.choices = list
                            .into_iter()
                            .map(|bastion| (bastion.id, bastion.form.name))
                            .collect();
                        // Replace stored IDs with names unless the user typed.
                        let names = tunnel
                            .base
                            .as_ref()
                            .map(|base| {
                                base.jump_chain
                                    .iter()
                                    .map(|id| tunnel.choice_name(id).unwrap_or(id).to_owned())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let initial = tunnel.initial_jump.clone();
                        if let Some(field) =
                            this.fields.iter().find(|field| field.key == "tunnel-jump")
                            && field.editor.read(cx).text(cx) == initial
                            && names != initial
                        {
                            field.editor.update(cx, |editor, cx| {
                                editor.set_text(names.clone(), window, cx)
                            });
                            if let Some(tunnel) = this.tunnel.as_mut() {
                                tunnel.initial_jump = names;
                            }
                        }
                    }
                    Err(error) => this.fail(error),
                }
                cx.notify();
            });
        });
        if let Some(tunnel) = self.tunnel.as_mut() {
            tunnel._load = Some(task);
        }
    }

    pub(super) fn tunnel_input(&self, cx: &App) -> Result<Option<DevelopmentSshTunnel>, String> {
        let Some(state) = self.tunnel.as_ref().filter(|state| state.enabled) else {
            return Ok(None);
        };
        let bastion = state
            .bastion
            .clone()
            .ok_or("Choose a Bastion Server for the SSH tunnel")?;
        let mut tunnel = state
            .base
            .clone()
            .unwrap_or_else(|| DevelopmentSshTunnel::new(bastion.clone()));
        tunnel.bastion_id = bastion;
        tunnel.jump_chain = resolve_hops(&self.value("tunnel-jump", cx), &state.choices)?;
        let optional = |key| {
            let value = self.value(key, cx);
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        };
        tunnel.local_port = optional("tunnel-local-port")
            .map(|value| {
                value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or("Local forward port must be between 1 and 65535")
            })
            .transpose()?;
        tunnel.keepalive_interval_seconds = optional("tunnel-keepalive")
            .map(|value| {
                value
                    .parse::<u32>()
                    .map_err(|_| "SSH keepalive must be a number of seconds")
            })
            .transpose()?;
        tunnel.proxy_command = optional("tunnel-proxy");
        Ok(Some(tunnel))
    }

    pub(super) fn tunnel_view(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let state = self.tunnel.as_ref()?;
        let enabled = state.enabled;
        let selected = state.bastion.clone();
        let choices = state.choices.clone();
        let loaded = state.loaded;
        let mut section =
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().flex().child(self.button(
                    "SSH tunnel",
                    FormAction::Tunnel,
                    enabled,
                    cx,
                )));
        if !enabled {
            return Some(section);
        }
        let note = |text: &'static str| {
            div()
                .text_sm()
                .text_color(crate::style::faint())
                .child(text)
        };
        let mut chips = div().flex().flex_wrap().gap(px(4.));
        if !loaded {
            chips = chips.child(note("Loading Bastion Servers…"));
        } else if choices.is_empty() {
            chips = chips.child(note(
                "No Bastion Servers yet; add one under Bastion servers.",
            ));
        }
        for (index, (id, name)) in choices.iter().enumerate() {
            chips = chips.child(self.button(
                format!("Via {name}"),
                FormAction::TunnelVia(index),
                selected.as_deref() == Some(id.as_str()),
                cx,
            ));
        }
        let mut via = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(
                div()
                    .text_size(px(crate::style::FONT_SMALL))
                    .text_color(crate::style::dim())
                    .child("Final Bastion"),
            )
            .child(chips);
        if let Some(id) = &selected
            && loaded
            && !choices.iter().any(|(choice, _)| choice == id)
        {
            via = via.child(note(
                "The saved Bastion Server no longer exists; choose another.",
            ));
        }
        section = section.child(via);
        let errors = super::validation::Errors::new();
        let keys: Vec<&'static str> = self
            .fields
            .iter()
            .map(|field| field.key)
            .filter(|key| key.starts_with("tunnel-"))
            .collect();
        let mut fields = div().grid().grid_cols(2).gap_x(px(12.)).gap_y(px(10.));
        for key in keys {
            if let Some(input) = self.text_field(key, &errors, false) {
                fields = fields.child(input);
            }
        }
        Some(section.child(fields).child(
            div().text_sm().text_color(crate::style::faint()).child(
            "The database host and port are dialled from the final Bastion. Connections fail closed until each Bastion's host key is tested and trusted. TLS verifies the database host name through the tunnel.",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hops_resolve_by_id_or_unique_name_and_refuse_ambiguity() {
        let choices = vec![
            ("id-a".to_owned(), "Edge".to_owned()),
            ("id-b".to_owned(), "Core".to_owned()),
            ("id-c".to_owned(), "Twin".to_owned()),
            ("id-d".to_owned(), "Twin".to_owned()),
        ];
        assert_eq!(
            resolve_hops(" Edge , id-b ,", &choices).unwrap(),
            ["id-a", "id-b"]
        );
        assert!(resolve_hops("", &choices).unwrap().is_empty());
        assert!(
            resolve_hops("Twin", &choices)
                .unwrap_err()
                .contains("More than one")
        );
        assert!(
            resolve_hops("Missing", &choices)
                .unwrap_err()
                .contains("No Bastion")
        );
    }
}
