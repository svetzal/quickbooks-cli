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
