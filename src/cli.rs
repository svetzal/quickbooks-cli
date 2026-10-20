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
