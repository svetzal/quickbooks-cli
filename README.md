# QuickBooks CLI

Private bookkeeping integration for Mojility Inc. Source is MIT licensed.
Read [the charter](CHARTER.md) for its scope.

## Local checks

Run each command in `.hone-gates.json` before pushing or installing.

## Connect

Use `cargo run --locked -- auth status` to inspect local connection state.
Use `cargo run --locked -- auth login --broker-url https://qbo-auth.mojility.ca --environment production`
to request live access. The owner completes Intuit authorization.
Use `cargo run --locked -- customer list` for customer lookup.

The broker deployment procedure is in
[the broker README](infra/qbo-auth-broker/README.md).

## License

MIT. See [LICENSE](LICENSE).
