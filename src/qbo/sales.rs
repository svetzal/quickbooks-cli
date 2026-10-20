//! Narrow, reviewed Canadian service sales. Preparation performs GETs only.
use crate::{
    config::QboEnvironment,
    http::{self, Transport},
    qbo::client::QboClient,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MINOR: &str = "75";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Estimate,
    Invoice,
}
impl Kind {
    fn entity(self) -> &'static str {
        match self {
            Self::Estimate => "Estimate",
            Self::Invoice => "Invoice",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Estimate => "estimate",
            Self::Invoice => "invoice",
        }
    }
}

/// Deliberately not an unrestricted Intuit payload. Money is decimal text.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sale {
    pub customer: String,
    pub date: String,
    pub due_date: Option<String>,
    pub terms: Option<String>,
    pub recipient: String,
    pub note: String,
    pub lines: Vec<Line>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub item: String,
    pub description: String,
    pub quantity: u32,
    pub rate: String,
    pub tax_code: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    version: u32,
    environment: QboEnvironment,
    realm: String,
    kind: Kind,
    operation: String,
    source: Value,
    /// Remote evidence and calculated totals are included in the approval digest.
    review: Value,
    payload: Value,
}

fn id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 40 && value.bytes().all(|b| b.is_ascii_digit()),
        "expected numeric QuickBooks ID"
    );
    Ok(())
}
fn email(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 254
            && value.is_ascii()
            && !value
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            && value.matches('@').count() == 1
            && value.split('@').all(|s| !s.is_empty())
            && !value.contains([',', ';', '?', '#', '&']),
        "one explicit email recipient required"
    );
    Ok(())
}
fn date(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    ensure!(
        bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()),
        "date must be YYYY-MM-DD"
    );
    let y: u32 = value[..4].parse()?;
    let m: u32 = value[5..7].parse()?;
    let d: u32 = value[8..].parse()?;
    let days = match m {
        4 | 6 | 9 | 11 => 30,
        2 if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    ensure!(
        (1900..=9999).contains(&y) && d > 0 && d <= days,
        "invalid calendar date"
    );
    Ok(())
}
/// Fixed two-place decimal, checked integer arithmetic throughout; no f64.
fn cents(text: &str) -> Result<i64> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    ensure!(
        !whole.is_empty()
            && whole.len() <= 9
            && fraction.len() <= 2
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.bytes().all(|b| b.is_ascii_digit()),
        "expected nonnegative decimal with at most two fractional places"
    );
    Ok(whole.parse::<i64>()? * 100
        + match fraction.len() {
            0 => 0,
            1 => fraction.parse::<i64>()? * 10,
            _ => fraction.parse()?,
        })
}
fn money(n: i64) -> String {
    format!("{}.{:02}", n / 100, n % 100)
}
fn number(n: i64) -> Result<Value> {
    Ok(serde_json::from_str(&money(n))?)
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().context("required QuickBooks field missing")
}
fn active(v: &Value) -> Result<()> {
    ensure!(v["Active"] == true, "inactive or unknown reference");
    Ok(())
}
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<_, _> = m.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
            json!(sorted)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => v.clone(),
    }
}
impl Proposal {
    pub fn digest(&self) -> Result<String> {
        digest_value(&serde_json::to_value(self)?)
    }
    pub fn check_execution(
        &self,
        environment: QboEnvironment,
        realm: &str,
        journal: &Journal,
    ) -> Result<()> {
        ensure!(
            environment == QboEnvironment::Sandbox,
            "production writes disabled"
        );
        ensure!(
            self.version == VERSION && self.environment == environment && self.realm == realm,
            "proposal version/environment/company mismatch"
        );
        journal.require_approval(&self.digest()?)
    }

    pub fn display(&self) -> Result<String> {
        Ok(format!(
            "REVIEW ONLY — {} {} / {} / realm {}\nApproval digest: {}\n{}\nExact HTTP payload:\n{}",
            self.operation,
            self.kind.path(),
            self.environment,
            self.realm,
            self.digest()?,
            serde_json::to_string_pretty(&self.review)?,
            serde_json::to_string_pretty(&self.payload)?
        ))
    }
}

pub struct Sales<'a, T> {
    transport: &'a T,
    builder: reqwest::Client,
    environment: QboEnvironment,
    realm: &'a str,
    token: &'a str,
}
impl QboClient {
    pub fn sales(&self) -> Sales<'_, reqwest::Client> {
        Sales {
            transport: &self.http,
            builder: self.http.clone(),
            environment: self.environment,
            realm: &self.realm_id,
            token: &self.access_token,
        }
    }
}
impl<'a, T: Transport> Sales<'a, T> {
    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        id(self.realm)?;
        Ok(self
            .builder
            .request(
                method,
                format!(
                    "{}/v3/company/{}/{}",
                    self.environment.api_base_url(),
                    self.realm,
                    path
                ),
            )
            .bearer_auth(self.token)
            .header("Accept", "application/json")
            .query(&[("minorversion", MINOR)]))
    }
    pub async fn lookup(&self, entity: &str, key: &str) -> Result<Value> {
        let name = match entity {
            "companyinfo" => "CompanyInfo",
            "preferences" => "Preferences",
            "customer" => "Customer",
            "item" => "Item",
            "taxcode" => "TaxCode",
            "taxrate" => "TaxRate",
            "term" => "Term",
            "estimate" => "Estimate",
            "invoice" => "Invoice",
            _ => bail!("unsupported lookup entity"),
        };
        id(key)?;
        let reply = self
            .transport
            .send(
                self.request(
                    reqwest::Method::GET,
                    &if entity == "preferences" {
                        "preferences".into()
                    } else {
                        format!("{entity}/{key}")
                    },
                )?
                .build()?,
            )
            .await?;
        if !reply.status.is_success() && !matches!(reply.status.as_u16(), 401 | 403) {
            bail!(
                "QuickBooks lookup rejected (HTTP {}); no document operation attempted",
                reply.status
            );
        }
        let envelope: Value = http::parse(reply)?;
        let value = envelope
            .get(name)
            .context("missing entity in response")?
            .clone();
        if !matches!(entity, "preferences" | "companyinfo") {
            ensure!(text(&value, "Id")? == key, "lookup returned wrong ID");
        }
        Ok(value)
    }
    async fn company(&self) -> Result<Value> {
        let company = self.lookup("companyinfo", self.realm).await?;
        let prefs = self.lookup("preferences", self.realm).await?;
        ensure!(
            company["Country"] == "CA",
            "only Canadian companies supported"
        );
        ensure!(
            prefs["CurrencyPrefs"]["HomeCurrency"]["value"] == "CAD"
                && prefs["CurrencyPrefs"]["MultiCurrencyEnabled"] == false,
            "only single-currency CAD supported"
        );
        ensure!(
            prefs["TaxPrefs"]["UsingSalesTax"] == true,
            "sales tax setup required"
        );
        ensure!(
            !text(&company, "CompanyName")?.is_empty(),
            "company identity missing"
        );
        Ok(json!({"company":company,"preferences":prefs}))
    }
    pub async fn prepare(&self, kind: Kind, sale: Sale) -> Result<Proposal> {
        id(&sale.customer)?;
        email(&sale.recipient)?;
        date(&sale.date)?;
        ensure!(
            !sale.lines.is_empty()
                && sale.lines.len() <= 50
                && sale.note.len() <= 1000
                && !sale.note.chars().any(char::is_control),
            "unsupported lines or note"
        );
        match kind {
            Kind::Invoice => {
                let due = sale
                    .due_date
                    .as_deref()
                    .context("invoice requires explicit due date")?;
                date(due)?;
                ensure!(due >= sale.date.as_str(), "due date precedes transaction");
            }
            Kind::Estimate => ensure!(
                sale.due_date.is_none() && sale.terms.is_none(),
                "estimate does not support terms or due date"
            ),
        }
        let identity = self.company().await?;
        let customer = self.lookup("customer", &sale.customer).await?;
        active(&customer)?;
        ensure!(
            customer["CurrencyRef"]["value"]
                .as_str()
                .is_none_or(|s| s == "CAD"),
            "customer currency unsupported"
        );
        ensure!(
            customer["IsProject"].as_bool() != Some(true),
            "projects unsupported"
        );
        let terms = if let Some(key) = &sale.terms {
            let t = self.lookup("term", key).await?;
            active(&t)?;
            Some(t)
        } else {
            None
        };
        let mut evidence = Vec::new();
        let mut lines = Vec::new();
        let mut subtotal = 0_i64;
        let mut bases: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for line in &sale.lines {
            ensure!(
                line.quantity > 0
                    && line.quantity <= 10000
                    && !line.description.is_empty()
                    && line.description.len() <= 1000
                    && !line.description.chars().any(char::is_control),
                "unsupported service line"
            );
            let rate = cents(&line.rate)?;
            ensure!(rate > 0, "positive service rate required");
            let amount = rate
                .checked_mul(i64::from(line.quantity))
                .context("amount overflow")?;
            subtotal = subtotal.checked_add(amount).context("subtotal overflow")?;
            let item = self.lookup("item", &line.item).await?;
            active(&item)?;
            ensure!(item["Type"] == "Service", "only service items supported");
            let code = self.lookup("taxcode", &line.tax_code).await?;
            active(&code)?;
            let details = code["SalesTaxRateList"]["TaxRateDetail"]
                .as_array()
                .context("sales tax rate details required")?;
            ensure!(
                details.len() == 1
                    && details[0]["TaxTypeApplicable"] == "TaxOnAmount"
                    && details[0]["TaxOrder"] == 0,
                "only one non-compounding sales rate supported"
            );
            let rate_id = text(&details[0]["TaxRateRef"], "value")?;
            let tax = self.lookup("taxrate", rate_id).await?;
            active(&tax)?;
            ensure!(
                tax["SpecialTaxType"].as_str().is_none_or(|s| s == "NONE"),
                "special taxes unsupported"
            );
            ensure!(
                tax["EffectiveTaxRate"].is_null(),
                "effective-dated taxes unsupported"
            );
            let percent = cents(&tax["RateValue"].to_string())?;
            ensure!(percent <= 10000, "unsupported tax percentage");
            let entry = bases.entry(rate_id.to_string()).or_insert((0, percent));
            ensure!(entry.1 == percent, "tax rate changed during preparation");
            entry.0 = entry.0.checked_add(amount).context("tax base overflow")?;
            evidence.push(json!({"item":item,"tax_code":code,"tax_rate":tax,"quantity":line.quantity,"rate":money(rate),"amount":money(amount)}));
            lines.push(json!({"Amount":number(amount)?,"Description":line.description,"DetailType":"SalesItemLineDetail","SalesItemLineDetail":{"ItemRef":{"value":line.item},"Qty":line.quantity,"UnitPrice":number(rate)?,"TaxCodeRef":{"value":line.tax_code}}}));
        }
        let mut estimated_tax = 0_i64;
        for (base, percent) in bases.values() {
            estimated_tax = estimated_tax
                .checked_add(
                    base.checked_mul(*percent)
                        .context("tax overflow")?
                        .checked_add(5000)
                        .context("tax rounding overflow")?
                        / 10000,
                )
                .context("tax total overflow")?;
        }
        let total = subtotal
            .checked_add(estimated_tax)
            .context("total overflow")?;
        ensure!(
            total <= 99_999_999_999,
            "total exceeds supported decimal range"
        );
        let mut payload = json!({"CustomerRef":{"value":sale.customer},"TxnDate":sale.date,"CurrencyRef":{"value":"CAD"},"GlobalTaxCalculation":"TaxExcluded","Line":lines,"CustomerMemo":{"value":sale.note},"BillEmail":{"Address":sale.recipient},"EmailStatus":"NotSet"});
        if let Some(due) = &sale.due_date {
            payload["DueDate"] = json!(due);
        }
        if let Some(key) = &sale.terms {
            payload["SalesTermRef"] = json!({"value":key});
        }
        Ok(Proposal {
            version: VERSION,
            environment: self.environment,
            realm: self.realm.into(),
            kind,
            operation: "create".into(),
            source: serde_json::to_value(&sale)?,
            review: json!({"identity":identity,"customer":customer,"terms":terms,"lines":evidence,"currency":"CAD","tax_basis":"TaxExcluded; rounded per rate on aggregate base; QuickBooks calculates final tax","subtotal":money(subtotal),"estimated_tax":money(estimated_tax),"estimated_total":money(total),"recipient":sale.recipient,"date":sale.date,"due_date":sale.due_date,"note":sale.note}),
            payload,
        })
    }
    pub async fn prepare_send(&self, kind: Kind, key: &str, recipient: &str) -> Result<Proposal> {
        id(key)?;
        email(recipient)?;
        let identity = self.company().await?;
        let document = self.lookup(kind.path(), key).await?;
        ensure!(
            document["CurrencyRef"]["value"] == "CAD"
                && document["GlobalTaxCalculation"] == "TaxExcluded"
                && document["EmailStatus"] == "NotSet",
            "only unsent CAD tax-excluded documents supported"
        );
        text(&document, "SyncToken")?;
        ensure!(
            document["TotalAmt"].is_number() && document["TxnTaxDetail"]["TotalTax"].is_number(),
            "remote totals required"
        );
        let lines = document["Line"]
            .as_array()
            .context("remote lines required")?;
        ensure!(
            !lines.is_empty()
                && lines.iter().all(|l| matches!(
                    l["DetailType"].as_str(),
                    Some("SalesItemLineDetail" | "SubTotalLineDetail")
                )),
            "unsupported remote line type"
        );
        // Only documents created and recorded by this release can be sent (checked by journal).
        Ok(Proposal {
            version: VERSION,
            environment: self.environment,
            realm: self.realm.into(),
            kind,
            operation: "send".into(),
            source: json!({"id":key,"recipient":recipient}),
            review: json!({"identity":identity,"document":document,"recipient":recipient}),
            payload: Value::Null,
        })
    }
    pub async fn execute(&self, proposal: &Proposal, journal: &Journal) -> Result<Value> {
        proposal.check_execution(self.environment, self.realm, journal)?;
        let digest = proposal.digest()?;
        if let Some(result) = journal.result(&digest)? {
            return Ok(result);
        }
        let current = match proposal.operation.as_str() {
            "create" => {
                self.prepare(
                    proposal.kind,
                    serde_json::from_value(proposal.source.clone())?,
                )
                .await?
            }
            "send" => {
                self.prepare_send(
                    proposal.kind,
                    text(&proposal.source, "id")?,
                    text(&proposal.source, "recipient")?,
                )
                .await?
            }
            _ => bail!("unsupported proposal operation"),
        };
        ensure!(
            &current == proposal,
            "preflight changed; prepare and review again"
        );
        if proposal.operation == "send" {
            journal.require_created(proposal)?;
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let mut request = if proposal.operation == "create" {
            self.request(reqwest::Method::POST, proposal.kind.path())?
                .json(&proposal.payload)
        } else {
            self.request(
                reqwest::Method::POST,
                &format!(
                    "{}/{}/send",
                    proposal.kind.path(),
                    text(&proposal.source, "id")?
                ),
            )?
            .query(&[("sendTo", text(&proposal.source, "recipient")?)])
            .header("Content-Type", "application/octet-stream")
            .body(Vec::new())
        };
        request = request.query(&[("requestid", &request_id)]);
        let request = request.build()?;
        // Durable exclusive claim precedes the first POST. Every failure thereafter is unknown.
        journal.claim(&digest, proposal, &request_id)?;
        if proposal.operation == "send" {
            let key = digest_value(&json!([
                proposal.kind,
                proposal.realm,
                text(&proposal.source, "id")?
            ]))?;
            journal.write_new(&journal.path(&key, "send-attempt")?, request_id.as_bytes())?;
        } else {
            // Review evidence can change while the exact accounting payload stays the same.
            // A fresh approval must not bypass an earlier ambiguous identical creation.
            let key = digest_value(&json!([
                proposal.environment,
                proposal.realm,
                proposal.kind,
                proposal.payload
            ]))?;
            journal.write_new(
                &journal.path(&key, "create-attempt")?,
                &serde_json::to_vec(&json!({"digest":digest,"requestid":request_id}))?,
            )?;
        }
        let reply = self.transport.send(request).await.map_err(|_| {
            anyhow::anyhow!(
                "outcome unknown; reconcile journal and remote document; never blindly retry"
            )
        })?;
        ensure!(
            reply.status.is_success(),
            "write not confirmed; reconcile journal; no automatic retry"
        );
        let body: Value = serde_json::from_slice(&reply.body)
            .map_err(|_| anyhow::anyhow!("write response invalid; outcome unknown; reconcile"))?;
        let document = body
            .get(proposal.kind.entity())
            .context("write outcome unknown; missing document; reconcile")?;
        id(text(document, "Id")?)?;
        if proposal.operation == "send" {
            ensure!(
                document["Id"] == proposal.source["id"] && document["EmailStatus"] == "EmailSent",
                "send not confirmed; reconcile without retry"
            );
        } else {
            ensure!(
                document["EmailStatus"] == "NotSet",
                "unexpected email state; reconcile without retry"
            );
        }
        journal.complete(&digest, proposal, document)?;
        Ok(document.clone())
    }
}

/// Private durable records contain proposals and identifiers, never authorization material.
pub struct Journal {
    root: PathBuf,
}
impl Journal {
    pub fn open(root: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            match fs::DirBuilder::new().mode(0o700).create(root) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            let meta = fs::symlink_metadata(root)?;
            ensure!(
                meta.is_dir()
                    && !meta.file_type().is_symlink()
                    && meta.permissions().mode() & 0o077 == 0,
                "journal must be a private directory (0700)"
            );
            Ok(Self { root: root.into() })
        }
        #[cfg(not(unix))]
        {
            let _ = root;
            bail!("durable private journal requires Unix permissions")
        }
    }
    fn path(&self, key: &str, extension: &str) -> Result<PathBuf> {
        ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid proposal digest"
        );
        Ok(self.root.join(format!("{key}.{extension}")))
    }
    fn write_new(&self, path: &Path, data: &[u8]) -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .context("record already exists or cannot be claimed; reconcile instead of retrying")?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }
    pub fn save(&self, proposal: &Proposal) -> Result<String> {
        let digest = proposal.digest()?;
        if self.path(&digest, "proposal")?.try_exists()? {
            ensure!(
                self.load(&digest)? == *proposal,
                "existing proposal differs"
            );
            return Ok(digest);
        }
        self.write_new(
            &self.path(&digest, "proposal")?,
            &serde_json::to_vec_pretty(proposal)?,
        )?;
        Ok(digest)
    }
    pub fn load(&self, digest: &str) -> Result<Proposal> {
        let p: Proposal = serde_json::from_slice(&fs::read(self.path(digest, "proposal")?)?)?;
        ensure!(p.digest()? == digest, "proposal digest mismatch");
        Ok(p)
    }
    pub fn approve(&self, digest: &str) -> Result<()> {
        let p = self.load(digest)?;
        ensure!(
            p.environment == QboEnvironment::Sandbox && p.version == VERSION,
            "only current sandbox proposals can be approved"
        );
        self.write_new(&self.path(digest, "approval")?, digest.as_bytes())
    }
    fn require_approval(&self, digest: &str) -> Result<()> {
        ensure!(
            fs::read(self.path(digest, "approval")?).context("explicit approval required")?
                == digest.as_bytes(),
            "approval mismatch"
        );
        Ok(())
    }
    fn claim(&self, digest: &str, p: &Proposal, request_id: &str) -> Result<()> {
        self.write_new(&self.path(digest,"attempt")?, &serde_json::to_vec(&json!({"proposal":p,"requestid":request_id,"outcome":"unknown; reconcile manually"}))?)
    }
    pub fn result(&self, digest: &str) -> Result<Option<Value>> {
        let path = self.path(digest, "result")?;
        if path.try_exists()? {
            return Ok(Some(serde_json::from_slice(&fs::read(path)?)?));
        }
        ensure!(
            !self.path(digest, "attempt")?.try_exists()?,
            "previous outcome unknown or in progress; read journal requestid and reconcile remotely; do not retry"
        );
        Ok(None)
    }
    fn complete(&self, digest: &str, p: &Proposal, document: &Value) -> Result<()> {
        if p.operation == "create" {
            let key = digest_value(&json!([p.kind, p.realm, text(document, "Id")?]))?;
            self.write_new(&self.path(&key, "created")?, digest.as_bytes())?;
        }
        self.write_new(
            &self.path(digest, "result")?,
            &serde_json::to_vec(document)?,
        )
    }
    fn require_created(&self, p: &Proposal) -> Result<()> {
        let key = digest_value(&json!([p.kind, p.realm, text(&p.source, "id")?]))?;
        let digest = fs::read_to_string(self.path(&key, "created")?)
            .context("sending requires a document created and recorded by this release")?;
        let original: Value = serde_json::from_slice(&fs::read(self.path(&digest, "result")?)?)?;
        ensure!(
            original == p.review["document"],
            "document changed since creation; remote edits unsupported in this release"
        );
        ensure!(
            !self.path(&key, "send-attempt")?.try_exists()?,
            "sending already attempted; reconcile manually"
        );
        Ok(())
    }
    pub fn inspect(&self, digest: &str) -> Result<String> {
        let p = self.load(digest)?;
        let attempt = self.path(digest, "attempt")?;
        let status = if attempt.try_exists()? {
            fs::read_to_string(attempt)?
        } else {
            "not attempted".into()
        };
        let result_path = self.path(digest, "result")?;
        let confirmation = if result_path.try_exists()? {
            format!("confirmed: {}", fs::read_to_string(result_path)?)
        } else {
            "not confirmed".into()
        };
        Ok(format!(
            "{}\nAttempt record: {}\nOutcome: {}\nUse read-only lookup to reconcile unknown outcomes; this command never retries.",
            p.display()?,
            status,
            confirmation
        ))
    }
}
fn digest_value(value: &Value) -> Result<String> {
    Ok(ring::digest::digest(
        &ring::digest::SHA256,
        &serde_json::to_vec(&canonical(value))?,
    )
    .as_ref()
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect())
}

#[cfg(test)]
mod tests;
