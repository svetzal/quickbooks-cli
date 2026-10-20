# Internal application review

This is Mojility Inc.'s private internal QuickBooks CLI, as scoped by
[CHARTER.md](CHARTER.md). The implemented accounting operations include read-only lookups and
reviewed sandbox service estimates/invoices; production writes are disabled. The broker handles connection, token refresh and revocation; the CLI
stores credentials in the platform secret store. This tool does not process
payments, provide financial advice or train AI models. These statements do
not establish Intuit's or another provider's data-use settings.

## Public content and owner declarations

The owner approved publication of the privacy policy and internal-use terms
in `infra/qbo-auth-broker/lambda/pages.json` on 2026-10-19. Support is
<info@mojility.ca>. The owner confirmed both ChatGPT model training and Codex
environment training are disabled. Corporate-history declarations and the
agreement to comply with Intuit security requirements were confirmed by the
owner and saved in the Intuit questionnaire. Intuit accepted the submitted
questionnaire and reports Completed / Approved.
Production credentials unlocked on 2026-10-19 (2026-10-20 UTC).

## Verified local behavior

All six repository gates pass on this uncommitted working tree, based on
`2e7f4a15573eb3d3d5c22c4741a5ba28181c2c6d`. No refs were changed and no
commit, push or deployment was performed by this run. The runtime package
includes the approved copy and stylesheet, with dependencies exported from
the maintained Python lock. Rust tests include rejected query syntax,
validation faults and malformed success responses without exposing payloads.

Shared executable contract: [qbo-broker-v2](contracts/broker-v2.json), SHA-256
`b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59`.
All 18 cases are verified by both sides:

| Side | Verification | Result |
| --- | --- | --- |
| Rust CLI consumer | `cargo test --locked`; contract-derived transport replies and actual request builders | Pass, 41 Rust tests total (including sales tests) |
| Python broker provider | `python3 -m unittest discover -s infra/qbo-auth-broker/tests -v`; handler requests from the identical contract set | Pass, 13 tests total |

Cases cover session creation, pending/exchanging sessions, delivery,
optional older metadata, failed/expired/missing/consumed sessions, invalid
poll credentials, rejection of legacy v1 query polling, refresh/revoke
success and failure, a rejected refresh grant that requires reconnecting,
and missing token input. Additional tests check callback state mismatch,
missing fields, denial, exact expiry, competing claims, replay, sanitized
errors, token response expiry validation, clean redirects, page headers,
oversized or malformed callback, path and poll-credential input,
result pages that never echo authorization parameters,
bounded retries and polling, and credential retention on failed broker or
direct revocation. Consumer tests also derive absent optional fields and
unknown new fields from the delivery contract.

HTTP, AWS and clock/sleeper doubles run in process. Tests do not open
sockets, contact Intuit/AWS, read real clock time or wait for real time.
Conditional claims permit at most one exchange/delivery; failures after
claiming can require reconnecting. The tests do not establish exactly-once
or durable token delivery.

All repository gates pass: formatting, Clippy with warnings denied, Rust
tests, broker tests, RustSec audit with warnings denied, and pip-audit of
the pinned runtime closure. Every configured gate command ran, with the Python audit additionally checking
its generated requirements export. The explicitly requested requirements-file
pip-audit command also passed, and
`cargo doc --locked --no-deps` also passed. Broker tests ran locally on Python
3.15; CI uses Python 3.12, the Lambda runtime. Rust
verification used 1.99.1; Rust 2024 and Tokio are preserved, as are dependency default-feature
settings. No MSRV is declared; declaring a supported MSRV is recommended
as a separate compatibility decision.

The deployment package bundles the application, approved pages, stylesheet
and the runtime dependency closure exported from `uv.lock`. It excludes tests
and bytecode caches. No advisory suppression was added. Foundry maintains and
audits both the Rust project and the nested Python runtime project.

## Live verification

The Canadian AWS broker was deployed from main. HTTPS, public routes and
security headers passed live checks. Lambda is Python 3.12 on arm64.
DynamoDB encryption is enabled. API Gateway access logging is not configured;
the application does not log events, authorization headers or payloads.
The sandbox and production callback URI is
`https://qbo-auth.mojility.ca/callback`.

The owner-authorized sandbox company passed connection, customer lookup,
access-token refresh at command start, remote revocation, local credential
removal, reconnection and another customer lookup. The CLI stored its company
identifier and tokens in macOS Keychain. No accounting records were changed.
A live check exposed DynamoDB Decimal token-expiry serialization, which was
fixed before consuming the session; tests now use the actual DynamoDB type
serializer/deserializer for resource reads.

The owner-approved policy pages and support contact are published. Intuit app
details reached 100%, the questionnaire was submitted, and Intuit reports it
Approved. Production credentials are stored in AWS Secrets Manager.
The owner approved connecting Mojility Inc. through Intuit. Production
authorization, Keychain token storage, refresh and a read-only customer lookup
then passed. The active CLI connection and deployed broker use production. The previously live-verified release implements customer lookup and
connection management. The current source adds sandbox estimate/invoice
preparation, separately approved creation and separately approved sending; these
new document paths have no live verification yet.

MFA enforcement by Intuit, external scans and broader organizational security
controls have not been certified by this work. The questionnaire records
owner declarations separately from implementation evidence.


## Reviewed document operations — local implementation

The CLI now prepares narrow Canadian CAD service estimates/invoices through
read-only company, preferences, customer, item, tax code, tax rate, term and
document lookups. Exact versioned proposals bind reference evidence, payload,
environment, realm, kind and operation to a SHA-256 digest. Approval and execution
are separate commands; sending has its own current-remote-document review and
approval, including recipient, totals and SyncToken. Production accounting writes
and sending are disabled. Creation always requests EmailStatus NotSet. No payment,
customer or item creation was added.

A private durable journal claims each operation before a single POST, records its
UUID requestid and prevents replay after an unknown outcome. Document-level send
claims prevent an ambiguous send being repeated through another proposal.
Proposals print as human-readable reviews of company identity, customer, lines,
tax codes and rates, per-rate tax estimates, totals, dates, note, recipient and
the exact request. Approval records name the digest, format, operation, kind,
environment and realm, and are refused unless the active connection is that
sandbox company. A read-only `reconcile` command records an unknown outcome only
when a GET of the named document confirms it; it never retries a write. Money
uses integer cents and explicit constrained tax models; unsupported input fails
closed. See book/src/sales.md for operational limitations, single-machine journal
scope, reconciliation, and the non-atomic remote edit/send window.

The consumer contract is contracts/qbo-sales-v1.json (minorversion 75). Its
in-process tests run in the existing Rust test gate; mocks derive from the same
fixture. Intuit is an external vendor API, not an owned service: its provider-side
tests and deployment revision are unavailable, so paired owned-application
verification does not apply. These tests verify consumer requirements only.
Live creation/send integration is **PENDING OWNER REVIEW**.
The broker-v2 producer/consumer boundary
was not changed; both existing verifiers remain required gates.

No credentials, AWS APIs, live Intuit APIs, deployment, publication or Intuit form
updates are part of this work. No owner document approval is assumed. The parent
must install reviewed clean main and present the exact first estimate proposal
for **Sandbox Company CA fe63**. The owner must review it before creation, and
separately review any send proposal before sending. Neither live sandbox creation
nor sending has been tested. Prior production read-only connection verification
does not verify these new operations.

This correction changes no dependencies, lockfiles, features, runtime, edition
or toolchain pins.


The Python requirements.txt remains a deterministic runtime-only export of
uv.lock. audit.sh verifies it matches a fresh locked export before auditing.
No Python package versions changed.


## Read-only sandbox baseline for reviewed sales

The parent supplied read-only evidence for Sandbox Company CA fe63: CA, home
CAD, multicurrency enabled, sales tax enabled, estimates allowed and custom
transaction numbers enabled. This task accessed no credentials or live APIs.
Preferences use the verified SELECT * FROM Preferences query; the rejected
/preferences/realm route is not used. CompanyInfo.Id may differ from the realm;
the configured authenticated request context binds the company.

The first owner-review estimate uses active CAD customer Benjamin Yeung ID 3,
Taxable=false, without an email; Service item Hours ID 2; HST ON TaxCode 5
pointing at TaxRate 12, HST on sales, 13%, TaxOnAmount, SpecialTaxType NONE.
The single EffectiveTaxRate entry is 13%, effective
1981-06-18T00:00:00-07:00. On 2026-10-19, one hour at CAD 100.00 yields
estimated tax CAD 13.00 and total CAD 113.00. Document number is explicitly
TEST-EST-20261019-01; note: "Sandbox automation test only. No services will be
delivered." These TEST details grant no creation or sending permission.

Creation permits absent recipient and omits BillEmail. Sending requires a
separate explicit reviewed recipient. CAD transactions remain supported when
multicurrency is enabled; foreign currency and conversion remain rejected.
Custom numbering requires a reviewed explicit number, with GET duplicate checks
for the same document kind at preparation and execution preflight. Exact
payload, selected tax rate/date, and metadata remain bound to approval; drift
invalidates it. Production mutations remain disabled. The duplicate GET check
cannot prevent concurrent external edits; durable requestid/journal replay
protection remains in place. The parent performs the read-only proposal preview.

## Document contract verification record

Both local sides of broker-v2 above were verified on the working tree based on
`2e7f4a15573eb3d3d5c22c4741a5ba28181c2c6d`; the Python provider source is unchanged.
The Rust consumer's 41 tests and Python provider's 13 tests all pass. CI will
identify the finalized commit when Foundry lands this work; there is no new tested
commit ID to claim during this uncommitted run.

| Boundary | Executable contract SHA-256 | Consumer | Provider |
| --- | --- | --- | --- |
| CLI / broker, qbo-broker-v2 | `b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59` | Passed in Rust test gate | Passed in Python broker-test gate, same repository revision |
| CLI / Intuit, qbo-sales-v1 revision 2, minorversion 75 | `e9664b96bede9a45b2a6d522525c9edf496439e8c5d9489b90fd9b0c0543ea19` | Consumer requirement tests passed in Rust test gate | Not applicable to external vendor; deployment revision unavailable; no provider conformance claim |

Live Intuit mutation/send integration remains **PENDING OWNER REVIEW**. No writes
are authorized to obtain evidence; the parent must present the exact sandbox
proposal before the owner considers creation. The consumer fixture does not
certify vendor behavior.
The tested local behaviors include exact requests, failed reference preflight,
unsupported features, decimal totals, optional fields, old proposal version
rejection, independent send approval, stale SyncToken/content rejection, durable
claims, an explicitly held in-flight concurrent POST, confirmed replay, unknown
transport outcomes, and prevention of identical creation through changed review
metadata. A fresh approval cannot bypass an earlier identical accounting payload
claim. Approval records cannot be moved to another proposal or used for another
company. Reconciliation uses only GET requests, rejects mismatched or unsent
documents and leaves their outcome unknown. CLI parsing also rejects a `--yes` shortcut and unrestricted create payloads.
