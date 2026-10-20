# Internal application review

This is Mojility Inc.'s private internal QuickBooks CLI, as scoped by
[CHARTER.md](CHARTER.md). The implemented accounting operation is customer
lookup. The broker handles connection, token refresh and revocation; the CLI
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

All six repository gates pass on main before deployment. The runtime package
includes the approved copy and stylesheet, with dependencies exported from
the maintained Python lock. Rust tests include rejected query syntax,
validation faults and malformed success responses without exposing payloads.

Shared executable contract: [qbo-broker-v2](contracts/broker-v2.json), SHA-256
`b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59`.
All 18 cases are verified by both sides:

| Side | Verification | Result |
| --- | --- | --- |
| Rust CLI consumer | `cargo test --locked`; contract-derived transport replies and actual request builders | Pass, 14 tests total |
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
the pinned runtime closure. Every gate command ran unchanged, and
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
then passed. The active CLI connection and deployed broker use production. The current release implements customer lookup and
connection management only; estimates and invoices remain planned.

MFA enforcement by Intuit, external scans and broader organizational security
controls have not been certified by this work. The questionnaire records
owner declarations separately from implementation evidence.
