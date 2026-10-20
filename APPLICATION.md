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
`c316fa03182ce4d05c31bb0795d898e5b8aaa82b`. No refs were changed and no
commit, push or deployment was performed by this run. The runtime package
includes the approved copy and stylesheet, with dependencies exported from
the maintained Python lock. Rust tests include rejected query syntax,
validation faults and malformed success responses without exposing payloads.

Shared executable contract: [qbo-broker-v2](contracts/broker-v2.json), SHA-256
`b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59`.
All 18 cases are verified by both sides:

| Side | Verification | Result |
| --- | --- | --- |
| Rust CLI consumer | `cargo test --locked`; contract-derived transport replies and actual request builders | Pass, 29 Rust tests total (including sales tests) |
| Python broker provider | `python3 -m unittest discover -s infra/qbo-auth-broker/tests -v`; handler requests from the identical contract set | Pass, 12 tests total |

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
claims prevent an ambiguous send being repeated through another proposal. Money
uses integer cents and explicit constrained tax models; unsupported input fails
closed. See book/src/sales.md for operational limitations, single-machine journal
scope, reconciliation, and the non-atomic remote edit/send window.

The consumer contract is contracts/qbo-sales-v1.json (minorversion 75). Its
in-process tests run in the existing Rust test gate; mocks derive from the same
fixture. Intuit provider verification against this exact contract is outstanding
and cannot be replaced by these consumer tests. Provider commit/version and
verification results are unavailable. The broker-v2 producer/consumer boundary
was not changed; both existing verifiers remain required gates.

No credentials, AWS APIs, live Intuit APIs, deployment, publication or Intuit form
updates are part of this work. No owner document approval is assumed. The parent
must install reviewed clean main and present the exact first estimate proposal
for **Sandbox Company CA fe63**. The owner must review it before creation, and
separately review any send proposal before sending. Neither live sandbox creation
nor sending has been tested. Prior production read-only connection verification
does not verify these new operations.

The only Rust dependency declaration added is ring 0.17.14, already present in
Cargo.lock via the existing TLS stack, for SHA-256. No transitive packages or
existing features, runtime, edition or toolchain pins change. Foundry should keep
this dependency declaration/lockfile adjustment separate when finalizing changes.


The explicitly requested Python requirements audit needs requirements.txt, which
was absent. It is now a deterministic runtime-only export of uv.lock, not a
separate dependency source. audit.sh verifies it matches a fresh locked uv export
before auditing. No Python package versions changed.


## Document contract verification record

Both local sides of broker-v2 above were verified on the working tree based on
`c316fa03182ce4d05c31bb0795d898e5b8aaa82b`; the Python provider source is unchanged.
The Rust consumer's 29 tests and Python provider's 12 tests all pass. CI will
identify the finalized commit when Foundry lands this work; there is no new tested
commit ID to claim during this uncommitted run.

| Boundary | Executable contract SHA-256 | Consumer | Provider |
| --- | --- | --- | --- |
| CLI / broker, qbo-broker-v2 | `b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59` | Passed in Rust test gate | Passed in Python broker-test gate, same repository revision |
| CLI / Intuit, qbo-sales-v1, minorversion 75 | `3d6cd9099d75398fcb91ee64546652d5bea43d980b84094d9a52e2da334c187d` | Consumer requirement tests passed in Rust test gate | Missing; Intuit tested revision/version and conformance results unavailable |

The Intuit boundary remains unfinished verification work for coordination with
the parent/owner. The consumer fixture is not labeled a verified provider contract.
The tested local behaviors include exact requests, failed reference preflight,
unsupported features, decimal totals, optional fields, old proposal version
rejection, independent send approval, stale SyncToken/content rejection, durable
claims, an explicitly held in-flight concurrent POST, confirmed replay, unknown
transport outcomes, and prevention of identical creation through changed review
metadata. A fresh approval cannot bypass an earlier identical accounting payload
claim. CLI parsing also rejects a `--yes` shortcut and unrestricted create payloads.
