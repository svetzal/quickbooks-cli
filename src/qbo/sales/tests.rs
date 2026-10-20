use super::*;
use reqwest::{Request, StatusCode};
use std::cell::{Cell, RefCell};

fn contract() -> Value {
    serde_json::from_str(include_str!("../../../contracts/qbo-sales-v1.json")).unwrap()
}
/// Every response and expected request is derived from the versioned consumer contract.
struct ContractTransport {
    fixture: Value,
    requests: RefCell<Vec<Request>>,
    fail_post: Cell<bool>,
    post_status: Cell<u16>,
    malformed_post: Cell<bool>,
    fail_read: Cell<bool>,
    hold_post: Cell<bool>,
}
impl ContractTransport {
    fn new() -> Self {
        Self {
            fixture: contract(),
            requests: RefCell::default(),
            fail_post: Cell::new(false),
            post_status: Cell::new(200),
            malformed_post: Cell::new(false),
            fail_read: Cell::new(false),
            hold_post: Cell::new(false),
        }
    }
    fn writes(&self) -> usize {
        self.requests
            .borrow()
            .iter()
            .filter(|r| r.method() == "POST")
            .count()
    }
    fn sales(&self) -> Sales<'_, Self> {
        Sales {
            transport: self,
            builder: http::client().unwrap(),
            environment: QboEnvironment::Sandbox,
            realm: "123",
            token: "in-process-placeholder",
        }
    }
    fn sale(&self, kind: Kind) -> Sale {
        let mut sale = self.fixture["sale"].clone();
        if kind == Kind::Invoice {
            sale["due_date"] = json!("2026-11-18");
            sale["terms"] = json!("50");
        }
        serde_json::from_value(sale).unwrap()
    }
}
impl Transport for ContractTransport {
    async fn send(&self, request: Request) -> Result<http::Reply> {
        assert_eq!(
            request.url().host_str(),
            Some("sandbox-quickbooks.api.intuit.com")
        );
        assert_eq!(request.headers()["Accept"], "application/json");
        let query: BTreeMap<_, _> = request
            .url()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(query["minorversion"], self.fixture["minorversion"]);
        let path = request
            .url()
            .path()
            .strip_prefix("/v3/company/123/")
            .unwrap()
            .to_string();
        let post = request.method() == "POST";
        let body = if !post {
            assert_eq!(request.method(), "GET");
            if path == "query" {
                assert_eq!(query.len(), 2);
                assert!(
                    query["query"] == self.fixture["preferences_query"]["query"]
                        || self.fixture["duplicate_queries"]
                            .get(&query["query"])
                            .is_some(),
                    "unexpected query"
                );
            } else {
                assert_eq!(query.len(), 1);
            }
            if path == "query" {
                if query["query"] == self.fixture["preferences_query"]["query"] {
                    self.fixture["preferences_query"]["response"].clone()
                } else {
                    self.fixture["duplicate_queries"][&query["query"]].clone()
                }
            } else {
                self.fixture["reads"][&path]
                    .as_object()
                    .map(|_| self.fixture["reads"][&path].clone())
                    .unwrap_or_else(|| {
                        let kind = path.split('/').next().unwrap();
                        assert_eq!(path, format!("{kind}/90"));
                        self.fixture["create"][kind]["response"].clone()
                    })
            }
        } else {
            let kind = path.split('/').next().unwrap();
            let expected = &self.fixture["create"][kind];
            assert!(uuid::Uuid::parse_str(&query["requestid"]).is_ok());
            if path.ends_with("/send") {
                assert_eq!(path, expected["send"]["path"]);
                assert_eq!(query["sendTo"], expected["send"]["recipient"]);
                assert_eq!(query.len(), 3);
                assert_eq!(
                    request.headers()["Content-Type"],
                    expected["send"]["content_type"].as_str().unwrap()
                );
                assert!(request.body().unwrap().as_bytes().unwrap().is_empty());
            } else {
                assert_eq!(path, expected["path"]);
                assert_eq!(query.len(), 2);
                assert_eq!(request.headers()["Content-Type"], "application/json");
                let body: Value =
                    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
                assert_eq!(body, expected["body"]);
                assert_eq!(body["EmailStatus"], "NotSet");
                assert!(!serde_json::to_string(&body).unwrap().contains("NeedToSend"));
            }
            if path.ends_with("/send") {
                expected["send"]["response"].clone()
            } else {
                expected["response"].clone()
            }
        };
        self.requests.borrow_mut().push(request);
        if post && self.hold_post.get() {
            std::future::poll_fn(|_| {
                if self.hold_post.get() {
                    std::task::Poll::Pending
                } else {
                    std::task::Poll::Ready(())
                }
            })
            .await;
        }
        if post && self.fail_post.get() {
            bail!("simulated ambiguous transport failure");
        }
        if !post && self.fail_read.get() {
            let error = &self.fixture["errors"][0];
            return Ok(http::Reply {
                status: StatusCode::from_u16(error["status"].as_u64().unwrap() as u16).unwrap(),
                body: serde_json::to_vec(&error["body"]).unwrap(),
            });
        }
        let body = if post && self.post_status.get() != 200 {
            self.fixture["errors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["status"] == self.post_status.get())
                .unwrap()["body"]
                .clone()
        } else {
            body
        };
        Ok(http::Reply {
            status: StatusCode::from_u16(if post { self.post_status.get() } else { 200 }).unwrap(),
            body: if post && self.malformed_post.get() {
                b"invalid".to_vec()
            } else {
                serde_json::to_vec(&body).unwrap()
            },
        })
    }
}
struct TestJournal {
    journal: Journal,
}
impl TestJournal {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("qbo-sales-test-{}", uuid::Uuid::new_v4()));
        Self {
            journal: Journal::open(&path).unwrap(),
        }
    }
    fn approved(&self, p: &Proposal) -> String {
        let digest = self.journal.save(p).unwrap();
        self.journal
            .approve(&digest, QboEnvironment::Sandbox, "123")
            .unwrap();
        digest
    }
}
impl Drop for TestJournal {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.journal.root).unwrap();
    }
}

#[tokio::test]
async fn contract_preparation_and_create_are_separate_for_both_kinds() {
    for kind in [Kind::Estimate, Kind::Invoice] {
        let fake = ContractTransport::new();
        let sales = fake.sales();
        let local = TestJournal::new();
        let p = sales.prepare(kind, fake.sale(kind)).await.unwrap();
        assert_eq!(fake.writes(), 0);
        assert_eq!(p.payload, fake.fixture["create"][kind.path()]["body"]);
        assert_eq!(p.review["subtotal"], "200.10");
        assert_eq!(p.review["estimated_tax"], "10.01");
        assert_eq!(p.review["estimated_total"], "210.11");
        assert!(sales.execute(&p, &local.journal).await.is_err());
        assert_eq!(fake.writes(), 0);
        let digest = local.approved(&p);
        let remote = sales.execute(&p, &local.journal).await.unwrap();
        assert_eq!(fake.writes(), 1);
        assert_eq!(sales.execute(&p, &local.journal).await.unwrap(), remote);
        assert_eq!(fake.writes(), 1);
        assert!(
            local
                .journal
                .inspect(&digest)
                .unwrap()
                .contains("requestid")
        );
        assert_eq!(local.journal.load(&digest).unwrap(), p);
    }
}
#[tokio::test]
async fn mismatched_binding_tampering_and_old_versions_never_write() {
    let fake = ContractTransport::new();
    let sales = fake.sales();
    let local = TestJournal::new();
    let p = sales
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    for field in [
        "realm",
        "environment",
        "version",
        "payload",
        "kind",
        "operation",
    ] {
        let mut v = serde_json::to_value(&p).unwrap();
        v[field] = match field {
            "realm" => json!("456"),
            "environment" => json!("production"),
            "version" => json!(0),
            "payload" => json!({}),
            "kind" => json!("invoice"),
            _ => json!("update"),
        };
        let changed: Proposal = serde_json::from_value(v).unwrap();
        assert!(sales.execute(&changed, &local.journal).await.is_err());
    }
    let mut production = fake.sales();
    production.environment = QboEnvironment::Production;
    assert!(production.execute(&p, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 0);
}
#[tokio::test]
async fn reference_drift_and_unsupported_cases_fail_preflight() {
    let mut fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.fixture["reads"]["customer/10"]["Customer"]["DisplayName"] = json!("Changed");
    assert!(fake.sales().execute(&p, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 0);
    for (path, key, value) in [
        ("companyinfo/123", "Country", json!("US")),
        ("customer/10", "Active", json!(false)),
        ("customer/10", "CurrencyRef", json!({"value":"USD"})),
        ("item/20", "Type", json!("Inventory")),
        (
            "taxcode/30",
            "SalesTaxRateList",
            json!({"TaxRateDetail":[]}),
        ),
        ("taxrate/40", "RateValue", json!("unknown")),
        ("taxrate/40", "EffectiveTaxRate", json!([])),
    ] {
        let mut fake = ContractTransport::new();
        let entity = fake.fixture["reads"][path]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        fake.fixture["reads"][path][entity][key] = value;
        assert!(
            fake.sales()
                .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
                .await
                .is_err()
        );
        assert_eq!(fake.writes(), 0);
    }
}
#[tokio::test]
async fn ambiguous_create_never_retries_and_errors_remain_unknown() {
    for mode in 0..7 {
        let fake = ContractTransport::new();
        let sales = fake.sales();
        let local = TestJournal::new();
        let p = sales
            .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
            .await
            .unwrap();
        let digest = local.approved(&p);
        match mode {
            0 => fake.fail_post.set(true),
            1 => fake.malformed_post.set(true),
            2 => fake.post_status.set(400),
            3 => fake.post_status.set(401),
            4 => fake.post_status.set(429),
            5 => fake.post_status.set(500),
            _ => {
                local
                    .journal
                    .claim(&digest, &p, "exclusive-concurrent-claim")
                    .unwrap();
            }
        }
        assert!(sales.execute(&p, &local.journal).await.is_err());
        assert!(sales.execute(&p, &local.journal).await.is_err());
        assert_eq!(fake.writes(), if mode == 6 { 0 } else { 1 });
        assert!(local.journal.inspect(&digest).unwrap().contains("unknown"));
    }
}
#[tokio::test]
async fn sending_requires_independent_review_and_never_repeats() {
    for kind in [Kind::Estimate, Kind::Invoice] {
        for fail in [false, true] {
            let fake = ContractTransport::new();
            let sales = fake.sales();
            let local = TestJournal::new();
            let p = sales.prepare(kind, fake.sale(kind)).await.unwrap();
            local.approved(&p);
            sales.execute(&p, &local.journal).await.unwrap();
            assert_eq!(fake.writes(), 1);
            let send = sales
                .prepare_send(kind, "90", "owner@example.invalid")
                .await
                .unwrap();
            assert_eq!(send.review["document"]["SyncToken"], "0");
            assert_ne!(send.digest().unwrap(), p.digest().unwrap());
            assert!(sales.execute(&send, &local.journal).await.is_err());
            assert_eq!(fake.writes(), 1);
            local.approved(&send);
            fake.fail_post.set(fail);
            let result = sales.execute(&send, &local.journal).await;
            assert_eq!(result.is_err(), fail);
            assert_eq!(fake.writes(), 2);
            assert_eq!(sales.execute(&send, &local.journal).await.is_err(), fail);
            assert_eq!(fake.writes(), 2);
            if fail {
                let changed = sales
                    .prepare_send(kind, "90", "different@example.invalid")
                    .await
                    .unwrap();
                local.approved(&changed);
                assert!(sales.execute(&changed, &local.journal).await.is_err());
                assert_eq!(fake.writes(), 2);
            }
        }
    }
}
#[tokio::test]
async fn changed_remote_document_or_recipient_invalidates_send_review() {
    let mut fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.sales().execute(&p, &local.journal).await.unwrap();
    let send = fake
        .sales()
        .prepare_send(Kind::Estimate, "90", "owner@example.invalid")
        .await
        .unwrap();
    local.approved(&send);
    fake.fixture["create"]["estimate"]["response"]["Estimate"]["SyncToken"] = json!("1");
    assert!(fake.sales().execute(&send, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 1);
    let changed = fake
        .sales()
        .prepare_send(Kind::Estimate, "90", "owner@example.invalid")
        .await
        .unwrap();
    local.approved(&changed);
    assert!(
        fake.sales()
            .execute(&changed, &local.journal)
            .await
            .is_err()
    );
    assert_eq!(fake.writes(), 1);
}
#[test]
fn decimals_dates_input_and_journal_permissions() {
    assert_eq!(cents("0.10").unwrap(), 10);
    assert_eq!(money(cents("123.45").unwrap()), "123.45");
    for n in ["-1", "NaN", "1.001", "1e2", "1.2.3", "9999999999"] {
        assert!(cents(n).is_err());
    }
    for d in [
        "2026-02-29",
        "2026-13-01",
        "2026-00-01",
        "2026-01-00",
        "ééééé",
    ] {
        assert!(date(d).is_err());
    }
    assert!(date("2024-02-29").is_ok());
    let mut input = contract()["sale"].clone();
    input["EmailStatus"] = json!("NeedToSend");
    assert!(serde_json::from_value::<Sale>(input).is_err());
    let journal = TestJournal::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&journal.journal.root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert!(journal.journal.load("../escape").is_err());
}
#[tokio::test]
async fn journal_persists_claims_across_reopening_and_canonical_keys() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let p = sales
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    let digest = local.approved(&p);
    local
        .journal
        .claim(&digest, &p, "persistent-request-id")
        .unwrap();
    let reopened = Journal::open(&local.journal.root).unwrap();
    assert!(
        sales
            .execute(&reopened.load(&digest).unwrap(), &reopened)
            .await
            .is_err()
    );
    assert_eq!(fake.writes(), 0);
    let bytes = serde_json::to_vec(&p).unwrap();
    let decoded: Proposal = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.digest().unwrap(), digest);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(local.journal.path(&digest, "attempt").unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn failed_read_preflight_never_claims_or_posts() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let p = sales
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    let digest = local.approved(&p);
    fake.fail_read.set(true);
    let error = sales
        .execute(&p, &local.journal)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("lookup rejected"));
    assert!(!error.contains("reconnect"));
    assert_eq!(fake.writes(), 0);
    assert!(!local.journal.path(&digest, "attempt").unwrap().exists());
}

#[tokio::test]
async fn optional_currency_and_terms_and_zero_tax_are_explicitly_supported() {
    let mut fake = ContractTransport::new();
    fake.fixture["reads"]["customer/10"]["Customer"]
        .as_object_mut()
        .unwrap()
        .remove("CurrencyRef");
    fake.fixture["reads"]["taxrate/40"]["TaxRate"]["RateValue"] = json!(0);
    let mut sale = fake.sale(Kind::Invoice);
    sale.terms = None;
    let p = fake.sales().prepare(Kind::Invoice, sale).await.unwrap();
    assert_eq!(p.review["estimated_tax"], "0.00");
    assert_eq!(p.review["estimated_total"], "200.10");
    assert!(p.payload.get("SalesTermRef").is_none());
    assert_eq!(fake.writes(), 0);
    let bad = fake
        .sales()
        .prepare_send(
            Kind::Invoice,
            "90",
            "one@example.invalid,two@example.invalid",
        )
        .await;
    assert!(bad.is_err());
}

#[tokio::test]
async fn compound_multiple_rates_and_foreign_preferences_are_rejected() {
    for mode in 0..4 {
        let mut fake = ContractTransport::new();
        match mode {
            0 => {
                fake.fixture["reads"]["taxcode/30"]["TaxCode"]["SalesTaxRateList"]["TaxRateDetail"]
                    [0]["TaxTypeApplicable"] = json!("TaxOnTax")
            }
            1 => {
                let rate = fake.fixture["reads"]["taxcode/30"]["TaxCode"]["SalesTaxRateList"]["TaxRateDetail"][0].clone();
                fake.fixture["reads"]["taxcode/30"]["TaxCode"]["SalesTaxRateList"]["TaxRateDetail"]
                    .as_array_mut()
                    .unwrap()
                    .push(rate);
            }
            2 => {
                fake.fixture["preferences_query"]["response"]["QueryResponse"]["Preferences"][0]["CurrencyPrefs"]
                    ["HomeCurrency"]["value"] = json!("USD")
            }
            _ => {
                fake.fixture["preferences_query"]["response"]["QueryResponse"]["Preferences"][0]["TaxPrefs"]
                    ["UsingSalesTax"] = json!(false)
            }
        }
        assert!(
            fake.sales()
                .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
                .await
                .is_err()
        );
        assert_eq!(fake.writes(), 0);
    }
}

#[tokio::test]
async fn altered_saved_proposal_and_unsupported_local_version_cannot_be_approved() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    let digest = local.journal.save(&p).unwrap();
    assert_eq!(local.journal.save(&p).unwrap(), digest);
    let mut changed = p.clone();
    changed.payload["EmailStatus"] = json!("NeedToSend");
    fs::write(
        local.journal.path(&digest, "proposal").unwrap(),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(
        local
            .journal
            .approve(&digest, QboEnvironment::Sandbox, "123")
            .is_err()
    );
    let mut old = p;
    old.version = 0;
    let old_digest = local.journal.save(&old).unwrap();
    assert!(
        local
            .journal
            .approve(&old_digest, QboEnvironment::Sandbox, "123")
            .is_err()
    );
    assert_eq!(fake.writes(), 0);
}

#[tokio::test]
async fn concurrent_execution_is_blocked_while_first_post_is_in_flight() {
    use std::{future::Future, task::Poll};
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let p = sales
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.hold_post.set(true);
    let mut first = std::pin::pin!(sales.execute(&p, &local.journal));
    // Drive the first operation exactly to the fake transport's explicit hold.
    // No clock, sleep or scheduler ordering determines this test's outcome.
    std::future::poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(fake.writes(), 1);
    assert!(sales.execute(&p, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 1);
    fake.hold_post.set(false);
    first.await.unwrap();
    assert!(sales.execute(&p, &local.journal).await.is_ok());
    assert_eq!(fake.writes(), 1);
}

#[tokio::test]
async fn new_reference_review_cannot_bypass_an_ambiguous_identical_payload() {
    let mut fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.fail_post.set(true);
    assert!(fake.sales().execute(&p, &local.journal).await.is_err());
    fake.fixture["reads"]["customer/10"]["Customer"]["DisplayName"] = json!("Changed display name");
    let revised = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    assert_eq!(revised.payload, p.payload);
    assert_ne!(revised.digest().unwrap(), p.digest().unwrap());
    local.approved(&revised);
    assert!(
        fake.sales()
            .execute(&revised, &local.journal)
            .await
            .is_err()
    );
    assert_eq!(fake.writes(), 1);
}

#[tokio::test]
async fn approval_is_bound_to_the_active_sandbox_company_and_exact_proposal() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let estimate = sales
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    let invoice = sales
        .prepare(Kind::Invoice, fake.sale(Kind::Invoice))
        .await
        .unwrap();
    let prepared_reads = fake.requests.borrow().len();
    let e = local.journal.save(&estimate).unwrap();
    let i = local.journal.save(&invoice).unwrap();
    for (environment, realm) in [
        (QboEnvironment::Production, "123"),
        (QboEnvironment::Sandbox, "456"),
    ] {
        assert!(local.journal.approve(&e, environment, realm).is_err());
    }
    assert!(!local.journal.path(&e, "approval").unwrap().exists());
    local
        .journal
        .approve(&e, QboEnvironment::Sandbox, "123")
        .unwrap();
    // Re-approving the same reviewed digest is harmless and idempotent.
    local
        .journal
        .approve(&e, QboEnvironment::Sandbox, "123")
        .unwrap();
    // An approval record cannot be transplanted onto a different proposal.
    fs::copy(
        local.journal.path(&e, "approval").unwrap(),
        local.journal.path(&i, "approval").unwrap(),
    )
    .unwrap();
    assert!(sales.execute(&invoice, &local.journal).await.is_err());
    // A legacy digest-only record is not an approval of kind, environment and company.
    fs::write(local.journal.path(&i, "approval").unwrap(), i.as_bytes()).unwrap();
    assert!(sales.execute(&invoice, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 0);
    let mut other_company = fake.sales();
    other_company.realm = "456";
    assert!(
        other_company
            .execute(&estimate, &local.journal)
            .await
            .is_err()
    );
    assert_eq!(
        fake.requests.borrow().len(),
        prepared_reads,
        "rejected approvals and executions send no request"
    );
    sales.execute(&estimate, &local.journal).await.unwrap();
    assert_eq!(fake.writes(), 1);
}

#[tokio::test]
async fn reconcile_records_a_matching_created_document_using_reads_only() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let p = sales
        .prepare(Kind::Invoice, fake.sale(Kind::Invoice))
        .await
        .unwrap();
    let digest = local.approved(&p);
    assert!(sales.reconcile(&p, &local.journal, "90").await.is_err());
    assert_eq!(fake.writes(), 0);
    fake.fail_post.set(true);
    assert!(sales.execute(&p, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 1);
    fake.fail_post.set(false);
    let before = fake.requests.borrow().len();
    let document = sales.reconcile(&p, &local.journal, "90").await.unwrap();
    assert_eq!(document["Id"], "90");
    assert_eq!(fake.writes(), 1);
    {
        let reads = &fake.requests.borrow()[before..];
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].method(), "GET");
        assert_eq!(reads[0].url().path(), "/v3/company/123/invoice/90");
    }
    // Confirmed replay and the separate send flow now work without another create.
    assert_eq!(sales.execute(&p, &local.journal).await.unwrap(), document);
    assert_eq!(
        sales.reconcile(&p, &local.journal, "90").await.unwrap(),
        document
    );
    let send = sales
        .prepare_send(Kind::Invoice, "90", "owner@example.invalid")
        .await
        .unwrap();
    local.approved(&send);
    sales.execute(&send, &local.journal).await.unwrap();
    assert_eq!(fake.writes(), 2);
    assert!(
        local
            .journal
            .inspect(&digest)
            .unwrap()
            .contains("confirmed")
    );
}

#[tokio::test]
async fn reconcile_rejects_mismatches_and_leaves_the_outcome_unknown() {
    for change in 0..4 {
        let mut fake = ContractTransport::new();
        let local = TestJournal::new();
        let p = fake
            .sales()
            .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
            .await
            .unwrap();
        let digest = local.approved(&p);
        fake.fail_post.set(true);
        assert!(fake.sales().execute(&p, &local.journal).await.is_err());
        fake.fail_post.set(false);
        let remote = &mut fake.fixture["create"]["estimate"]["response"]["Estimate"];
        match change {
            0 => remote["Line"][0]["Amount"] = json!(200.11),
            1 => remote["CustomerRef"]["value"] = json!("11"),
            2 => remote["EmailStatus"] = json!("NeedToSend"),
            _ => remote["Line"].as_array_mut().unwrap().push(
                json!({"Amount":1,"DetailType":"DiscountLineDetail","DiscountLineDetail":{}}),
            ),
        }
        let sales = fake.sales();
        assert!(sales.reconcile(&p, &local.journal, "90").await.is_err());
        assert!(matches!(
            local.journal.outcome(&digest).unwrap(),
            Outcome::Unknown
        ));
        assert!(sales.execute(&p, &local.journal).await.is_err());
        assert_eq!(fake.writes(), 1);
    }
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    let mut wrong = fake.sales();
    wrong.realm = "456";
    let before = fake.requests.borrow().len();
    assert!(wrong.reconcile(&p, &local.journal, "90").await.is_err());
    assert_eq!(fake.requests.borrow().len(), before);
}

#[tokio::test]
async fn ambiguous_send_reconciles_only_when_quickbooks_reports_it_sent() {
    let mut fake = ContractTransport::new();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.sales().execute(&p, &local.journal).await.unwrap();
    let send = fake
        .sales()
        .prepare_send(Kind::Estimate, "90", "owner@example.invalid")
        .await
        .unwrap();
    let digest = local.approved(&send);
    fake.fail_post.set(true);
    assert!(fake.sales().execute(&send, &local.journal).await.is_err());
    fake.fail_post.set(false);
    assert_eq!(fake.writes(), 2);
    // Still NotSet remotely: unknown stays unknown and nothing is resent.
    assert!(
        fake.sales()
            .reconcile(&send, &local.journal, "90")
            .await
            .is_err()
    );
    let before = fake.requests.borrow().len();
    assert!(
        fake.sales()
            .reconcile(&send, &local.journal, "91")
            .await
            .is_err()
    );
    assert_eq!(fake.requests.borrow().len(), before);
    assert!(fake.sales().execute(&send, &local.journal).await.is_err());
    assert_eq!(fake.writes(), 2);
    fake.fixture["create"]["estimate"]["response"] =
        fake.fixture["create"]["estimate"]["send"]["response"].clone();
    let sent = fake
        .sales()
        .reconcile(&send, &local.journal, "90")
        .await
        .unwrap();
    assert_eq!(sent["EmailStatus"], "EmailSent");
    assert_eq!(
        fake.sales().execute(&send, &local.journal).await.unwrap(),
        sent
    );
    assert_eq!(fake.writes(), 2);
    assert!(
        local
            .journal
            .inspect(&digest)
            .unwrap()
            .contains("EmailSent")
    );
}

#[tokio::test]
async fn proposals_are_human_readable_exact_and_name_their_identity() {
    let fake = ContractTransport::new();
    let local = TestJournal::new();
    let sales = fake.sales();
    let p = sales
        .prepare(Kind::Invoice, fake.sale(Kind::Invoice))
        .await
        .unwrap();
    let text = p.display().unwrap();
    for expected in [
        "REVIEW ONLY",
        "create invoice",
        "Environment:      sandbox",
        "Contract Sandbox CA (realm 123, country CA, home currency CAD)",
        "Review Customer (ID 10)",
        "Date:             2026-10-19",
        "Due date:         2026-11-18",
        "Net 30 (ID 50)",
        "Item Consulting (ID 20): 2 x 100.05 = 200.10 CAD",
        "Tax code GST (ID 30): GST 5% (rate ID 40) at 5.00%",
        "GST 5% (rate ID 40): 5.00% of 200.10 = 10.01 CAD",
        "Subtotal:         200.10 CAD",
        "Estimated tax:    10.01 CAD",
        "Estimated total:  210.11 CAD",
        "TaxExcluded",
        "owner@example.invalid (stored on the document; creation sends no email)",
        "Service proposal",
        "POST /v3/company/123/invoice?minorversion=75",
        "\"EmailStatus\": \"NotSet\"",
        &format!("qbo document approve {}", p.digest().unwrap()),
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(!text.contains("NeedToSend"));
    assert!(!text.contains("in-process-placeholder"));
    local.approved(&p);
    sales.execute(&p, &local.journal).await.unwrap();
    let send = sales
        .prepare_send(Kind::Invoice, "90", "other@example.invalid")
        .await
        .unwrap();
    let text = send.display().unwrap();
    for expected in [
        "send invoice 90",
        "Recipient:        other@example.invalid (sendTo",
        "owner@example.invalid (DIFFERS from recipient)",
        "SyncToken 0, EmailStatus NotSet",
        "QuickBooks tax:   10.01 CAD",
        "QuickBooks total: 210.11 CAD",
        "/invoice/90/send?minorversion=75&sendTo=other@example.invalid",
        "Content-Type application/octet-stream",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(!text.contains("in-process-placeholder"));
    assert_eq!(fake.writes(), 1);
}

#[test]
fn payload_matching_compares_decimal_text_and_ignores_quickbooks_additions() {
    let payload = contract()["create"]["invoice"]["body"].clone();
    let mut remote = contract()["create"]["invoice"]["response"]["Invoice"].clone();
    assert!(matches_payload(&payload, &remote));
    remote["Line"][0]["SalesItemLineDetail"]["Qty"] = serde_json::from_str("2.0").unwrap();
    remote["CustomerRef"]["name"] = json!("Review Customer");
    remote["Line"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Amount":200.1,"DetailType":"SubTotalLineDetail","SubTotalLineDetail":{}}));
    assert!(matches_payload(&payload, &remote));
    remote["Line"][0]["SalesItemLineDetail"]["UnitPrice"] = json!(100.06);
    assert!(!matches_payload(&payload, &remote));
}

fn sandbox_baseline() -> ContractTransport {
    let mut fake = ContractTransport::new();
    let baseline = fake.fixture["sandbox_baseline"].clone();
    for (key, value) in baseline.as_object().unwrap() {
        fake.fixture[key] = value.clone();
    }
    fake
}

#[tokio::test]
async fn actual_cad_sandbox_baseline_prepares_without_recipient_and_replays_safely() {
    let fake = sandbox_baseline();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    assert_eq!(fake.writes(), 0);
    assert_eq!(p.payload, fake.fixture["create"]["estimate"]["body"]);
    assert!(p.payload.get("BillEmail").is_none());
    assert_eq!(p.payload["CurrencyRef"]["value"], "CAD");
    assert_eq!(p.review["customer"]["Taxable"], false);
    assert_eq!(p.review["estimated_tax"], "13.00");
    assert_eq!(p.review["estimated_total"], "113.00");
    assert_eq!(p.review["lines"][0]["selected_rate"], "13.00");
    let display = p.display().unwrap();
    for expected in [
        "Benjamin Yeung",
        "TEST-EST-20261019-01",
        "1981-06-18T00:00:00-07:00",
        "113.00 CAD",
    ] {
        assert!(display.contains(expected), "missing {expected}");
    }
    assert!(fake.requests.borrow().iter().any(|r| {
        r.url()
            .query_pairs()
            .any(|(k, v)| k == "query" && v == "SELECT * FROM Preferences")
    }));
    assert!(
        fake.requests
            .borrow()
            .iter()
            .all(|r| !r.url().path().ends_with("/preferences/123"))
    );
    local.approved(&p);
    // All writes below are captured in-process; there is no network transport.
    let result = fake.sales().execute(&p, &local.journal).await.unwrap();
    assert_eq!(result["TotalAmt"], 113);
    assert_eq!(
        fake.sales().execute(&p, &local.journal).await.unwrap(),
        result
    );
    assert_eq!(fake.writes(), 1);
    let reads = fake.requests.borrow().len();
    assert!(
        fake.sales()
            .prepare_send(Kind::Estimate, "90", "")
            .await
            .is_err()
    );
    assert_eq!(fake.requests.borrow().len(), reads);
}

#[tokio::test]
async fn explicit_number_and_estimate_preferences_fail_closed() {
    for mode in 0..6 {
        let mut fake = sandbox_baseline();
        let mut sale = fake.sale(Kind::Estimate);
        match mode {
            0 => sale.doc_number = None,
            1 => sale.doc_number = Some("x' OR Id > '0".into()),
            2 => sale.doc_number = Some("x".repeat(22)),
            3 => {
                fake.fixture["preferences_query"]["response"]["QueryResponse"]["Preferences"][0]["SalesFormsPrefs"]
                    ["AllowEstimates"] = json!(false)
            }
            4 => {
                fake.fixture["preferences_query"]["response"]["QueryResponse"]["Preferences"][0]["SalesFormsPrefs"]
                    ["CustomTxnNumbers"] = json!(false)
            }
            _ => {
                fake.fixture["preferences_query"]["response"]["QueryResponse"]["Preferences"][0]["SalesFormsPrefs"]
                    .as_object_mut()
                    .unwrap()
                    .remove("CustomTxnNumbers");
            }
        }
        assert!(
            fake.sales().prepare(Kind::Estimate, sale).await.is_err(),
            "mode {mode}"
        );
        assert_eq!(fake.writes(), 0);
    }
}

#[tokio::test]
async fn same_kind_duplicate_number_is_checked_again_before_creation() {
    for kind in [Kind::Estimate, Kind::Invoice] {
        let mut fake = sandbox_baseline();
        let query = format!(
            "SELECT * FROM {} WHERE DocNumber = 'TEST-EST-20261019-01' MAXRESULTS 1",
            kind.entity()
        );
        fake.fixture["duplicate_queries"][&query] = json!({"QueryResponse":{}});
        let local = TestJournal::new();
        let p = fake.sales().prepare(kind, fake.sale(kind)).await.unwrap();
        local.approved(&p);
        fake.fixture["duplicate_queries"][&query] =
            json!({"QueryResponse":{kind.entity():[{"Id":"91"}]}});
        let error = fake.sales().execute(&p, &local.journal).await.unwrap_err();
        assert!(error.to_string().contains("duplicate document number"));
        assert!(fake.sales().prepare(kind, fake.sale(kind)).await.is_err());
        assert_eq!(fake.writes(), 0);
    }
}

#[tokio::test]
async fn unsupported_effective_schedules_are_rejected_and_evidence_drift_invalidates_review() {
    for schedule in [
        json!([]),
        json!([{"RateValue":13,"EffectiveDate":"2027-01-01T00:00:00-07:00"}]),
        json!([{"RateValue":12,"EffectiveDate":"1981-06-18T00:00:00-07:00"}]),
        json!([{"RateValue":13,"EffectiveDate":"1981-02-30T00:00:00-07:00"}]),
        json!([{"RateValue":13,"EffectiveDate":"1981-06-18"}]),
        json!([{"RateValue":13,"EffectiveDate":"1981-06-18T00:00:00-07:00"},{"RateValue":13,"EffectiveDate":"2020-01-01T00:00:00-07:00"}]),
    ] {
        let mut fake = sandbox_baseline();
        fake.fixture["reads"]["taxrate/12"]["TaxRate"]["EffectiveTaxRate"] = schedule;
        assert!(
            fake.sales()
                .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
                .await
                .is_err()
        );
        assert_eq!(fake.writes(), 0);
    }
    let mut fake = sandbox_baseline();
    let local = TestJournal::new();
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    local.approved(&p);
    fake.fixture["reads"]["taxrate/12"]["TaxRate"]["EffectiveTaxRate"][0]["EffectiveDate"] =
        json!("1981-06-19T00:00:00-07:00");
    assert!(
        fake.sales()
            .execute(&p, &local.journal)
            .await
            .unwrap_err()
            .to_string()
            .contains("preflight changed")
    );
    assert_eq!(fake.writes(), 0);
}

#[tokio::test]
async fn malformed_preferences_and_duplicate_queries_never_create() {
    for response in [
        json!({}),
        json!({"QueryResponse":{}}),
        json!({"QueryResponse":{"Preferences":[]}}),
        json!({"QueryResponse":{"Preferences":[{},{}]}}),
    ] {
        let mut fake = sandbox_baseline();
        fake.fixture["preferences_query"]["response"] = response;
        assert!(
            fake.sales()
                .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
                .await
                .is_err()
        );
        assert_eq!(fake.writes(), 0);
    }
    for response in [
        json!({}),
        json!({"Fault":{"Error":[]}}),
        json!({"QueryResponse":{"Estimate":null}}),
        json!({"QueryResponse":{"Estimate":{}}}),
    ] {
        let mut fake = sandbox_baseline();
        let query = fake.fixture["duplicate_queries"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        fake.fixture["duplicate_queries"][query] = response;
        assert!(
            fake.sales()
                .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
                .await
                .is_err()
        );
        assert_eq!(fake.writes(), 0);
    }
}

#[tokio::test]
async fn omitted_recipient_is_supported_and_foreign_conversion_is_rejected() {
    let mut fake = sandbox_baseline();
    fake.fixture["sale"]
        .as_object_mut()
        .unwrap()
        .remove("recipient");
    let p = fake
        .sales()
        .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
        .await
        .unwrap();
    assert!(p.payload.get("BillEmail").is_none());
    fake.fixture["reads"]["customer/3"]["Customer"]["CurrencyRef"]["value"] = json!("USD");
    assert!(
        fake.sales()
            .prepare(Kind::Estimate, fake.sale(Kind::Estimate))
            .await
            .is_err()
    );
    fake.fixture["create"]["estimate"]["response"]["Estimate"]["ExchangeRate"] = json!(1.25);
    assert!(
        fake.sales()
            .prepare_send(Kind::Estimate, "90", "owner@example.invalid")
            .await
            .unwrap_err()
            .to_string()
            .contains("exchange conversion unsupported")
    );
    assert_eq!(fake.writes(), 0);
}
