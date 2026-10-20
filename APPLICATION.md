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

Both applications are in this repository. Both were tested against base
commit `0ab47fb9050d3b4b87cec09d38db3fd5cfc1ed88` plus the uncommitted
correction in this Foundry worktree; neither has a new tested commit yet.
Foundry owns finalization. The verification source digest is
`faf3adc31ab38a1be6f04c648ff30d0f32a4726c1470309205486a69c8ed0e64`.
It hashes sorted relative paths, a NUL, each file's bytes and a NUL for
Cargo.toml, Cargo.lock, .hone-gates.json and source/contract/runtime/CI files
under `src`, `contracts`, `infra/qbo-auth-broker`, `.github/workflows` with
extensions rs, py, json, yaml, yml, sh or txt, excluding dist and bytecode
cache directories. This identifies the tested patch without inventing a
commit for uncommitted changes.

Shared executable contract: [qbo-broker-v2](contracts/broker-v2.json), SHA-256
`faea4409a48463f0cb80884384e84a21b1e1556685000979602dcb3375e74b93`.
All 17 cases are verified by both sides:

| Side | Verification | Result |
| --- | --- | --- |
| Rust CLI consumer | `cargo test --locked`; contract-derived transport replies and actual request builders | Pass, 9 tests total |
| Python broker provider | `python3 -m unittest discover -s infra/qbo-auth-broker/tests -v`; handler requests from the identical contract set | Pass, 7 tests total; also passes on Python 3.12 |

Cases cover session creation, pending/exchanging sessions, delivery,
optional older metadata, failed/expired/missing/consumed sessions, invalid
poll credentials, rejection of legacy v1 query polling, refresh/revoke
success and failure, and missing token input. Additional tests check
callback state mismatch, missing fields, denial, exact expiry, competing
claims, replay, sanitized errors, token response expiry validation, clean
redirects, page headers, bounded retries and polling, and credential
retention on failed revocation. Consumer tests also derive absent optional
fields and unknown new fields from the delivery contract.

HTTP, AWS and clock/sleeper doubles run in process. Tests do not open
sockets, contact Intuit/AWS, read real clock time or wait for real time.
Conditional claims permit at most one exchange/delivery; failures after
claiming can require reconnecting. The tests do not establish exactly-once
or durable token delivery.

All repository gates pass: formatting, Clippy with warnings denied, Rust
tests, broker tests, RustSec audit with warnings denied, and pip-audit of
the pinned runtime closure. Rust docs also build. Rust verification used
1.99.1; Rust 2024 and Tokio are preserved, as are dependency default-feature
settings. No MSRV is declared; declaring a supported MSRV is recommended
as a separate compatibility decision.

The deployment package bundles app.py and the audited pinned boto3 runtime
closure. It excludes tests and bytecode caches. Dependency updates to
Cargo.lock should be finalized separately from the functional correction.
No advisory suppression was added.

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
