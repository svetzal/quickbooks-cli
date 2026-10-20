mod auth;
mod cli;
mod config;
mod http;
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
        Command::Document(command) => handle_document(command).await,
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
        AuthCommand::Disconnect => auth::disconnect().await,
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
                    table.load_style(UTF8_FULL);
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

async fn handle_document(command: cli::DocumentCommand) -> Result<()> {
    use cli::DocumentCommand;
    use qbo::sales::{Journal, Sale};
    let data =
        dirs::data_local_dir().ok_or_else(|| anyhow::anyhow!("missing local data directory"))?;
    std::fs::create_dir_all(&data)?;
    let journal = Journal::open(&data.join("qbo-sales"))?;
    match command {
        DocumentCommand::Review { digest } => println!("{}", journal.inspect(&digest)?),
        DocumentCommand::Approve { digest } => {
            let config = QboConfig::load()?;
            let proposal = journal.load(&digest)?;
            println!("{}", proposal.display()?);
            journal.approve(&digest, config.environment, connected_realm(&config)?)?;
            println!("\nApproved exact digest {digest}; nothing executed.");
        }
        DocumentCommand::Execute { digest } => {
            let mut config = QboConfig::load()?;
            let proposal = journal.load(&digest)?;
            // Refuse production, unapproved or mismatched proposals before token refresh or any HTTP.
            proposal.check_execution(config.environment, connected_realm(&config)?, &journal)?;
            if let Some(result) = journal.result(&digest)? {
                println!("Already confirmed; no request sent.");
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(());
            }
            let client = connect(&mut config).await?;
            let document = client.sales().execute(&proposal, &journal).await?;
            println!("{}", serde_json::to_string_pretty(&document)?);
        }
        DocumentCommand::Reconcile { digest, id } => {
            let mut config = QboConfig::load()?;
            let proposal = journal.load(&digest)?;
            let client = connect(&mut config).await?;
            let document = client.sales().reconcile(&proposal, &journal, &id).await?;
            println!("{}", serde_json::to_string_pretty(&document)?);
            println!("Recorded confirmed outcome from a read-only lookup; nothing was retried.");
        }
        DocumentCommand::Lookup { entity, id } => {
            let mut config = QboConfig::load()?;
            let client = connect(&mut config).await?;
            let value = client.sales().lookup(&entity, &id).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        DocumentCommand::Plan { kind, file } => {
            let sale: Sale = serde_json::from_slice(&std::fs::read(file)?)?;
            let mut config = QboConfig::load()?;
            let client = connect(&mut config).await?;
            let proposal = client.sales().prepare(kind, sale).await?;
            save_for_review(&journal, &proposal)?;
        }
        DocumentCommand::PlanSend {
            kind,
            id,
            recipient,
        } => {
            let mut config = QboConfig::load()?;
            let client = connect(&mut config).await?;
            let proposal = client.sales().prepare_send(kind, &id, &recipient).await?;
            save_for_review(&journal, &proposal)?;
        }
    }
    Ok(())
}

fn connected_realm(config: &QboConfig) -> Result<&str> {
    config
        .realm_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("missing QBO realm ID; run `qbo auth login` first"))
}

async fn connect(config: &mut QboConfig) -> Result<qbo::client::QboClient> {
    let client = qbo::client::QboClient::from_config(config).await?;
    config.save()?;
    Ok(client)
}

fn save_for_review(journal: &qbo::sales::Journal, proposal: &qbo::sales::Proposal) -> Result<()> {
    println!("{}", proposal.display()?);
    journal.save(proposal)?;
    println!("\nSaved for review. No documents created or sent.");
    Ok(())
}
