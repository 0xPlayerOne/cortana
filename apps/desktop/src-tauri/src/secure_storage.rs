use keyring::{Entry, Error as KeyringError};

const SERVICE_NAME: &str = "ai.cortana.desktop";

/// The native credential-store adapter used only after an explicit Desktop
/// migration. Headless/runtime configurations continue to use their
/// environment or owner-only secret file paths.
pub(crate) fn get(name: &str) -> Result<Option<String>, String> {
    let entry = entry(name)?;
    match entry.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Ok(None),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(format!("read secure storage entry: {error}")),
    }
}

pub(crate) fn set(name: &str, value: &str) -> Result<(), String> {
    entry(name)?
        .set_password(value)
        .map_err(|error| format!("write secure storage entry: {error}"))
}

pub(crate) fn clear(name: &str) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(format!("remove secure storage entry: {error}")),
    }
}

#[cfg(test)]
pub(crate) fn entry_name(name: &str) -> String {
    format!("{SERVICE_NAME}:{name}")
}

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE_NAME, name).map_err(|error| format!("open secure storage entry: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable_and_do_not_include_secret_values() {
        assert_eq!(
            entry_name("CORTANA_QUERY_API_KEY"),
            "ai.cortana.desktop:CORTANA_QUERY_API_KEY"
        );
        assert!(!entry_name("CORTANA_QUERY_API_KEY").contains("private"));
    }
}
