# Changelog

## Unreleased

- Add proposed internal-use broker pages and clean callback result redirects.
  Privacy and application terms require owner review before publication.
- Protect polling with a bearer header and conditional exchange/delivery
  claims; reject legacy query polling. Upgrade the CLI and restart login.
- Add broker revocation and `qbo auth disconnect`; retain local auth on
  revocation failure. Local logout still does not revoke authorization.
- Replace direct secret arguments/environment input with stdin or the
  platform secret store. Bound HTTP calls and read retries; sanitize errors.
- Verify the shared v2 broker contract on both sides with in-process tests.
  Add CI gates, dependency audits and runtime-only deployment packaging.
- Update locked anyhow, quinn-proto, rustls and affected transitive packages
  to address dependency audit findings. Keep dependency upgrades separable
  from the OAuth correction when finalizing commits.

- Return HTTP 401 `reconnect_required` from broker refresh when Intuit
  rejects the refresh grant, so the CLI gives reconnect guidance instead
  of a generic retry message. Contract case added; both sides verify it.
- Make direct and broker `qbo auth disconnect` share one tested path that
  keeps local credentials unless revocation is confirmed.

No deployment, credential changes, Intuit form submission or live sandbox
verification is included.
