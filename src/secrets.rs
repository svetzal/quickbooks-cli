use anyhow::Result;
use keyring::Entry;

const SERVICE: &str = "mojility-qbo-cli";
const CLIENT_SECRET: &str = "client-secret";
const ACCESS_TOKEN: &str = "access-token";
const REFRESH_TOKEN: &str = "refresh-token";
const REALM_ID: &str = "realm-id";

pub fn set_realm_id(value: &str) -> Result<()> {
    entry(REALM_ID)?.set_password(value)?;
    Ok(())
}

pub fn get_realm_id() -> Result<Option<String>> {
    match entry(REALM_ID)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn set_client_secret(secret: &str) -> Result<()> {
    entry(CLIENT_SECRET)?.set_password(secret)?;
    Ok(())
}

pub fn get_client_secret() -> Result<String> {
    Ok(entry(CLIENT_SECRET)?.get_password()?)
}

pub fn set_access_token(token: &str) -> Result<()> {
    entry(ACCESS_TOKEN)?.set_password(token)?;
    Ok(())
}

pub fn set_refresh_token(token: &str) -> Result<()> {
    entry(REFRESH_TOKEN)?.set_password(token)?;
    Ok(())
}

pub fn get_refresh_token() -> Result<String> {
    Ok(entry(REFRESH_TOKEN)?.get_password()?)
}

pub fn has_refresh_token() -> Result<bool> {
    match entry(REFRESH_TOKEN)?.get_password() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn delete_all() -> Result<()> {
    for account in [CLIENT_SECRET, ACCESS_TOKEN, REFRESH_TOKEN, REALM_ID] {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

fn entry(account: &str) -> Result<Entry> {
    Ok(Entry::new(SERVICE, account)?)
}
