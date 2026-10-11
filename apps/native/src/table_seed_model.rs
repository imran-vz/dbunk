//! Pure seed-form parsing. Recipe text is never generated data or execution
//! authority; the backend resolves and binds it to an observed target.
use dbunk_lib::backend::table_seed::{
    MAX_TABLE_SEED_TEXT_CHARS, TableSeedColumnSpec, TableSeedGenerator, TableSeedLength,
    TableSeedLocale, TableSeedSource,
};

pub const GENERATORS: [(TableSeedGenerator, &str); 33] = [
    (TableSeedGenerator::Email, "Email"),
    (TableSeedGenerator::FirstName, "First name"),
    (TableSeedGenerator::MiddleName, "Middle name"),
    (TableSeedGenerator::LastName, "Last name"),
    (TableSeedGenerator::FullName, "Full name"),
    (TableSeedGenerator::UserName, "Username"),
    (TableSeedGenerator::Company, "Company"),
    (TableSeedGenerator::Url, "URL"),
    (TableSeedGenerator::Phone, "Phone"),
    (TableSeedGenerator::City, "City"),
    (TableSeedGenerator::Country, "Country"),
    (TableSeedGenerator::StreetAddress, "Street address"),
    (TableSeedGenerator::AddressLine2, "Address line 2"),
    (TableSeedGenerator::Region, "State or region"),
    (TableSeedGenerator::PostalCode, "Postal code"),
    (TableSeedGenerator::FullAddress, "Full address"),
    (TableSeedGenerator::CountryCode, "Country code"),
    (TableSeedGenerator::Word, "Word"),
    (TableSeedGenerator::Sentence, "Sentence"),
    (TableSeedGenerator::Paragraph, "Paragraph"),
    (TableSeedGenerator::Boolean, "Boolean"),
    (TableSeedGenerator::TinyInt, "Tiny integer"),
    (TableSeedGenerator::SmallInt, "Small integer"),
    (TableSeedGenerator::Integer, "Integer"),
    (TableSeedGenerator::BigInt, "Big integer"),
    (TableSeedGenerator::Float, "Float"),
    (TableSeedGenerator::Decimal, "Decimal"),
    (TableSeedGenerator::Price, "Price"),
    (TableSeedGenerator::Uuid, "UUID"),
    (TableSeedGenerator::Date, "Date"),
    (TableSeedGenerator::Time, "Time"),
    (TableSeedGenerator::Timestamp, "Timestamp"),
    (TableSeedGenerator::Json, "JSON"),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ColumnMode {
    #[default]
    Auto,
    Default,
    Constant,
    Values,
    Generator(TableSeedGenerator),
}
impl ColumnMode {
    /// Text generators take a length range. Auto does when the backend
    /// reports the column's Auto source as text (`auto_length`).
    pub fn accepts_length(self, auto_length: bool) -> bool {
        match self {
            ColumnMode::Auto => auto_length,
            ColumnMode::Generator(generator) => generator.accepts_length(),
            ColumnMode::Default | ColumnMode::Constant | ColumnMode::Values => false,
        }
    }
}

#[derive(Clone, Default)]
pub struct ColumnDraft {
    pub mode: ColumnMode,
    pub constant: String,
    pub values_text: String,
    pub null_percent: String,
    /// Character-length range for generated text; blank ends are automatic.
    pub min_length: String,
    pub max_length: String,
}
impl ColumnDraft {
    pub fn overridden(&self, nullable: bool, auto_length: bool) -> bool {
        self.mode != ColumnMode::Auto
            || (nullable && !self.null_percent.trim().is_empty())
            || (self.mode.accepts_length(auto_length)
                && !(self.min_length.trim().is_empty() && self.max_length.trim().is_empty()))
    }
    pub fn spec(
        &self,
        column: &str,
        nullable: bool,
        auto_length: bool,
    ) -> Result<TableSeedColumnSpec, &'static str> {
        if column.is_empty() || column.len() > 63 || column.contains('\0') {
            return Err("Seed column identity is invalid");
        }
        let null_rate = if nullable && self.mode != ColumnMode::Default {
            parse_null_rate(&self.null_percent)?
        } else {
            None
        };
        let source = match self.mode {
            ColumnMode::Auto => TableSeedSource::Auto { generator: None },
            ColumnMode::Default => TableSeedSource::Default,
            ColumnMode::Generator(generator) => TableSeedSource::Auto {
                generator: Some(generator),
            },
            ColumnMode::Constant => {
                if self.constant.len() > 8192 || self.constant.contains('\0') {
                    return Err("Seed constants are limited to 8192 UTF-8 bytes without NUL");
                }
                TableSeedSource::Constant {
                    value: self.constant.clone(),
                }
            }
            ColumnMode::Values => {
                // Match baseline comma-separated, trimmed, nonempty values, but
                // refuse an empty list instead of silently falling back to Auto.
                let values = self
                    .values_text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let count = values.clone().take(1025).count();
                if count == 0
                    || count > 1024
                    || values.clone().any(|s| s.len() > 8192 || s.contains('\0'))
                    || self.values_text.len() > 128 * 1024
                {
                    return Err("Use 1–1024 comma-separated values, each at most 8192 UTF-8 bytes");
                }
                let mut captured = Vec::with_capacity(count);
                captured.extend(values.map(str::to_owned));
                TableSeedSource::Values { values: captured }
            }
        };
        let length = if self.mode.accepts_length(auto_length) {
            parse_length(&self.min_length, &self.max_length)?
        } else {
            None
        };
        Ok(TableSeedColumnSpec {
            column: column.to_owned(),
            source,
            null_rate,
            length,
        })
    }
}

/// Parse the full unsigned integer without routing it through a floating point
/// value. An empty field requests a backend-selected seed, frozen in review.
pub fn parse_seed(value: &str) -> Result<Option<u64>, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Seed must contain only decimal digits");
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| "Seed must be at most 18446744073709551615")
}

pub fn parse_row_count(value: &str) -> Result<u32, &'static str> {
    let count = parse_seed(value)
        .map_err(|_| "Rows must be a whole number from 1 to 1000000")?
        .filter(|count| (1..=1_000_000).contains(count))
        .ok_or("Rows must be a whole number from 1 to 1000000")?;
    Ok(count as u32)
}

/// Either end may be blank (automatic); both blank means no range.
pub fn parse_length(min: &str, max: &str) -> Result<Option<TableSeedLength>, &'static str> {
    const MESSAGE: &str = "Lengths must be whole numbers from 1 to 100000, minimum at most maximum";
    let parse = |value: &str| -> Result<Option<u32>, &'static str> {
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(MESSAGE);
        }
        value
            .parse::<u32>()
            .ok()
            .filter(|length| (1..=MAX_TABLE_SEED_TEXT_CHARS).contains(length))
            .map(Some)
            .ok_or(MESSAGE)
    };
    let (min, max) = (parse(min)?, parse(max)?);
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Err(MESSAGE);
    }
    Ok((min.is_some() || max.is_some()).then_some(TableSeedLength { min, max }))
}

/// Comma-separated locale ids; blank means en-US.
pub fn parse_locales(value: &str) -> Result<Vec<TableSeedLocale>, &'static str> {
    const MESSAGE: &str = "Locales are comma-separated, without repeats, from: en-US, en-GB, en-IN, de, fr, es, pt-BR, ja";
    let mut locales = Vec::new();
    for id in value.split(',').map(str::trim).filter(|id| !id.is_empty()) {
        let locale = TableSeedLocale::ALL
            .into_iter()
            .find(|locale| locale.id().eq_ignore_ascii_case(id))
            .ok_or(MESSAGE)?;
        // Repeats are refused, so the list never outgrows the eight locales
        // (MAX_TABLE_SEED_LOCALES); the backend checks the bound again.
        if locales.contains(&locale) {
            return Err(MESSAGE);
        }
        locales.push(locale);
    }
    if locales.is_empty() {
        locales.push(TableSeedLocale::EnUs);
    }
    Ok(locales)
}

pub fn locales_text(locales: &[TableSeedLocale]) -> String {
    locales
        .iter()
        .map(|locale| locale.id())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn parse_null_rate(value: &str) -> Result<Option<f64>, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let percent = value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or("NULL percentage must be a finite number from 0 to 100")?;
    Ok(Some(percent / 100.0))
}

#[cfg(test)]
mod tests;
