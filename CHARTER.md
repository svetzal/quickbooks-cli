# QuickBooks CLI

Connect Mojility Inc.'s internal bookkeeping workflow to QuickBooks Online.
The CLI reads accounting records and will prepare estimates and invoices for
owner review. The OAuth broker keeps the Intuit client secret in AWS Secrets
Manager and transfers tokens to the CLI for storage in the platform secret
store. This tool does not process payments, provide financial advice, or train
AI models. It does not replace QuickBooks as the accounting source of truth.

Live accounting writes and sending documents need explicit operator intent.
Tests must use sandbox companies or in-process transport doubles.
