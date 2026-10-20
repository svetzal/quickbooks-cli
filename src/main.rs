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
            let p = journal.load(&digest)?;
            println!("{}", p.display()?);
            journal.approve(&digest)?;
            println!("Approved exact digest {digest}; nothing executed.");
        }
        other => {
            let mut config = QboConfig::load()?;
            // Refuse production execution before token refresh or any HTTP.
            if let DocumentCommand::Execute { digest } = &other {
                let p = journal.load(digest)?;
                p.check_execution(
                    config.environment,
                    config.realm_id.as_deref().unwrap_or(""),
                    &journal,
                )?;
                if let Some(result) = journal.result(digest)? {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                    return Ok(());
                }
            }
            let client = qbo::client::QboClient::from_config(&mut config).await?;
            let sales = client.sales();
            let proposal = match other {
                DocumentCommand::Lookup { entity, id } => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&sales.lookup(&entity, &id).await?)?
                    );
                    return Ok(());
                }
                DocumentCommand::Plan { kind, file } => {
                    let sale: Sale = serde_json::from_slice(&std::fs::read(file)?)?;
                    sales.prepare(kind, sale).await?
                }
                DocumentCommand::PlanSend {
                    kind,
                    id,
                    recipient,
                } => sales.prepare_send(kind, &id, &recipient).await?,
                DocumentCommand::Execute { digest } => {
                    let p = journal.load(&digest)?;
                    let document = sales.execute(&p, &journal).await?;
                    println!("{}", serde_json::to_string_pretty(&document)?);
                    return Ok(());
                }
                _ => unreachable!(),
            };
            println!("{}", proposal.display()?);
            journal.save(&proposal)?;
            println!("Saved for review. No documents created or sent.");
        }
    }
    Ok(())
}
