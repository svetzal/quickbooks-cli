# QuickBooks CLI

Private bookkeeping integration for Mojility Inc. Source is MIT licensed.
Read [the charter](CHARTER.md) for scope. The accounting API is limited to
customer lookup; no payment processing or AI training is implemented.

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
owner-operated deployment. Public privacy and terms pages are drafts and
must receive owner review before publication. No deployment is part of
this change.

## License

MIT. See [LICENSE](LICENSE).
