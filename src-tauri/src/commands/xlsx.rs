//! Tauri adapters for XLSX import and export.

use tauri::ipc::Response;

use crate::xlsx::{self, ExportXlsxPayload, ParseXlsxPayload, ParsedSheet};

/// Parse an XLSX file (sent as base64) into sheets with auto-detected
/// headers. Returns one `ParsedSheet` per worksheet.
#[tauri::command]
pub async fn parse_xlsx(payload: ParseXlsxPayload) -> Result<Vec<ParsedSheet>, String> {
    xlsx::parse_xlsx(&payload)
}

/// Build an XLSX file from columns + rows and return raw bytes through
/// Tauri's IPC response.
#[tauri::command]
pub async fn export_xlsx(payload: ExportXlsxPayload) -> Result<Response, String> {
    Ok(Response::new(xlsx::build_xlsx_buffer(&payload)?))
}
