//! Human-readable rendering of an exact proposal. Pure: no I/O, no arithmetic
//! on remote money; values are shown exactly as recorded in the proposal.
use super::{Kind, MINOR, Proposal};
use anyhow::{Result, bail};
use serde_json::Value;
use std::fmt::Write;

pub(super) fn render(p: &Proposal, digest: &str) -> Result<String> {
    let mut out = String::new();
    match p.operation.as_str() {
        "create" => create(&mut out, p)?,
        "send" => send(&mut out, p)?,
        _ => bail!("unsupported proposal operation"),
    }
    writeln!(out)?;
    writeln!(out, "Approval digest: {digest}")?;
    writeln!(
        out,
        "The digest binds the proposal format, operation, kind, environment, company, reference evidence and exact request."
    )?;
    write!(
        out,
        "Approve only after reviewing every value above, as a separate command:\n  qbo document approve {digest}"
    )?;
    Ok(out)
}

fn create(out: &mut String, p: &Proposal) -> Result<()> {
    let r = &p.review;
    let currency = show(&r["currency"]);
    writeln!(
        out,
        "REVIEW ONLY: preparing or reviewing this proposal creates and sends nothing."
    )?;
    writeln!(
        out,
        "Operation:        create {} (proposal format v{})",
        p.kind.path(),
        p.version
    )?;
    identity(out, p)?;
    writeln!(
        out,
        "Customer:         {} (ID {})",
        show(&r["customer"]["DisplayName"]),
        show(&r["customer"]["Id"])
    )?;
    writeln!(out, "Date:             {}", show(&r["date"]))?;
    if p.kind == Kind::Invoice {
        writeln!(out, "Due date:         {}", show(&r["due_date"]))?;
        let terms = &r["terms"];
        let terms = if terms.is_null() {
            "none".to_string()
        } else {
            format!("{} (ID {})", show(&terms["Name"]), show(&terms["Id"]))
        };
        writeln!(out, "Terms:            {terms}")?;
    }
    writeln!(out, "Currency:         {currency}")?;
    writeln!(
        out,
        "Bill email:       {} (stored on the document; creation sends no email)",
        show(&r["recipient"])
    )?;
    writeln!(out, "Note to customer: {}", show(&r["note"]))?;
    writeln!(out, "Lines:")?;
    for (n, line) in r["lines"].as_array().into_iter().flatten().enumerate() {
        writeln!(out, "  {}. {}", n + 1, show(&line["description"]))?;
        writeln!(
            out,
            "     Item {} (ID {}): {} x {} = {} {currency}",
            show(&line["item"]["Name"]),
            show(&line["item"]["Id"]),
            show(&line["quantity"]),
            show(&line["rate"]),
            show(&line["amount"]),
        )?;
        writeln!(
            out,
            "     Tax code {} (ID {}): {} (rate ID {}) at {}%",
            show(&line["tax_code"]["Name"]),
            show(&line["tax_code"]["Id"]),
            show(&line["tax_rate"]["Name"]),
            show(&line["tax_rate"]["Id"]),
            show(&line["tax_rate"]["RateValue"]),
        )?;
    }
    writeln!(out, "Tax basis:        {}", show(&r["tax_basis"]))?;
    for tax in r["taxes"].as_array().into_iter().flatten() {
        writeln!(
            out,
            "  {} (rate ID {}): {}% of {} = {} {currency}",
            show(&tax["name"]),
            show(&tax["tax_rate"]),
            show(&tax["percent"]),
            show(&tax["base"]),
            show(&tax["estimated_tax"]),
        )?;
    }
    writeln!(out, "Subtotal:         {} {currency}", show(&r["subtotal"]))?;
    writeln!(
        out,
        "Estimated tax:    {} {currency}",
        show(&r["estimated_tax"])
    )?;
    writeln!(
        out,
        "Estimated total:  {} {currency}",
        show(&r["estimated_total"])
    )?;
    writeln!(
        out,
        "QuickBooks calculates the final tax; a separate send review shows its final totals."
    )?;
    writeln!(
        out,
        "Request at execution: POST /v3/company/{}/{}?minorversion={MINOR}&requestid=<new UUID, journaled before sending>",
        p.realm,
        p.kind.path()
    )?;
    writeln!(out, "Exact JSON body:")?;
    writeln!(out, "{}", serde_json::to_string_pretty(&p.payload)?)?;
    Ok(())
}

fn send(out: &mut String, p: &Proposal) -> Result<()> {
    let r = &p.review;
    let d = &r["document"];
    let currency = show(&d["CurrencyRef"]["value"]);
    let recipient = show(&r["recipient"]);
    writeln!(
        out,
        "REVIEW ONLY: preparing or reviewing this proposal sends nothing."
    )?;
    writeln!(
        out,
        "Operation:        send {} {} (proposal format v{})",
        p.kind.path(),
        show(&p.source["id"]),
        p.version
    )?;
    identity(out, p)?;
    writeln!(
        out,
        "Recipient:        {recipient} (sendTo; QuickBooks emails this address)"
    )?;
    let bill = show(&d["BillEmail"]["Address"]);
    let note = if bill == recipient {
        ""
    } else {
        " (DIFFERS from recipient)"
    };
    writeln!(out, "Bill email:       {bill}{note}")?;
    writeln!(
        out,
        "Document:         ID {}, number {}, SyncToken {}, EmailStatus {}",
        show(&d["Id"]),
        show(&d["DocNumber"]),
        show(&d["SyncToken"]),
        show(&d["EmailStatus"]),
    )?;
    writeln!(out, "Customer:         {}", reference(&d["CustomerRef"]))?;
    writeln!(out, "Date:             {}", show(&d["TxnDate"]))?;
    match p.kind {
        Kind::Invoice => {
            writeln!(out, "Due date:         {}", show(&d["DueDate"]))?;
            writeln!(out, "Terms:            {}", reference(&d["SalesTermRef"]))?;
        }
        Kind::Estimate => writeln!(out, "Expiration date:  {}", show(&d["ExpirationDate"]))?,
    }
    writeln!(out, "Currency:         {currency}")?;
    writeln!(
        out,
        "Tax basis:        {}",
        show(&d["GlobalTaxCalculation"])
    )?;
    writeln!(
        out,
        "Note to customer: {}",
        show(&d["CustomerMemo"]["value"])
    )?;
    writeln!(out, "Lines:")?;
    let mut n = 0;
    for line in d["Line"].as_array().into_iter().flatten() {
        if line["DetailType"] == "SubTotalLineDetail" {
            writeln!(out, "     Subtotal: {} {currency}", show(&line["Amount"]))?;
            continue;
        }
        n += 1;
        let detail = &line["SalesItemLineDetail"];
        writeln!(out, "  {n}. {}", show(&line["Description"]))?;
        writeln!(
            out,
            "     Item {}: {} x {} = {} {currency}; tax code {}",
            reference(&detail["ItemRef"]),
            show(&detail["Qty"]),
            show(&detail["UnitPrice"]),
            show(&line["Amount"]),
            reference(&detail["TaxCodeRef"]),
        )?;
    }
    for tax in d["TxnTaxDetail"]["TaxLine"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let detail = &tax["TaxLineDetail"];
        writeln!(
            out,
            "  Tax {}: {}% of {} = {} {currency}",
            reference(&detail["TaxRateRef"]),
            show(&detail["TaxPercent"]),
            show(&detail["NetAmountTaxable"]),
            show(&tax["Amount"]),
        )?;
    }
    writeln!(
        out,
        "QuickBooks tax:   {} {currency}",
        show(&d["TxnTaxDetail"]["TotalTax"])
    )?;
    writeln!(out, "QuickBooks total: {} {currency}", show(&d["TotalAmt"]))?;
    writeln!(
        out,
        "Request at execution: POST /v3/company/{}/{}/{}/send?minorversion={MINOR}&sendTo={recipient}&requestid=<new UUID, journaled before sending>",
        p.realm,
        p.kind.path(),
        show(&p.source["id"]),
    )?;
    writeln!(out, "  Empty body, Content-Type application/octet-stream.")?;
    writeln!(
        out,
        "Full remote document snapshot (re-read and compared before sending):"
    )?;
    writeln!(out, "{}", serde_json::to_string_pretty(d)?)?;
    Ok(())
}

fn identity(out: &mut String, p: &Proposal) -> Result<()> {
    let company = &p.review["identity"]["company"];
    let prefs = &p.review["identity"]["preferences"];
    writeln!(out, "Environment:      {}", p.environment)?;
    writeln!(
        out,
        "Company:          {} (realm {}, country {}, home currency {})",
        show(&company["CompanyName"]),
        p.realm,
        show(&company["Country"]),
        show(&prefs["CurrencyPrefs"]["HomeCurrency"]["value"]),
    )?;
    Ok(())
}

/// Strings verbatim, numbers in their recorded decimal form, absent as "none".
fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "none".into(),
        other => other.to_string(),
    }
}

fn reference(v: &Value) -> String {
    match (&v["name"], &v["value"]) {
        (Value::Null, Value::Null) => "none".into(),
        (Value::Null, id) => format!("ID {}", show(id)),
        (name, id) => format!("{} (ID {})", show(name), show(id)),
    }
}
