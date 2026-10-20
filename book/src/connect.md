# Install and connect

This is Mojility Inc.'s private internal bookkeeping integration. Customer
lookup is the only accounting command. It does not process payments, provide
financial advice or train AI models. QuickBooks remains the source of truth.

Build with Rust 2024 tooling (verification uses Rust 1.99.1). The project does
not yet declare an MSRV. On Linux, install `pkg-config` and `libdbus-1-dev`
first. macOS uses Keychain; Linux uses the configured Secret Service, which
must be available and unlocked. No plaintext token storage is supported.

```sh
cargo install --locked --path .
qbo auth status
qbo auth login --broker-url https://qbo-auth.mojility.ca --environment sandbox
```

The owner approves the company on Intuit's page. Use only a separately
configured sandbox broker and sandbox company for live tests. The broker's
configured environment determines the resulting connection; the CLI flag
is a fallback for older responses. The public broker pages in this change
are proposals awaiting owner review and deployment.

For direct OAuth, register a loopback redirect URI and pass the client ID.
Use `--client-secret-stdin` with a secure secret-store pipe, or omit it to
reuse the platform secret store. Never type the secret in arguments, shell
history, environment variables or a temporary file. For example, if the
secret is already in the macOS Keychain entry used by this CLI:

```sh
security find-generic-password -s mojility-qbo-cli -a client-secret -w |
  qbo auth login --client-id YOUR_PUBLIC_CLIENT_ID --client-secret-stdin
```

Direct mode requires the redirect port to match `--callback-bind` and the
path to be `/callback`. The default is `http://localhost:8734/callback`.
Direct callbacks and browser result requests have finite waits. Follow the
clean result redirect in the browser to finish the local handshake.
