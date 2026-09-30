use anyhow::Result;
use keyring::Entry;

const SERVICE: &str = "mojility-qbo-cli";
const CLIENT_SECRET: &str = "client-secret";
const ACCESS_TOKEN: &str = "access-token";
const REFRESH_TOKEN: &str = "refresh-token";

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
    for account in [CLIENT_SECRET, ACCESS_TOKEN, REFRESH_TOKEN] {
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
