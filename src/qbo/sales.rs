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

mod display;

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
    pub recipient: Option<String>,
    pub doc_number: Option<String>,
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
/// Support only a single historic effective rate agreeing with the current rate.
/// Multiple schedules need broader date semantics and remain unsupported.
fn effective_rate_date(tax: &Value, txn_date: &str, percent: i64) -> Result<Option<String>> {
    if tax["EffectiveTaxRate"].is_null() {
        return Ok(None);
    }
    let rates = tax["EffectiveTaxRate"]
        .as_array()
        .context("invalid effective rates")?;
    ensure!(
        rates.len() == 1,
        "multiple or empty effective rates unsupported"
    );
    let effective = text(&rates[0], "EffectiveDate")?;
    let day = effective.get(..10).context("invalid effective date")?;
    date(day)?;
    ensure!(
        supported_effective_timestamp(effective) && day <= txn_date,
        "future or malformed effective tax date unsupported"
    );
    ensure!(
        cents(&rates[0]["RateValue"].to_string())? == percent,
        "effective tax rate differs from current rate"
    );
    Ok(Some(effective.into()))
}
// This release handles midnight effective dates with an explicit UTC offset.
fn supported_effective_timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    if b.get(10..19) != Some(b"T00:00:00") {
        return false;
    }
    if b.get(19..) == Some(b"Z") {
        return true;
    }
    b.len() == 25
        && matches!(b[19], b'+' | b'-')
        && b[22] == b':'
        && [20, 21, 23, 24].iter().all(|&i| b[i].is_ascii_digit())
        && &value[20..22] <= "23"
        && &value[23..25] <= "59"
}
struct TaxBase {
    base: i64,
    percent: i64,
    name: String,
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
    /// Local check, before any HTTP: the proposal belongs to the active connection.
    fn check_binding(&self, environment: QboEnvironment, realm: &str) -> Result<()> {
        ensure!(
            self.version == VERSION,
            "unsupported proposal format; prepare a new proposal"
        );
        ensure!(
            self.environment == environment,
            "proposal environment differs from the active connection; prepare a new proposal"
        );
        ensure!(
            self.realm == realm,
            "proposal company differs from the active connection; prepare a new proposal"
        );
        Ok(())
    }
    /// Write capability is sandbox only in this release, including sending.
    fn check_writable(&self, environment: QboEnvironment, realm: &str) -> Result<()> {
        ensure!(
            environment == QboEnvironment::Sandbox && self.environment == QboEnvironment::Sandbox,
            "production writes, including sending, are disabled in this release"
        );
        self.check_binding(environment, realm)
    }
    pub fn check_execution(
        &self,
        environment: QboEnvironment,
        realm: &str,
        journal: &Journal,
    ) -> Result<()> {
        self.check_writable(environment, realm)?;
        journal.require_approval(self)
    }

    /// Human-readable exact proposal, ending with its approval digest.
    pub fn display(&self) -> Result<String> {
        display::render(self, &self.digest()?)
    }
}

/// An approval names exactly what was reviewed; any difference invalidates it.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Approval {
    digest: String,
    version: u32,
    operation: String,
    kind: Kind,
    environment: QboEnvironment,
    realm: String,
}
impl Approval {
    fn of(p: &Proposal) -> Result<Self> {
        Ok(Self {
            digest: p.digest()?,
            version: p.version,
            operation: p.operation.clone(),
            kind: p.kind,
            environment: p.environment,
            realm: p.realm.clone(),
        })
    }
}

/// Journal state of one proposal's single permitted write.
pub enum Outcome {
    NotAttempted,
    /// A claim exists without a confirmed result: reconcile by reading, never retry.
    Unknown,
    Confirmed(Value),
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
        let request = if entity == "preferences" {
            self.request(reqwest::Method::GET, "query")?
                .query(&[("query", "SELECT * FROM Preferences")])
        } else {
            self.request(reqwest::Method::GET, &format!("{entity}/{key}"))?
        };
        let reply = self.transport.send(request.build()?).await?;
        if !reply.status.is_success() && !matches!(reply.status.as_u16(), 401 | 403) {
            bail!(
                "QuickBooks lookup rejected (HTTP {}); no document operation attempted",
                reply.status
            );
        }
        let envelope: Value = http::parse(reply)?;
        let value = if entity == "preferences" {
            let values = envelope["QueryResponse"][name]
                .as_array()
                .context("missing preferences query result")?;
            ensure!(values.len() == 1, "expected exactly one preferences result");
            values[0].clone()
        } else {
            envelope
                .get(name)
                .context("missing entity in response")?
                .clone()
        };
        if !matches!(entity, "preferences" | "companyinfo") {
            ensure!(text(&value, "Id")? == key, "lookup returned wrong ID");
        }
        Ok(value)
    }
    async fn check_doc_number(&self, kind: Kind, doc: &str) -> Result<()> {
        // The restricted alphabet is validated before building this query.
        let query = format!(
            "SELECT * FROM {} WHERE DocNumber = '{}' MAXRESULTS 1",
            kind.entity(),
            doc
        );
        let reply = self
            .transport
            .send(
                self.request(reqwest::Method::GET, "query")?
                    .query(&[("query", query)])
                    .build()?,
            )
            .await?;
        let envelope: Value = http::parse(reply)?;
        let result = envelope["QueryResponse"]
            .as_object()
            .context("missing query response")?;
        if let Some(matches) = result.get(kind.entity()) {
            ensure!(
                matches
                    .as_array()
                    .context("invalid duplicate query result")?
                    .is_empty(),
                "duplicate document number; prepare with a different reviewed number"
            );
        }
        Ok(())
    }
    async fn company(&self) -> Result<Value> {
        let company = self.lookup("companyinfo", self.realm).await?;
        let prefs = self.lookup("preferences", self.realm).await?;
        ensure!(
            company["Country"] == "CA",
            "only Canadian companies supported"
        );
        ensure!(
            prefs["CurrencyPrefs"]["HomeCurrency"]["value"] == "CAD",
            "only home-CAD transactions supported"
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
        if let Some(recipient) = &sale.recipient {
            email(recipient)?;
        }
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
        let prefs = &identity["preferences"]["SalesFormsPrefs"];
        if kind == Kind::Estimate {
            ensure!(prefs["AllowEstimates"] == true, "estimates must be enabled");
        }
        match prefs["CustomTxnNumbers"].as_bool() {
            Some(true) => {
                let doc = sale
                    .doc_number
                    .as_deref()
                    .context("explicit reviewed document number required")?;
                ensure!(
                    !doc.is_empty()
                        && doc.len() <= 21
                        && doc
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
                    "document number must be at most 21 ASCII letters, digits, hyphens or underscores"
                );
                self.check_doc_number(kind, doc).await?;
            }
            Some(false) => ensure!(
                sale.doc_number.is_none(),
                "custom document numbers disabled"
            ),
            None => bail!("document numbering preference missing"),
        }
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
        let mut bases: BTreeMap<String, TaxBase> = BTreeMap::new();
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
            let percent = cents(&tax["RateValue"].to_string())?;
            let effective_date = effective_rate_date(&tax, &sale.date, percent)?;
            ensure!(percent <= 10000, "unsupported tax percentage");
            let entry = bases.entry(rate_id.to_string()).or_insert(TaxBase {
                base: 0,
                percent,
                name: text(&tax, "Name")?.to_string(),
            });
            ensure!(
                entry.percent == percent,
                "tax rate changed during preparation"
            );
            entry.base = entry
                .base
                .checked_add(amount)
                .context("tax base overflow")?;
            evidence.push(json!({"description":line.description,"item":item,"tax_code":code,"tax_rate":tax,"selected_rate":money(percent),"effective_date":effective_date,"quantity":line.quantity,"rate":money(rate),"amount":money(amount)}));
            lines.push(json!({"Amount":number(amount)?,"Description":line.description,"DetailType":"SalesItemLineDetail","SalesItemLineDetail":{"ItemRef":{"value":line.item},"Qty":line.quantity,"UnitPrice":number(rate)?,"TaxCodeRef":{"value":line.tax_code}}}));
        }
        let mut estimated_tax = 0_i64;
        let mut taxes = Vec::new();
        for (rate_id, t) in &bases {
            // Percent is held in hundredths, so base * percent is in 1/10000 cents.
            let tax = t
                .base
                .checked_mul(t.percent)
                .context("tax overflow")?
                .checked_add(5000)
                .context("tax rounding overflow")?
                / 10000;
            estimated_tax = estimated_tax
                .checked_add(tax)
                .context("tax total overflow")?;
            taxes.push(json!({"tax_rate":rate_id,"name":t.name,"percent":money(t.percent),"base":money(t.base),"estimated_tax":money(tax)}));
        }
        let total = subtotal
            .checked_add(estimated_tax)
            .context("total overflow")?;
        ensure!(
            total <= 99_999_999_999,
            "total exceeds supported decimal range"
        );
        let mut payload = json!({"CustomerRef":{"value":sale.customer},"TxnDate":sale.date,"CurrencyRef":{"value":"CAD"},"GlobalTaxCalculation":"TaxExcluded","Line":lines,"CustomerMemo":{"value":sale.note},"EmailStatus":"NotSet"});
        if let Some(recipient) = &sale.recipient {
            payload["BillEmail"] = json!({"Address":recipient});
        }
        if let Some(doc) = &sale.doc_number {
            payload["DocNumber"] = json!(doc);
        }
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
            review: json!({"identity":identity,"customer":customer,"terms":terms,"lines":evidence,"taxes":taxes,"currency":"CAD","tax_basis":"amounts exclude tax (GlobalTaxCalculation TaxExcluded); estimate rounds half up per rate on the aggregate base","subtotal":money(subtotal),"estimated_tax":money(estimated_tax),"estimated_total":money(total),"recipient":sale.recipient,"date":sale.date,"due_date":sale.due_date,"note":sale.note}),
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
        if !document["ExchangeRate"].is_null() {
            ensure!(
                cents(&document["ExchangeRate"].to_string())? == 100,
                "exchange conversion unsupported"
            );
        }
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
    /// Resolves an unknown outcome by reading one named document. It never
    /// sends a POST; a mismatch or an unsent document leaves the outcome unknown.
    pub async fn reconcile(
        &self,
        proposal: &Proposal,
        journal: &Journal,
        document_id: &str,
    ) -> Result<Value> {
        proposal.check_binding(self.environment, self.realm)?;
        id(document_id)?;
        let digest = proposal.digest()?;
        match journal.outcome(&digest)? {
            Outcome::Confirmed(document) => return Ok(document),
            Outcome::NotAttempted => {
                bail!("no execution attempt is recorded for this proposal; nothing to reconcile")
            }
            Outcome::Unknown => {}
        }
        if proposal.operation == "send" {
            ensure!(
                text(&proposal.source, "id")? == document_id,
                "send reconciliation must read the proposal's own document"
            );
        }
        let document = self.lookup(proposal.kind.path(), document_id).await?;
        match proposal.operation.as_str() {
            "create" => ensure!(
                matches_payload(&proposal.payload, &document),
                "remote document does not match the approved payload; nothing recorded"
            ),
            "send" => {
                ensure!(
                    document["EmailStatus"] == "EmailSent",
                    "QuickBooks does not report this document as sent. The outcome stays unknown and this release will not resend it; handle sending manually in QuickBooks"
                );
            }
            _ => bail!("unsupported proposal operation"),
        }
        journal.complete(&digest, proposal, &document)?;
        Ok(document)
    }
}

/// True when every approved field and service line is present remotely with the
/// same value. QuickBooks adds IDs, names, totals and subtotal lines.
fn matches_payload(payload: &Value, document: &Value) -> bool {
    let Some(remote) = document["Line"].as_array() else {
        return false;
    };
    let supported = remote.iter().all(|l| {
        matches!(
            l["DetailType"].as_str(),
            Some("SalesItemLineDetail" | "SubTotalLineDetail")
        )
    });
    let services: Vec<Value> = remote
        .iter()
        .filter(|l| l["DetailType"] == "SalesItemLineDetail")
        .cloned()
        .collect();
    let mut header = payload.clone();
    let lines = header
        .as_object_mut()
        .and_then(|m| m.remove("Line"))
        .unwrap_or(Value::Null);
    supported && subset(&header, document) && subset(&lines, &Value::Array(services))
}
fn subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => e
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|av| subset(v, av))),
        (Value::Array(e), Value::Array(a)) => {
            e.len() == a.len() && e.iter().zip(a).all(|(e, a)| subset(e, a))
        }
        // Compare decimal text in cents, so 2 and 2.0 agree without float arithmetic.
        (Value::Number(e), Value::Number(a)) => {
            e == a
                || matches!(
                    (cents(&e.to_string()), cents(&a.to_string())),
                    (Ok(x), Ok(y)) if x == y
                )
        }
        _ => expected == actual,
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
    /// Records explicit approval of one saved proposal for the active connection.
    /// Approval never executes anything and performs no HTTP.
    pub fn approve(&self, digest: &str, environment: QboEnvironment, realm: &str) -> Result<()> {
        let p = self.load(digest)?;
        p.check_writable(environment, realm)?;
        let approval = Approval::of(&p)?;
        let path = self.path(digest, "approval")?;
        if path.try_exists()? {
            return self.require_approval(&p);
        }
        self.write_new(&path, &serde_json::to_vec_pretty(&approval)?)
    }
    fn require_approval(&self, p: &Proposal) -> Result<()> {
        let digest = p.digest()?;
        let bytes = fs::read(self.path(&digest, "approval")?).context(
            "explicit approval required: review, then run `qbo document approve DIGEST`",
        )?;
        let approval: Approval = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("approval record invalid; prepare a new proposal"))?;
        ensure!(
            approval == Approval::of(p)?,
            "approval does not match this proposal's digest, kind, environment and company"
        );
        Ok(())
    }
    fn claim(&self, digest: &str, p: &Proposal, request_id: &str) -> Result<()> {
        self.write_new(&self.path(digest,"attempt")?, &serde_json::to_vec(&json!({"proposal":p,"requestid":request_id,"outcome":"unknown; reconcile manually"}))?)
    }
    pub fn outcome(&self, digest: &str) -> Result<Outcome> {
        let path = self.path(digest, "result")?;
        if path.try_exists()? {
            return Ok(Outcome::Confirmed(serde_json::from_slice(&fs::read(
                path,
            )?)?));
        }
        if self.path(digest, "attempt")?.try_exists()? {
            return Ok(Outcome::Unknown);
        }
        Ok(Outcome::NotAttempted)
    }
    /// Confirmed result for replay, or an error when the outcome is unknown.
    pub fn result(&self, digest: &str) -> Result<Option<Value>> {
        match self.outcome(digest)? {
            Outcome::Confirmed(document) => Ok(Some(document)),
            Outcome::NotAttempted => Ok(None),
            Outcome::Unknown => bail!(
                "previous outcome unknown or in progress; do not retry. Find the document in QuickBooks, then run `qbo document reconcile DIGEST --id DOCUMENT_ID`"
            ),
        }
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
        // QuickBooks increments SyncToken on every change to a transaction.
        let current = &p.review["document"];
        ensure!(
            original["Id"] == current["Id"] && original["SyncToken"] == current["SyncToken"],
            "document changed since creation (SyncToken differs); remote edits unsupported in this release"
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
            "{}\n\nAttempt record: {}\nOutcome: {}\nResolve an unknown outcome with read-only `qbo document reconcile DIGEST --id DOCUMENT_ID`; nothing here retries a write.",
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
