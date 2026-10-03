use super::*;

pub(super) trait Storage: AsRef<str> + Serialize {
    fn capacity(&self) -> usize;
}
impl Storage for String {
    fn capacity(&self) -> usize {
        String::capacity(self)
    }
}
impl Storage for &str {
    fn capacity(&self) -> usize {
        self.len()
    }
}

pub(super) fn encoded(value: &impl Serialize, maximum: usize) -> Option<usize> {
    struct Count {
        bytes: usize,
        maximum: usize,
    }
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.maximum.saturating_sub(self.bytes) {
                return Err(io::Error::other("server details byte limit"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Count { bytes: 0, maximum };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.bytes)
}

fn sum(values: impl IntoIterator<Item = Option<usize>>) -> Option<usize> {
    values
        .into_iter()
        .try_fold(0usize, |total, value| total.checked_add(value?))
}
pub(super) fn identity<S: Storage>(value: &S) -> Option<usize> {
    (!value.as_ref().is_empty()
        && value.as_ref().len() <= MAX_SERVER_IDENTITY_BYTES
        && !value.as_ref().contains('\0'))
    .then(|| value.capacity())
}
pub(super) fn cell<S: Storage>(value: &ServerText<S>) -> Option<usize> {
    match value {
        ServerText::Value(value) => {
            (value.as_ref().len() <= MAX_SERVER_TEXT_BYTES).then(|| value.capacity())
        }
        ServerText::Null => Some(0),
        ServerText::Omitted { bytes } => (*bytes > MAX_SERVER_TEXT_BYTES as u64).then_some(0),
    }
}
pub(super) fn setting<S: Storage>(row: &ServerSetting<S>) -> Option<usize> {
    if row.inspection_override != matches!(row.name.as_ref(), "statement_timeout" | "lock_timeout")
    {
        return None;
    }
    sum([
        identity(&row.name),
        identity(&row.category),
        identity(&row.source),
        cell(&row.setting),
        cell(&row.unit),
        cell(&row.short_desc),
        cell(&row.boot_val),
        cell(&row.reset_val),
    ])
}
pub(super) fn extension<S: Storage>(row: &ServerExtension<S>) -> Option<usize> {
    sum([
        identity(&row.name),
        identity(&row.schema),
        cell(&row.version),
        cell(&row.description),
    ])
}
pub(super) fn reader<S: Storage>(value: &ReaderContext<S>) -> Option<usize> {
    if value.statement_timeout_ms == 0
        || value.statement_timeout_ms > 10_000
        || value.lock_timeout_ms != 2000
    {
        return None;
    }
    sum([
        cell(&value.current_user),
        cell(&value.session_user),
        cell(&value.search_path),
    ])
}
pub(super) fn facts<S: Storage>(value: &ServerFacts<S>) -> Option<usize> {
    sum([
        cell(&value.server_version),
        cell(&value.encoding),
        cell(&value.locale),
        cell(&value.timezone),
    ])
}

fn rows<T: Serialize>(
    section: &ServerSection<ServerRows<T>>,
    cap: usize,
    bytes: usize,
    row_bytes: impl Fn(&T) -> Option<usize>,
    name: impl Fn(&T) -> &str,
) -> Option<usize> {
    encoded(section, bytes)?;
    let ServerSection::Loaded(value) = section else {
        return Some(0);
    };
    if value.rows.len() > cap || value.rows.capacity() > cap {
        return None;
    }
    if value.limit == Some(ServerLimit::RowLimit) && value.rows.len() != cap {
        return None;
    }
    let mut total = value.rows.capacity().checked_mul(size_of::<T>())?;
    for (index, row) in value.rows.iter().enumerate() {
        if value.rows[..index]
            .iter()
            .any(|previous| name(previous) == name(row))
        {
            return None;
        }
        total = total.checked_add(row_bytes(row)?)?;
        if total > bytes {
            return None;
        }
    }
    Some(total)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Header<'a, S = String> {
    pub database: &'a S,
    pub reader_pid: i32,
    pub collected_start: &'a str,
    pub collected_end: &'a str,
    pub reader: &'a ReaderContext<S>,
    pub facts: &'a ServerSection<ServerFacts<S>>,
}

pub(super) fn snapshot(value: &ServerDetailsSnapshot) -> Option<usize> {
    if value.reader_pid <= 0
        || value.collected_start.len() > 64
        || value.collected_end.len() > 64
        || value.collected_start.is_empty()
        || value.collected_end.is_empty()
    {
        return None;
    }
    let facts_heap = match &value.facts {
        ServerSection::Loaded(f) => facts(f)?,
        _ => 0,
    };
    let header = size_of::<ServerDetailsSnapshot>().checked_add(sum([
        identity(&value.database),
        reader(&value.reader),
        Some(facts_heap),
        Some(value.collected_start.capacity()),
        Some(value.collected_end.capacity()),
    ])?)?;
    if header > HEADER_BYTES {
        return None;
    }
    encoded(
        &Header {
            database: &value.database,
            reader_pid: value.reader_pid,
            collected_start: &value.collected_start,
            collected_end: &value.collected_end,
            reader: &value.reader,
            facts: &value.facts,
        },
        HEADER_BYTES - 128,
    )?;
    let settings = rows(
        &value.settings,
        MAX_SERVER_SETTINGS,
        SETTINGS_BYTES,
        setting,
        |row| &row.name,
    )?;
    let extensions = rows(
        &value.extensions,
        MAX_SERVER_EXTENSIONS,
        EXTENSIONS_BYTES,
        extension,
        |row| &row.name,
    )?;
    let total = header.checked_add(settings)?.checked_add(extensions)?;
    if total > MAX_SERVER_DETAILS_BYTES {
        return None;
    }
    encoded(value, MAX_SERVER_DETAILS_BYTES)?;
    Some(total)
}

/// Preflight uses borrowed wire rows with the exact DTO serialization. The
/// vector's complete backing store is admitted before any row strings exist.
pub(super) struct RowBudget {
    pub heap: usize,
    encoded: usize,
    maximum: usize,
}
impl RowBudget {
    pub fn new<T>(cap: usize, maximum: usize) -> Result<(Self, Vec<T>), CatalogError> {
        let heap = cap
            .checked_mul(size_of::<T>())
            .ok_or(CatalogError::ServerDetailsLimit)?;
        if heap > maximum {
            return Err(CatalogError::ServerDetailsLimit);
        }
        let rows = Vec::with_capacity(cap);
        if rows.capacity() != cap {
            return Err(CatalogError::ServerDetailsLimit);
        }
        // Section/array wrappers, separators and the eventual limit marker.
        Ok((
            Self {
                heap,
                encoded: 128,
                maximum,
            },
            rows,
        ))
    }
    pub fn admit(&mut self, row: &impl Serialize, strings: usize) -> bool {
        let Some(bytes) = encoded(row, self.maximum).and_then(|n| n.checked_add(1)) else {
            return false;
        };
        if strings > self.maximum.saturating_sub(self.heap)
            || bytes > self.maximum.saturating_sub(self.encoded)
        {
            return false;
        }
        self.heap += strings;
        self.encoded += bytes;
        true
    }
}

pub(super) fn own(value: &str) -> Result<String, CatalogError> {
    let mut owned = String::with_capacity(value.len());
    // Do not copy into capacity that was not included in borrowed admission.
    if owned.capacity() != value.len() {
        return Err(CatalogError::ServerDetailsLimit);
    }
    owned.push_str(value);
    Ok(owned)
}
pub(super) fn own_cell(value: ServerText<&str>) -> Result<ServerText, CatalogError> {
    Ok(match value {
        ServerText::Value(value) => ServerText::Value(own(value)?),
        ServerText::Null => ServerText::Null,
        ServerText::Omitted { bytes } => ServerText::Omitted { bytes },
    })
}
