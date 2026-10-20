# QuickBooks CLI

Private bookkeeping integration for Mojility Inc. Source is MIT licensed.
Read [the charter](CHARTER.md) for scope. The accounting API supports read-only lookups and reviewed sandbox service
estimates/invoices. Production writes are disabled; no payment processing or AI
training is implemented.

## Install and use

```sh
cargo install --locked --path .
qbo auth login --broker-url https://qbo-auth.mojility.ca --environment sandbox
qbo customer list
qbo auth disconnect
```

The broker defaults to the Intuit sandbox; production use needs production
credentials and separate owner authorization. The owner completes Intuit
authorization. macOS uses Keychain; Linux needs
an unlocked Secret Service and the DBus development package to build.
See [installation and connection](book/src/connect.md) and
[operation, disconnection and migration](book/src/operations.md).
`qbo auth logout` clears local state without revoking Intuit authorization.
Direct login uses stdin or the platform secret store for client secrets;
`--client-secret` and `QBO_CLIENT_SECRET` are no longer supported.

## Estimates and invoices

`qbo document plan estimate service.json` prepares a read-only proposal.
`qbo document review DIGEST` displays it. After owner review, separate
`qbo document approve DIGEST` and `qbo document execute DIGEST` commands can
create it in sandbox. Sending requires its own `plan-send`, review, approval
and execution. No automatic email, production writes or `--yes` override exists.
After an unknown outcome, `qbo document reconcile DIGEST --id DOCUMENT_ID`
records what a read-only lookup confirms; nothing is retried.
See [the input format, examples, limits and recovery procedure](book/src/sales.md).
The first live test must use Sandbox Company CA fe63 after the owner reviews the
exact proposal. Live sandbox creation and sending remain PENDING OWNER REVIEW. Intuit is an
external vendor; our in-process tests verify consumer requirements.

## Verification

Run every command in [.hone-gates.json](.hone-gates.json). Install
`cargo-audit` and `pip-audit` to run the security gates. Tests use in-process
HTTP/AWS doubles and an injected clock/sleeper; they do not require network
listeners, AWS credentials or an Intuit company. Audits require registry
and advisory database access.

Both Rust consumer tests and Python provider tests verify the same
[versioned executable contract](contracts/broker-v2.json). CI runs every
gate, identifies its contract hash and tested commit, builds docs and checks
the runtime archive. [Application review](APPLICATION.md) distinguishes
verified local behavior from required owner and live sandbox checks.

The [broker README](infra/qbo-auth-broker/README.md) describes packaging and
owner-authorized deployment. The owner approved the published privacy
policy and internal-use terms on 2026-10-19.

## License

MIT. See [LICENSE](LICENSE).
