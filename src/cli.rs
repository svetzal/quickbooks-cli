use std::net::SocketAddr;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::config::QboEnvironment;

#[derive(Debug, Parser)]
#[command(name = "qbo")]
#[command(about = "QuickBooks Online command-line tools for Mojility")]
#[command(after_help = "Support: info@mojility.ca")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(subcommand)]
    Auth(AuthCommand),
    #[command(subcommand)]
    Customer(CustomerCommand),
    #[command(subcommand)]
    Document(DocumentCommand),
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Connect this CLI to a QuickBooks Online company.
    Login(LoginArgs),
    /// Show local auth/configuration state.
    Status,
    /// Revoke Intuit authorization, then remove local credentials.
    Disconnect,
    /// Remove local auth/configuration state without revocation.
    Logout,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(long, env = "QBO_AUTH_BROKER_URL")]
    pub broker_url: Option<String>,
    #[arg(long, env = "QBO_CLIENT_ID")]
    pub client_id: Option<String>,
    /// Read the direct OAuth client secret from stdin; otherwise use the secret store.
    #[arg(long, requires = "client_id", conflicts_with = "broker_url")]
    pub client_secret_stdin: bool,
    #[arg(long, default_value = "http://localhost:8734/callback")]
    pub redirect_uri: String,
    #[arg(long, default_value = "127.0.0.1:8734")]
    pub callback_bind: SocketAddr,
    #[arg(long, value_enum, default_value_t = QboEnvironment::Sandbox)]
    pub environment: QboEnvironment,
    #[arg(long = "no-open-browser")]
    pub no_open_browser: bool,
}

#[derive(Debug, Subcommand)]
pub enum CustomerCommand {
    /// List customers from QuickBooks Online.
    List(CustomerListArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Args)]
pub struct CustomerListArgs {
    /// Maximum number of customers to fetch per page.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
    pub limit: u16,
    /// Fetch all pages, up to QBO's available results.
    #[arg(long)]
    pub all: bool,
    /// Match display names with a QBO LIKE query.
    #[arg(long)]
    pub name: Option<String>,
    /// Include inactive customers.
    #[arg(long)]
    pub include_inactive: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Subcommand)]
pub enum DocumentCommand {
    /// Read companyinfo, preferences, customer, item, taxcode, taxrate, term, estimate or invoice.
    Lookup { entity: String, id: String },
    /// Prepare a typed sale JSON file; performs reads only and prints an exact proposal.
    Plan {
        #[arg(value_enum)]
        kind: crate::qbo::sales::Kind,
        file: std::path::PathBuf,
    },
    /// Review current remote content and recipient for a separate sending operation.
    PlanSend {
        #[arg(value_enum)]
        kind: crate::qbo::sales::Kind,
        id: String,
        #[arg(long)]
        recipient: String,
    },
    /// Display a saved proposal and execution record, without HTTP.
    Review { digest: String },
    /// Explicitly approve this exact reviewed digest; does not execute it.
    Approve { digest: String },
    /// Execute one approved sandbox operation after read-only preflight.
    Execute { digest: String },
    /// Resolve an unknown outcome by reading one document; never retries a write.
    Reconcile {
        digest: String,
        /// QuickBooks ID of the document found during manual reconciliation.
        #[arg(long)]
        id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_review_approval_creation_and_sending_are_distinct_commands() {
        for args in [
            vec!["qbo", "document", "plan", "estimate", "service.json"],
            vec!["qbo", "document", "plan", "invoice", "service.json"],
            vec![
                "qbo",
                "document",
                "plan-send",
                "invoice",
                "90",
                "--recipient",
                "owner@example.invalid",
            ],
            vec!["qbo", "document", "review", "digest"],
            vec!["qbo", "document", "approve", "digest"],
            vec!["qbo", "document", "execute", "digest"],
            vec!["qbo", "document", "reconcile", "digest", "--id", "90"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from(["qbo", "document", "execute", "digest", "--yes"]).is_err());
        assert!(Cli::try_parse_from(["qbo", "document", "create", "--payload", "{}"]).is_err());
        assert!(Cli::try_parse_from(["qbo", "document", "plan-send", "invoice", "90"]).is_err());
        assert!(Cli::try_parse_from(["qbo", "document", "approve", "digest", "--yes"]).is_err());
        assert!(Cli::try_parse_from(["qbo", "document", "reconcile", "digest"]).is_err());
    }
}
