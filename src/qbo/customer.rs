use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Customer {
    #[serde(rename = "Id")]
    pub id: Option<String>,
    #[serde(rename = "DisplayName")]
    pub display_name: Option<String>,
    #[serde(rename = "CompanyName")]
    pub company_name: Option<String>,
    #[serde(rename = "PrimaryEmailAddr")]
    pub primary_email_addr: Option<EmailAddress>,
    #[serde(rename = "Active")]
    pub active: Option<bool>,
    #[serde(rename = "Balance")]
    pub balance: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct EmailAddress {
    #[serde(rename = "Address")]
    pub address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryEnvelope {
    #[serde(rename = "QueryResponse")]
    pub query_response: QueryResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryResponse {
    #[serde(rename = "Customer", default)]
    pub customers: Vec<Customer>,
}
