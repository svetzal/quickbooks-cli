# Review estimates and invoices

This release prepares Canadian service estimates and invoices using read-only
QuickBooks lookups. Creation and sending each need a separate explicit approval
of an exact proposal. Production accounting writes, including sending, are
disabled. There is no `--yes` shortcut. It does not create customers, items or
payments, and creation never requests automatic email.

The first owner-run tests must use **Sandbox Company CA fe63**. The owner must
review the exact proposal before any sandbox creation or sending. No such
approval or live verification is claimed by this implementation. Confirm the
full realm ID and returned company name; the display suffix `fe63` is not an API
realm ID. Do not switch the existing production connection or use credentials
for autonomous tests. Preparation can read production, but cannot write there.

## Find the existing records

Use the configured company's numeric IDs, not these illustrative placeholders:

```sh
qbo document lookup companyinfo REALM_ID
qbo document lookup preferences REALM_ID
qbo customer list --format json
qbo document lookup customer CUSTOMER_ID
qbo document lookup item ITEM_ID
qbo document lookup taxcode TAX_CODE_ID
qbo document lookup taxrate TAX_RATE_ID
qbo document lookup term TERM_ID
```

Preferences is a singleton; its lookup ID argument is ignored by the endpoint.
Customer, item, tax code, tax rate and term IDs must exist and be active. Choose
the tax code deliberately for the service and customer; the CLI does not decide
which tax jurisdiction applies or offer tax advice.

## Prepare and review creation

Write `service.json` with this constrained input (replace the fictitious IDs and
recipient). Money is decimal text, quantity is a positive whole number:

```json
{
  "customer": "10",
  "date": "2026-10-19",
  "due_date": null,
  "terms": null,
  "recipient": "owner@example.invalid",
  "note": "Service proposal",
  "lines": [
    {
      "item": "20",
      "description": "Consulting",
      "quantity": 2,
      "rate": "100.05",
      "tax_code": "30"
    }
  ]
}
```

```sh
qbo document plan estimate service.json
qbo document review DIGEST
```

`plan` performs GETs only and saves a versioned proposal. It prints a
human-readable proposal: environment, company name, realm, country and home
currency; customer name and ID; dates and terms; currency; each line's
description, item, quantity, unit rate and amount; each tax code and rate; the
tax basis with a per-rate estimate; estimated subtotal, tax and total; the
customer note; the bill email; the exact request and JSON body; and the approval
digest. For example (fictitious sandbox values, JSON body omitted):

```text
REVIEW ONLY: preparing or reviewing this proposal creates and sends nothing.
Operation:        create estimate (proposal format v1)
Environment:      sandbox
Company:          Example Sandbox CA (realm 123, country CA, home currency CAD)
Customer:         Review Customer (ID 10)
Date:             2026-10-19
Currency:         CAD
Bill email:       owner@example.invalid (stored on the document; creation sends no email)
Note to customer: Service proposal
Lines:
  1. Consulting
     Item Consulting (ID 20): 2 x 100.05 = 200.10 CAD
     Tax code GST (ID 30): GST 5% (rate ID 40) at 5%
Tax basis:        amounts exclude tax (GlobalTaxCalculation TaxExcluded); estimate rounds half up per rate on the aggregate base
  GST 5% (rate ID 40): 5.00% of 200.10 = 10.01 CAD
Subtotal:         200.10 CAD
Estimated tax:    10.01 CAD
Estimated total:  210.11 CAD
Request at execution: POST /v3/company/123/estimate?minorversion=75&requestid=<new UUID, journaled before sending>
```

The saved proposal also holds the full reference evidence (company information,
preferences, customer, item, tax code, tax rate and term records). That evidence
is part of the digest. For an
invoice use `plan invoice`, set an explicit `due_date` on or after `date`, and
optionally set an existing `terms` ID. Estimates do not accept due dates/terms.

After owner review of that exact digest, the operator can run these **separate**
commands. These examples are instructions, not recorded owner approval:

```sh
qbo document approve DIGEST
qbo document execute DIGEST
```

Approval does not execute anything and sends no HTTP request. It is refused
unless the active connection is sandbox and its realm is the proposal's realm.
The approval record names the SHA-256 digest, proposal format, operation, kind,
environment and realm. The digest binds proposal version, operation, kind,
environment, realm, full reference evidence and payload. Execution checks the
approval record against the proposal and the active connection before token
refresh or any HTTP, then re-reads references. Any changed
payload, company, preferences, references or environment needs a fresh proposal
and approval. Creation sets `EmailStatus: NotSet`, never `NeedToSend`. It does
not send, schedule email or create a send approval.

## Separately prepare and review sending

```sh
qbo document plan-send estimate DOCUMENT_ID --recipient owner@example.invalid
qbo document review SEND_DIGEST
```

Review the current remote document in full, including customer, lines, dates,
notes, currency, final QuickBooks totals, tax, document ID and `SyncToken`, plus
the explicit recipient and company identity. The send review prints the
recipient next to the document's bill email and marks them when they differ. It
also prints the full remote snapshot. Sending is restricted to unsent documents
that this release created (or reconciled) and recorded. The `SyncToken` must be
unchanged since then; QuickBooks increments it on every edit. Remote edits need
manual handling in QuickBooks in this release.

Only after the owner's separate send review:

```sh
qbo document approve SEND_DIGEST
qbo document execute SEND_DIGEST
```

The CLI re-reads the document, compares the whole snapshot and uses the separate
`POST /estimate/ID/send` or `POST /invoice/ID/send` endpoint with `sendTo`. The
endpoint does not provide an atomic conditional send using `SyncToken`; avoid
concurrent remote editing between preflight and sending. Successful creation
approval cannot authorize sending. Production sending remains disabled.

## Supported scope and recovery

Only Canadian, single-currency CAD companies with sales-tax setup are supported.
Each explicitly selected tax code must contain one percentage sales rate applied
to the amount with tax order zero. Zero percent is supported. Compound, multiple
rates per code, effective-dated/special taxes, tax-inclusive pricing, foreign
currency, projects, inventory, discounts, deposits and arbitrary API fields are
unsupported. Unknown fields in the input fail validation. Invalid dates, negative
rates and fractional quantities are rejected. The supported total is at most
CAD 999,999,999.99. Calculations use checked integer cents, never floating-point
arithmetic. Estimated tax rounds half up per aggregate rate; QuickBooks computes
final tax. Inspect the remote final totals during the separate send review.

A private `qbo-sales` directory under the platform local data directory holds
proposals, approvals, request IDs and outcomes, with directory mode 0700 and file
mode 0600. It contains customer/business data but no OAuth credentials or tokens.
Keep it private and durable. Do not delete it to retry an operation. This is a
single-machine journal: do not execute the same proposal from other machines or
independent journal copies.

An exclusive, synced execution claim is written before any POST. Repeating a
confirmed operation returns the recorded response without HTTP writes. Concurrent
claims permit only one POST. Every write carries a UUID `requestid`; no POST is
automatically retried, including on 429/5xx. A crash, timeout, transport failure,
invalid response or unconfirmed status leaves an unknown outcome. Review the
journal and reconcile. No automatic retry or resend can resolve that ambiguity.

To reconcile, find the document in QuickBooks by hand (for example by customer,
date and amount). Then record what you found:

```sh
qbo document review DIGEST
qbo document lookup estimate DOCUMENT_ID
qbo document reconcile DIGEST --id DOCUMENT_ID
```

`reconcile` only performs GET requests. It never sends a POST, and it never
creates or sends anything. For a creation it records the document only when
every approved field and service line matches the remote document, `EmailStatus`
is still `NotSet` and no other proposal recorded that document. A subsequent
`execute` then returns the recorded result, and a separate send review becomes
possible. For a send it reads the proposal's own document and records the
outcome only when QuickBooks reports `EmailSent`. If the document is not
reported as sent, the outcome stays unknown and this release will not resend it;
handle sending manually in QuickBooks. A mismatch records nothing. If no matching
document exists, the create claim remains and this release will not create the
same payload again; handle that case manually.
A document-level send claim also blocks another proposal from re-sending after an
ambiguous outcome. A separate create claim binds the accounting payload, realm,
environment and kind, so re-planning with changed reference metadata cannot
bypass an earlier identical creation attempt. Deliberately creating identical
documents requires manual handling outside this release. A disk error can leave a conservative claim even if no HTTP
was sent; that still requires reconciliation.

## Verification boundary

`contracts/qbo-sales-v1.json` is the executable consumer requirement set for
minorversion 75. In-process transport tests derive their responses and exact
request checks from it. They test creation/send endpoints and error behavior,
optional inputs, version rejection, failed preflight, scope mismatches, journal
replay/concurrency claims, ambiguous outcomes, approval binding to the active
sandbox company, read-only reconciliation and the readable proposal text. `cargo test --locked` runs them
in the required gates. No sockets, credentials, AWS calls or Intuit calls are
used. The existing broker-v2 contract and both its verifiers are unchanged.

**Intuit provider verification is missing.** Consumer fixtures are not proof that
Intuit implements this contract. The owner/parent must coordinate provider-side
verification of the same identifiable contract, record the provider version and
results, then run the reviewed first sandbox journey in Sandbox Company CA fe63.
No other repository is modified here. Old proposal versions are rejected; v1 is
the only supported local proposal format. No live creation or sending has been
verified. See `APPLICATION.md` for the current evidence and outstanding checks.

Primary references: [Canadian/non-US tax model](https://developer.intuit.com/app/developer/qbo/docs/develop/tutorials/manage-sales-tax-for-non-us-locales),
[TaxCode API](https://developer.intuit.com/app/developer/qbo/docs/api/accounting/all-entities/TaxCode),
[Invoice API](https://developer.intuit.com/app/developer/qbo/docs/api/accounting/all-entities/invoice),
[Estimate API](https://developer.intuit.com/app/developer/qbo/docs/api/accounting/all-entities/Estimate),
and [REST request IDs](https://developer.intuit.com/app/developer/qbo/docs/develop/rest-api-features).
