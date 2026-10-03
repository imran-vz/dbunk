//! Pure seed-form parsing. Recipe text is never generated data or execution
//! authority; the backend resolves and binds it to an observed target.
use dbunk_lib::backend::table_seed::{TableSeedColumnSpec, TableSeedGenerator, TableSeedSource};

pub const GENERATORS: [(TableSeedGenerator, &str); 26] = [
    (TableSeedGenerator::Email, "Email"),
    (TableSeedGenerator::FirstName, "First name"),
    (TableSeedGenerator::LastName, "Last name"),
    (TableSeedGenerator::FullName, "Full name"),
    (TableSeedGenerator::UserName, "Username"),
    (TableSeedGenerator::Company, "Company"),
    (TableSeedGenerator::Url, "URL"),
    (TableSeedGenerator::Phone, "Phone"),
    (TableSeedGenerator::City, "City"),
    (TableSeedGenerator::Country, "Country"),
    (TableSeedGenerator::StreetAddress, "Street address"),
    (TableSeedGenerator::Word, "Word"),
    (TableSeedGenerator::Sentence, "Sentence"),
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

#[derive(Clone, Default)]
pub struct ColumnDraft {
    pub mode: ColumnMode,
    pub constant: String,
    pub values_text: String,
    pub null_percent: String,
}
impl ColumnDraft {
    pub fn overridden(&self, nullable: bool) -> bool {
        self.mode != ColumnMode::Auto || (nullable && !self.null_percent.trim().is_empty())
    }
    pub fn spec(&self, column: &str, nullable: bool) -> Result<TableSeedColumnSpec, &'static str> {
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
        Ok(TableSeedColumnSpec {
            column: column.to_owned(),
            source,
            null_rate,
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
