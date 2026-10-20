"""In-process provider verification and OAuth behavior tests; no AWS/network."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import types
import unittest
import urllib.error
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
CONTRACT = ROOT / "contracts/broker-v2.json"
SID = "a" * 32
STATE = SID + "." + "b" * 32
TOKEN = {"access_token": "test-access-placeholder", "refresh_token": "test-refresh-placeholder", "expires_in": 3600}


class ClientError(Exception):
    def __init__(self, response, operation="test"):
        self.response = response


class Table:
    def __init__(self):
        self.items = {}
        self.before_claim = None

    def put_item(self, Item, **kwargs):
        self.items[Item["session_id"]] = copy.deepcopy(Item)

    def get_item(self, Key, ConsistentRead):
        assert ConsistentRead
        item = self.items.get(Key["session_id"])
        return {"Item": copy.deepcopy(item)} if item else {}

    def update_item(self, Key, UpdateExpression, ExpressionAttributeValues, **kwargs):
        if self.before_claim:
            hook, self.before_claim = self.before_claim, None
            hook()
        item = self.items[Key["session_id"]]
        values = ExpressionAttributeValues
        expected = values.get(":expected", values.get(":exchanging"))
        assert kwargs["ConditionExpression"] == ("#status = :expected AND expires_at > :now" if ":expected" in values else "#status = :exchanging")
        if item["status"] != expected or (":now" in values and item["expires_at"] <= values[":now"]):
            raise ClientError({"Error": {"Code": "ConditionalCheckFailedException"}})
        for assignment in UpdateExpression.removeprefix("SET ").split(", "):
            key, value = assignment.split(" = ")
            item[kwargs["ExpressionAttributeNames"].get(key, key)] = copy.deepcopy(values[value])

    def delete_item(self, Key, **kwargs):
        assert self.items[Key["session_id"]]["status"] == kwargs["ExpressionAttributeValues"][":delivered"]
        del self.items[Key["session_id"]]


class Secrets:
    def get_secret_value(self, **kwargs):
        return {"SecretString": json.dumps({"client_id": "test-client", "client_secret": "test-client-placeholder"})}


def load_app():
    boto = types.ModuleType("boto3")
    boto.resource = lambda _: types.SimpleNamespace(Table=lambda _: Table())
    boto.client = lambda _: Secrets()
    core = types.ModuleType("botocore")
    errors = types.ModuleType("botocore.exceptions")
    errors.ClientError = ClientError
    with patch.dict(sys.modules, {"boto3": boto, "botocore": core, "botocore.exceptions": errors}), patch.dict(os.environ, {"SESSION_TABLE": "fake"}):
        spec = importlib.util.spec_from_file_location("broker_app", ROOT / "infra/qbo-auth-broker/lambda/app.py")
        app = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(app)
    return app


app = load_app()


class BrokerTests(unittest.TestCase):
    def setUp(self):
        app.table = Table()
        app.now = lambda: 1000
        self.env = patch.dict(os.environ, {"CLIENT_SECRET_ID": "test", "REDIRECT_URI": "https://broker.invalid/callback", "ENVIRONMENT": "sandbox", "SESSION_TTL_SECONDS": "600"})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.http = patch.object(app, "open_http", self.reply)
        self.http.start()
        self.addCleanup(self.http.stop)
        self.calls = []
        self.failure = False
        self.upstream_token = TOKEN

    def reply(self, request, timeout):
        self.calls.append(request)
        self.assertEqual(timeout, 10)
        if self.failure == "invalid_grant":
            # Intuit's documented rejection of an expired or revoked refresh token.
            body = b'{"error":"invalid_grant","error_description":"sensitive-upstream-placeholder"}'
            raise urllib.error.HTTPError(request.full_url, 400, "Bad Request", {}, io.BytesIO(body))
        if self.failure:
            raise RuntimeError("sensitive-upstream-placeholder")
        data = b"{}" if request.full_url == app.REVOKE_ENDPOINT else json.dumps(self.upstream_token).encode()
        class Reply:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, limit): return data[:limit]
        return Reply()

    def seed(self, status="pending"):
        if status == "missing": return
        app.table.items[SID] = dict(session_id=SID, state=STATE, poll_secret_sha256=app.sha256("test-poll-placeholder"),
                                   status="complete" if status == "legacy" else status, expires_at=1000 if status == "expired" else 1600,
                                   token=copy.deepcopy(TOKEN), realm_id="123")
        if status != "legacy":
            app.table.items[SID].update(client_id="test-client", redirect_uri="https://broker.invalid/callback")

    def event(self, method, path, **kwargs):
        return dict(requestContext={"http": {"method": method}}, rawPath=path, **kwargs)

    def callback(self, **params):
        return app.handler(self.event("GET", "/callback", queryStringParameters={"state": STATE, "code": "test-code-placeholder", "realmId": "123", **params}), None)

    def poll(self):
        return app.handler(self.event("GET", "/sessions/" + SID, headers={"Authorization": "Bearer test-poll-placeholder"}), None)

    def test_shared_provider_contracts(self):
        contract = json.loads(CONTRACT.read_text())
        self.assertEqual(contract["version"], "qbo-broker-v2")
        print("Provider contract sha256=" + hashlib.sha256(CONTRACT.read_bytes()).hexdigest())
        for case in contract["cases"]:
            with self.subTest(case=case["name"]):
                app.table = Table()
                self.seed(case.get("seed", "missing"))
                self.failure = case.get("upstream_failure", False)
                req = case["request"]
                event = self.event(req["method"], req["path"], headers=req.get("headers", {}),
                                   queryStringParameters=req.get("query", {}), body=json.dumps(req.get("body", {})))
                uuids = [types.SimpleNamespace(hex=SID)] + [types.SimpleNamespace(hex="b"*32)]*3
                with patch.object(app.uuid, "uuid4", side_effect=uuids):
                    result = app.handler(event, None)
                self.assertEqual(result["statusCode"], case["response"]["status"])
                self.assertEqual(json.loads(result["body"]), case["response"]["body"])
                self.assert_security(result)

    def assert_security(self, result):
        for key, value in app.HEADERS.items(): self.assertEqual(result["headers"][key], value)
        self.assertNotIn("sensitive-upstream-placeholder", result["body"])

    def test_success_exchange_delivery_and_replay(self):
        self.seed()
        result = self.callback()
        self.assertEqual(result["headers"]["location"], "/auth/success")
        self.assertEqual(result["statusCode"], 302)
        self.assertEqual(result["body"], "")
        self.assertEqual(self.callback()["headers"]["location"], "/auth/failure")
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(json.loads(self.poll()["body"])["access_token"], TOKEN["access_token"])
        self.assertEqual(self.poll()["statusCode"], 404)

    def test_state_expiry_denial_and_missing_fields(self):
        for params, exchange in [({"state": SID+"."+"c"*32}, False), ({"state": ""}, False), ({"error": "denied", "error_description": "sensitive-upstream-placeholder"}, False), ({"code": ""}, False)]:
            with self.subTest(params=params):
                self.seed()
                self.assertEqual(self.callback(**params)["headers"]["location"], "/auth/failure")
                self.assertFalse(self.calls)
        self.seed()
        app.now = lambda: 1600
        self.assertEqual(self.callback()["headers"]["location"], "/auth/failure")
        self.assertEqual(self.poll()["statusCode"], 410)

    def test_conditional_claims_stop_racing_callback_and_delivery(self):
        self.seed()
        app.table.before_claim = lambda: app.table.items[SID].update(status="exchanging")
        self.assertEqual(self.callback()["headers"]["location"], "/auth/failure")
        self.assertFalse(self.calls)
        self.seed("complete")
        app.table.before_claim = lambda: app.table.items[SID].update(status="delivered")
        self.assertEqual(self.poll()["statusCode"], 409)

    def test_exchange_failure_is_sanitized_and_not_retried(self):
        self.seed()
        self.failure = True
        result = self.callback()
        self.assertEqual(result["headers"]["location"], "/auth/failure")
        self.assertEqual(self.poll()["statusCode"], 400)
        self.callback()
        self.assertEqual(len(self.calls), 1)
        self.assertNotIn("sensitive", json.dumps(app.table.items))

    def test_refresh_and_revoke_wire_format_and_validation(self):
        for path in ("/tokens/refresh", "/tokens/revoke"):
            result = app.handler(self.event("POST", path, body=json.dumps({"refresh_token": TOKEN["refresh_token"]})), None)
            self.assertEqual(result["statusCode"], 200)
            request = self.calls[-1]
            self.assertTrue(request.get_header("Authorization").startswith("Basic "))
            if path.endswith("revoke"):
                self.assertEqual(request.full_url, app.REVOKE_ENDPOINT)
                self.assertEqual(json.loads(request.data), {"token": TOKEN["refresh_token"]})
            else:
                self.assertIn(b"grant_type=refresh_token", request.data)
        for token in ({}, {"access_token": "x", "refresh_token": "y", "expires_in": -1}, {**TOKEN, "expires_in": True}):
            self.upstream_token = token
            self.assertEqual(app.handler(self.event("POST", "/tokens/refresh", body=json.dumps({"refresh_token": "test"})), None)["statusCode"], 502)

    def test_rejected_refresh_token_requires_reconnect_without_leaking_upstream(self):
        self.failure = "invalid_grant"
        result = app.handler(self.event("POST", "/tokens/refresh", body=json.dumps({"refresh_token": "test"})), None)
        self.assertEqual(result["statusCode"], 401)
        self.assertEqual(json.loads(result["body"]), {"error": "reconnect_required"})
        self.assert_security(result)
        self.assertEqual(len(self.calls), 1)

    def test_other_upstream_http_errors_stay_generic(self):
        def rejected(request, timeout):
            self.calls.append(request)
            body = io.BytesIO(b'{"error":"invalid_client","detail":"sensitive-upstream-placeholder"}')
            raise urllib.error.HTTPError(request.full_url, 401, "Unauthorized", {}, body)
        with patch.object(app, "open_http", rejected):
            for path, error in (("/tokens/refresh", "refresh_failed"), ("/tokens/revoke", "revoke_failed")):
                result = app.handler(self.event("POST", path, body=json.dumps({"refresh_token": "test"})), None)
                self.assertEqual((result["statusCode"], json.loads(result["body"])), (502, {"error": error}))
                self.assert_security(result)
        # Broker client credentials failing is an operator problem, never a reason to reconnect blindly.
        self.assertEqual(len(self.calls), 2)

    def test_pages_and_malformed_input(self):
        for path in app.PAGES:
            result = app.handler(self.event("GET", path), None)
            self.assertEqual(result["statusCode"], 200)
            self.assert_security(result)
        for body in ("{", "[]", "null", "x" * (app.MAX_INPUT+1), '{"refresh_token":42}'):
            result = app.handler(self.event("POST", "/tokens/refresh", body=body), None)
            self.assertEqual(result["statusCode"], 400)
            self.assert_security(result)
        result = app.handler(self.event("GET", "/callback", rawQueryString="state=a&state=b"), None)
        self.assertEqual(result["headers"]["location"], "/auth/failure")
        self.assertEqual(app.handler(self.event("POST", "/tokens/revoke", body="!!!", isBase64Encoded=True), None)["statusCode"], 400)

    def test_oversized_or_malformed_callback_input_never_exchanges(self):
        # Oversized values are rejected before lookup; well-formed state with bad
        # fields consumes the session so the code can never be exchanged later.
        cases = [
            ({"code": "x" * 4097}, "pending"),
            ({"code": "line\nbreak"}, "failed"),
            ({"realmId": "not-a-realm"}, "failed"),
            ({"realmId": "1" * 33}, "failed"),
        ]
        for params, status in cases:
            with self.subTest(params=list(params)):
                self.seed()
                result = self.callback(**params)
                self.assertEqual((result["statusCode"], result["headers"]["location"], result["body"]), (302, "/auth/failure", ""))
                self.assert_security(result)
                self.assertEqual(app.table.items[SID]["status"], status)
        self.assertFalse(self.calls)
        result = app.handler(self.event("GET", "/callback", rawQueryString="state=" + "a" * (app.MAX_INPUT + 1)), None)
        self.assertEqual(result["headers"]["location"], "/auth/failure")

    def test_malformed_poll_requests_are_rejected_without_delivery(self):
        self.seed("complete")
        for path, headers, status in [
            ("/sessions/" + "A" * 32, {"Authorization": "Bearer test-poll-placeholder"}, 400),
            ("/sessions/../" + SID, {"Authorization": "Bearer test-poll-placeholder"}, 400),
            ("/sessions/" + SID, {}, 403),
            ("/sessions/" + SID, {"Authorization": "Basic test-poll-placeholder"}, 403),
            ("/sessions/" + SID, {"Authorization": "Bearer " + "x" * 4097}, 403),
            ("/" + "x" * 300, {}, 400),
        ]:
            with self.subTest(path=path[:40], headers=list(headers)):
                result = app.handler(self.event("GET", path, headers=headers), None)
                self.assertEqual(result["statusCode"], status)
                self.assert_security(result)
                self.assertNotIn(TOKEN["access_token"], result["body"])
        self.assertEqual(app.table.items[SID]["status"], "complete")
        self.assertEqual(self.poll()["statusCode"], 200)

    def test_result_pages_never_echo_authorization_parameters(self):
        self.seed()
        location = self.callback(code="test-code-placeholder")["headers"]["location"]
        self.assertNotIn("?", location)
        page = app.handler(self.event("GET", location), None)
        self.assertEqual(page["statusCode"], 200)
        self.assertNotIn("test-code-placeholder", page["body"])
        self.assertNotIn(STATE, page["body"])
        self.assertNotIn(TOKEN["access_token"], page["body"])


if __name__ == "__main__": unittest.main()
