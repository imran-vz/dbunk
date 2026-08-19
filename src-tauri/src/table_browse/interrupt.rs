use super::protocol::TableBrowseError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Interrupt {
    #[default]
    None,
    Supersede,
    Cancel,
    Closing,
}

pub(crate) fn finalize_interrupt<T>(
    interrupt: Interrupt,
    result: Result<T, TableBrowseError>,
) -> Result<T, TableBrowseError> {
    match interrupt {
        Interrupt::None => result,
        Interrupt::Supersede => Err(TableBrowseError::Superseded),
        Interrupt::Cancel => Err(TableBrowseError::Cancelled),
        Interrupt::Closing => Err(TableBrowseError::ConnectionClosing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> TableBrowseError {
        TableBrowseError::Database {
            code: Some("42703".into()),
            message: "undefined column".into(),
            severity: Some("ERROR".into()),
            position: None,
        }
    }

    #[test]
    fn interrupt_unconditionally_determines_terminal_result() {
        assert!(matches!(
            finalize_interrupt::<()>(Interrupt::Supersede, Ok(())),
            Err(TableBrowseError::Superseded)
        ));
        assert!(matches!(
            finalize_interrupt::<()>(Interrupt::Supersede, Err(database())),
            Err(TableBrowseError::Superseded)
        ));
        assert!(matches!(
            finalize_interrupt::<()>(Interrupt::Cancel, Err(database())),
            Err(TableBrowseError::Cancelled)
        ));
        assert!(matches!(
            finalize_interrupt::<()>(Interrupt::Closing, Ok(())),
            Err(TableBrowseError::ConnectionClosing)
        ));
        assert!(matches!(
            finalize_interrupt::<()>(Interrupt::None, Err(database())),
            Err(TableBrowseError::Database { .. })
        ));
    }
}
