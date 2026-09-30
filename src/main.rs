mod auth;
mod cli;
mod config;
mod qbo;
mod secrets;

use anyhow::Result;
use clap::Parser;
use cli::{AuthCommand, Cli, Command, CustomerCommand, OutputFormat};
use comfy_table::{Cell, Table, presets::UTF8_FULL};
use config::QboConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Auth(command) => handle_auth(command).await,
        Command::Customer(command) => handle_customer(command).await,
    }
}

async fn handle_auth(command: AuthCommand) -> Result<()> {
    match command {
        AuthCommand::Login(args) => auth::login(args).await,
        AuthCommand::Status => {
            let config = QboConfig::load()?;
            println!("Environment: {}", config.environment);
            println!("Auth mode: {}", config.auth_mode);
            println!(
                "Realm ID: {}",
                config.realm_id.as_deref().unwrap_or("not connected")
            );
            println!(
                "Client ID: {}",
                config.client_id.as_deref().unwrap_or("not configured")
            );
            println!(
                "Broker URL: {}",
                config.broker_url.as_deref().unwrap_or("not configured")
            );
            println!("Redirect URI: {}", config.redirect_uri);
            println!(
                "Refresh token: {}",
                if secrets::has_refresh_token()? {
                    "stored"
                } else {
                    "missing"
                }
            );
            Ok(())
        }
        AuthCommand::Logout => {
            QboConfig::remove()?;
            secrets::delete_all()?;
            println!("Removed local QBO auth state.");
            Ok(())
        }
    }
}

async fn handle_customer(command: CustomerCommand) -> Result<()> {
    match command {
        CustomerCommand::List(args) => {
            let mut config = QboConfig::load()?;
            let client = qbo::client::QboClient::from_config(&mut config).await?;
            config.save()?;

            let customers = client.list_customers(&args).await?;

            match args.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&customers)?);
                }
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL);
                    table.set_header(vec![
                        Cell::new("ID"),
                        Cell::new("Display Name"),
                        Cell::new("Company"),
                        Cell::new("Email"),
                        Cell::new("Active"),
                        Cell::new("Balance"),
                    ]);

                    for customer in customers {
                        table.add_row(vec![
                            Cell::new(customer.id.unwrap_or_default()),
                            Cell::new(customer.display_name.unwrap_or_default()),
                            Cell::new(customer.company_name.unwrap_or_default()),
                            Cell::new(
                                customer
                                    .primary_email_addr
                                    .and_then(|email| email.address)
                                    .unwrap_or_default(),
                            ),
                            Cell::new(customer.active.map(|v| v.to_string()).unwrap_or_default()),
                            Cell::new(customer.balance.map(|v| v.to_string()).unwrap_or_default()),
                        ]);
                    }

                    println!("{table}");
                }
            }

            Ok(())
        }
    }
}
