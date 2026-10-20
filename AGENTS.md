# QuickBooks CLI guidance

Read CHARTER.md first. Work directly on main. Do not create pull requests.
Fetch and rebase before changes. Stop on rebase conflicts.

Run every gate in .hone-gates.json before pushing. Add meaningful tests for
OAuth state, expiry, token handling and error behavior. Use in-process doubles
for unit tests. Live integration tests must use an Intuit sandbox company.

Never write credentials or tokens to plaintext files, command arguments,
logs, test artifacts or documentation. Use macOS Keychain or AWS Secrets
Manager. Do not change live credentials or deploy from autonomous tasks.

The broker runs in AWS ca-central-1. Public pages must describe the actual
private internal app. Do not invent controls, certifications or completed
tests for the Intuit questionnaire. Proposed privacy and license text needs
owner review before publication.

## Design intent: minimal input, QBO defaults, explicit overrides

This applies to every feature built onto QuickBooks Online. Preserve the
experience of giving QBO minimal input and having it fill in the details.
Our integration should make that experience available to the owner.

- Treat QBO as the source of truth for customers, products and services,
  descriptions, prices, currency, payment terms and tax configuration. Find
  and reuse existing records before proposing new ones.
- Start with the owner's intent, such as a customer, service and quantity.
  Resolve the rest from QBO's saved settings. Ask only for missing or
  ambiguous information. Do not require the owner to re-enter known details.
- Prefer QBO's native defaulting and calculation behavior. Where the API
  requires explicit fields, read the corresponding values from QBO and
  populate them. Do not assume the API inherits every default that the UI
  applies; verify the behavior in the sandbox.
- Let QBO handle tax calculations and its business rules. Do not build a
  parallel tax engine. If QBO cannot supply a result before creation, label
  it as pending and explain the limitation. Do not present our calculation
  as a QBO-confirmed amount or create a record just to obtain a preview
  without the owner's approval.
- Keep overrides explicit and exceptional. Show which values come from QBO,
  which the owner supplied, and which override saved defaults. A transaction
  override must not silently change a customer, item or company setting.
- Present the resolved document for owner review before creation. Sending
  requires separate approval of the final QBO document and its recipients.
  Review should make meaningful choices and exceptions clear, rather than
  make the owner confirm every inherited field separately.

(Stacey, 2026-10-19: design intent for all QBO integration work.)
