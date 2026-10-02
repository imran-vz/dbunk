use tauri::State;

pub(crate) use crate::safety::gate::{assert_legacy_permitted, record_override, resolved_policy};
use crate::{storage, AppState, SafetyOverrideRecord};

#[tauri::command]
pub(crate) async fn load_safety_overrides(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<SafetyOverrideRecord>, String> {
    storage::read_safety_overrides(&state.inner().pool, &connection_id).await
}
