/**
 * SQLite engine-specific fields. The SQLite connection form has no
 * extras beyond the shared name/engine/database rows handled by the
 * parent — but the component exists for symmetry with the other
 * engines so the parent's switch is uniform.
 */

export function SqliteFields() {
  return null;
}
