# Internal application review

This is Mojility Inc.'s private internal QuickBooks CLI, as scoped by
[CHARTER.md](CHARTER.md). The implemented accounting operation is customer
lookup. The broker handles connection, token refresh and revocation; the CLI
stores credentials in the platform secret store. This tool does not process
payments, provide financial advice or train AI models. These statements do
not establish Intuit's or another provider's data-use settings.

## Proposed public content

The broker implements `/`, `/privacy`, `/terms`, `/connect`, `/disconnect`
and clean authorization result pages. Privacy and application terms are
explicitly proposed internal-use drafts. Owner review is required before
publication, including retention, contacts, legal terms and organizational
commitments. Source licensing remains MIT. No Intuit form has been filled
or submitted as part of this correction.

## Verified local behavior

Both applications are in this repository. Both sides were tested at base
commit `6d8bb2dd97253ad57ac7b5b183f275fc347d2ff2` plus the uncommitted
follow-up correction in this Foundry worktree. Foundry assigns the final
commit; the follow-up has no tested commit identifier yet. CI records the
contract hash and `git rev-parse HEAD` for every run.

Shared executable contract: [qbo-broker-v2](contracts/broker-v2.json), SHA-256
`b485644af25d2d0bf0b6c330850bb222f6fb5da0f2647fb1299b2ea3412ebb59`.
All 18 cases are verified by both sides:

| Side | Verification | Result |
| --- | --- | --- |
| Rust CLI consumer | `cargo test --locked`; contract-derived transport replies and actual request builders | Pass, 13 tests total |
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

The deployment package bundles app.py and the audited pinned boto3 runtime
closure. It excludes tests and bytecode caches. Commit `1ffcc24` combined
the Cargo.lock dependency updates with the functional correction; this
follow-up changes no dependencies. No advisory suppression was added.

## Outstanding owner and live verification

No deployment or credential change has been performed. Live sandbox and
production behavior are unverified. Owner-operated sandbox checks remain
required for DNS/TLS, actual API Gateway routing and headers, Intuit browser
authorization, AWS conditional writes/IAM, Secrets Manager, refresh rotation,
revocation, secret-store integration, reconnection and customer lookup.
These external provider checks are outside local contract verification;
Intuit/AWS implementations are not modified or certified by this work.

MFA enforcement, external scans, provider no-training settings, corporate
and legal declarations, deployed logging, encryption settings, retention,
abuse protection and broader security controls have not been verified.
Do not represent them as completed or use this document to attest to them
in an Intuit questionnaire. See the
[deployment guide](infra/qbo-auth-broker/README.md) for review boundaries.
