# Operate and disconnect

```sh
qbo customer list
qbo customer list --name Adam --all --format json
qbo auth disconnect
```

Customer reads and broker polling retry HTTP 429, 500, 502, 503 and 504 at
most three times with delays of 1, 2 and 4 seconds. Other errors and
transport failures return immediately. Each HTTP call has a 15 second
limit and a 5 second connect limit. Polling is limited to 150 rounds.
Token exchange, refresh and revocation are never retried automatically:
a failure may mean the remote operation already happened. Reconnect with
`qbo auth login` when authorization is rejected, expired, or a one-time
token delivery response is lost. When Intuit rejects a refresh token as
expired or revoked (`invalid_grant`), the broker answers HTTP 401
`reconnect_required` and the CLI asks you to log in again. Other upstream
failures stay a generic HTTP 502 and are not a reason to reconnect.

`qbo auth disconnect` asks Intuit to revoke access before removing local
credentials and configuration. Broker and direct connections behave the
same way: local auth is removed only after revocation is confirmed. Any
failure or unconfirmed result preserves local auth so you can retry, or
disconnect through QuickBooks Connected Apps and then run `qbo auth logout`. If local
cleanup fails after remote success, resolve the secret-store error and use
`qbo auth logout` to finish local removal. `logout` alone only removes local
auth; it does not revoke the Intuit connection.

Client secrets and tokens are stored in the platform secret store. Config
contains connection metadata only. Do not redirect secret-store output to
files, enable HTTP body logging, or share authorization callback URLs.

## Broker protocol migration

The executable contract is `contracts/broker-v2.json`. Version 2 polling
uses `Authorization: Bearer <poll secret>` and rejects version 1's
`poll_secret` query parameter. Upgrade the CLI and start a fresh login;
existing pending sessions should be abandoned. There is no insecure query
fallback. Optional connection metadata in older responses remains readable.
The server identifies the protocol using `x-qbo-contract-version: 2`.

Callback and delivery claims are conditional and permit at most one code
exchange and one token delivery. Delivery is not recoverable after a lost
response; start login again. DynamoDB TTL deletion is asynchronous, so
expiry is also enforced by application checks. See the broker deployment
README for the owner-operated deployment and remaining sandbox checks.

For read-only accounting lookups and separately approved sandbox document
creation/sending, see [Review estimates and invoices](sales.md).
