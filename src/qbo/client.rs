use anyhow::{Context, Result, bail};
use reqwest::StatusCode;

use crate::{
    auth,
    cli::CustomerListArgs,
    config::QboConfig,
    qbo::customer::{Customer, QueryEnvelope},
    secrets,
};

pub struct QboClient {
    http: reqwest::Client,
    base_url: &'static str,
    realm_id: String,
    access_token: String,
}

impl QboClient {
    pub async fn from_config(config: &mut QboConfig) -> Result<Self> {
        let realm_id = config
            .realm_id
            .clone()
            .context("missing QBO realm ID; run `qbo auth login` first")?;

        let token_response = auth::refresh_access_token(config).await?;
        secrets::set_access_token(&token_response.access_token)?;
        secrets::set_refresh_token(&token_response.refresh_token)?;

        Ok(Self {
            http: reqwest::Client::new(),
            base_url: config.environment.api_base_url(),
            realm_id,
            access_token: token_response.access_token,
        })
    }

    pub async fn list_customers(&self, args: &CustomerListArgs) -> Result<Vec<Customer>> {
        let mut customers = Vec::new();
        let mut start_position = 1_u32;

        loop {
            let query = customer_query(args, start_position);
            let page = self.query_customers(&query).await?;
            let page_len = page.len();
            customers.extend(page);

            if !args.all || page_len < args.limit as usize {
                break;
            }

            start_position += args.limit as u32;
        }

        Ok(customers)
    }

    async fn query_customers(&self, query: &str) -> Result<Vec<Customer>> {
        let url = format!("{}/v3/company/{}/query", self.base_url, self.realm_id);
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.access_token)
            .header(reqwest::header::ACCEPT, "application/json")
            .query(&[("query", query)])
            .send()
            .await
            .context("failed to call QBO customer query endpoint")?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        if status != StatusCode::OK {
            bail!("QBO customer query returned {status}: {body}");
        }

        let envelope: QueryEnvelope =
            serde_json::from_str(&body).context("failed to parse QBO customer response")?;
        Ok(envelope.query_response.customers)
    }
}

fn customer_query(args: &CustomerListArgs, start_position: u32) -> String {
    let mut query = "SELECT * FROM Customer".to_string();
    let mut filters = Vec::new();

    if let Some(name) = &args.name {
        filters.push(format!("DisplayName LIKE '{}%'", escape_qbo_string(name)));
    }

    if args.include_inactive {
        filters.push("Active IN (true, false)".to_string());
    }

    if !filters.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&filters.join(" AND "));
    }

    query.push_str(" ORDERBY DisplayName");
    query.push_str(&format!(
        " STARTPOSITION {} MAXRESULTS {}",
        start_position, args.limit
    ));
    query
}

fn escape_qbo_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use crate::{cli::OutputFormat, qbo::client::customer_query};

    use super::*;

    #[test]
    fn builds_default_customer_query() {
        let args = CustomerListArgs {
            limit: 100,
            all: false,
            name: None,
            include_inactive: false,
            format: OutputFormat::Table,
        };

        assert_eq!(
            customer_query(&args, 1),
            "SELECT * FROM Customer ORDERBY DisplayName STARTPOSITION 1 MAXRESULTS 100"
        );
    }

    #[test]
    fn builds_filtered_customer_query() {
        let args = CustomerListArgs {
            limit: 50,
            all: false,
            name: Some("Adam's".to_string()),
            include_inactive: true,
            format: OutputFormat::Json,
        };

        assert_eq!(
            customer_query(&args, 51),
            "SELECT * FROM Customer WHERE DisplayName LIKE 'Adam\\'s%' AND Active IN (true, false) ORDERBY DisplayName STARTPOSITION 51 MAXRESULTS 50"
        );
    }
}
