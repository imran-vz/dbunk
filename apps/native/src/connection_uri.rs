//! Explicit clipboard operations. Raw imported URIs never enter an editor,
//! persisted draft, undo history or diagnostic message.
use dbunk_lib::backend::{
    DevelopmentPostgresConnection,
    connection_uri::{ParsedPostgresUri, build_postgres_uri, parse_postgres_uri},
};
use gpui::{App, ClipboardEntry, ClipboardItem};

fn text(item: &ClipboardItem) -> Option<&str> {
    match item.entries() {
        [ClipboardEntry::String(value)] => Some(value.text()),
        _ => None,
    }
}

pub fn read(cx: &App) -> Result<ParsedPostgresUri, String> {
    let item = cx
        .read_from_clipboard()
        .ok_or("Clipboard text is unavailable")?;
    let source = text(&item).ok_or("Copy one PostgreSQL URI as plain text first")?;
    // Parsing checks the byte bound before copying or decoding. The platform's
    // preceding clipboard allocation is outside that parser bound.
    parse_postgres_uri(source).map_err(|error| error.to_string())
}

pub fn copy(input: &DevelopmentPostgresConnection, cx: &mut App) -> Result<String, String> {
    let exported = build_postgres_uri(input).map_err(|error| error.to_string())?;
    cx.write_to_clipboard(ClipboardItem::new_string(exported.uri.clone()));
    // GPUI's platform write has no Result. Check immediate readback rather than
    // claiming that a refused clipboard write succeeded.
    if cx.read_from_clipboard().as_ref().and_then(text) != Some(exported.uri.as_str()) {
        return Err("Could not verify URI clipboard copy".into());
    }
    let omissions = exported.omissions;
    Ok(
        if omissions.tls_files || omissions.tls_server_name || omissions.driver_options {
            "URI copied. Password, policy, TLS files and connection options omitted"
        } else {
            "URI copied. Password and saved connection policy omitted"
        }
        .into(),
    )
}
