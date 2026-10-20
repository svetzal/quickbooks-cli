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

No deployment, credential changes, Intuit form submission or live sandbox
verification is included.
