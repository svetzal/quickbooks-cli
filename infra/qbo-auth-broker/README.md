# QBO Auth Broker

Private OAuth broker for Mojility Inc.'s internal QuickBooks CLI. The AWS
stack targets ca-central-1: API Gateway, Python 3.12 Lambda, DynamoDB TTL
sessions, Secrets Manager, ACM and an existing Route 53 zone. The broker
handles OAuth code exchange, refresh and revocation. It does not call
accounting APIs, process payments or train AI models.

## Verify and package

uv is required for lock export and runtime packaging.

```sh
python3 -m venv .venv
. .venv/bin/activate
uv export --locked --project infra/qbo-auth-broker --no-dev --no-emit-project --no-hashes | python3 -m pip install pip-audit -r /dev/stdin
python3 -m unittest discover -s infra/qbo-auth-broker/tests -v
infra/qbo-auth-broker/audit.sh
infra/qbo-auth-broker/package.sh
```

`package.sh` stages only `lambda/app.py`, the approved pages and stylesheet,
and the runtime closure exported from `uv.lock`. It excludes source tests,
credentials and
bytecode caches. The bundled boto3/botocore versions are audited rather than
relying on an unspecified Lambda-supplied SDK. Foundry maintains the nested
Python project. Re-audit the lock when
upgrading; never suppress fixed advisories. The archive is written to
`infra/qbo-auth-broker/dist/qbo-auth-broker.zip`.

## Owner-operated deployment

Deployment and credential changes require explicit owner operation. The
privacy and terms proposals in `/privacy` and `/terms` need owner review
before publication, including retention, contact and legal commitments.
This implementation does not establish organizational policy or prove the
controls used in any Intuit questionnaire. Do not submit forms based only
on these tests.

Review `template.yaml` and configure non-secret `deploy.env` values for
`AWS_REGION=ca-central-1`, `STACK_NAME`, `AUTH_DOMAIN_NAME`,
`EXISTING_HOSTED_ZONE_ID`, `LAMBDA_CODE_S3_BUCKET` and `LAMBDA_CODE_S3_KEY`.
Optionally set `QBO_ENVIRONMENT` to `sandbox` (the default) or `production`.
The script stops if `AWS_REGION` is not `ca-central-1` or if
`QBO_ENVIRONMENT` has another value.
Then, only after review and explicit authorization, the owner can run:

```sh
infra/qbo-auth-broker/deploy.sh
```

The script packages, uploads and updates CloudFormation. It must not be run
as part of autonomous checks. Register the exact broker `/callback` HTTPS
URI with Intuit. Manage the client ID/client secret JSON only through AWS
Secrets Manager using an approved secure interface. Never pass secret JSON
in command arguments, environment files, logs or documentation. The template
creates a placeholder secret; the owner configures real credentials.

Keep API Gateway/Lambda access and diagnostic logs free of authorization
headers, query strings, request/response bodies and events. Callback queries
contain authorization codes even though browser responses redirect cleanly.
The code does not log events or upstream bodies; external logging settings
still require owner verification. Sessions temporarily hold tokens in
DynamoDB, which uses AWS encryption at rest; TTL deletion is asynchronous.
The CLI receives tokens once and stores them in the platform secret store.
Review deployed IAM, encryption, logging, retention and abuse controls before
live use; this change does not assert they were externally verified.

## Page content status

`lambda/app.py` serves the owner-approved copy in `lambda/pages.json`.
The owner approved publication, the support contact and provider no-training
settings on 2026-10-19. Further policy changes still require owner review.
The deployment uses a content-addressed S3 key so a code change updates Lambda.
The audit checks every pinned transitive runtime dependency exported from
`uv.lock`; it skips dependency resolution because the export is complete.

## Routes and migration

Public GET routes: `/`, `/privacy`, `/terms`, `/connect`, `/disconnect`,
`/auth/success`, `/auth/failure`. Callback GET `/callback` always returns a
302 to a clean result route. All application responses set no-store/no-cache,
no-referrer, nosniff and a restrictive CSP.

API routes: POST `/sessions`, GET `/sessions/{id}`, POST `/tokens/refresh`,
POST `/tokens/revoke`. Token operations accept `refresh_token` in a JSON
body. Polling requires `Authorization: Bearer <poll secret>`. Version 1
query-string polling is deliberately rejected: update the CLI and restart
login. There is no query-secret fallback. See
[the executable v2 contract](../../contracts/broker-v2.json), verified by
both sides in this repository's gates, and the
[migration guide](../../book/src/operations.md).

Conditional DynamoDB claims block duplicate code exchange and token
delivery. Expiry is checked independently of TTL cleanup. A claim followed
by process failure or a lost delivery response requires reconnecting; no
exactly-once or recoverable delivery guarantee is made. Upstream token and
revoke calls have a 10 second timeout and are not retried.

Refresh maps only Intuit's HTTP 400 `invalid_grant` to HTTP 401
`{"error": "reconnect_required"}`; the broker reads just the OAuth error
code from a bounded upstream body. Every other upstream failure, including
rejected broker client credentials, returns a generic 502 with no upstream
text.

## Remaining live checks

With a separately configured Intuit sandbox company, the owner still needs
to verify DNS/TLS, browser authorization, actual DynamoDB conditions/IAM,
Secrets Manager access, refresh rotation, revocation, CLI secret-store
storage, reconnection and customer lookup end to end. No live sandbox check,
external scan, MFA enforcement or provider no-training configuration has
been verified by this work. Intuit's documented
[OAuth revocation interface](https://developer.intuit.com/app/developer/qbo/docs/develop/authentication-and-authorization/oauth-2.0)
is the reference for the revoke request; local doubles do not prove Intuit
service behavior.
