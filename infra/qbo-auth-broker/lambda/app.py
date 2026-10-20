"""Private internal OAuth broker. Policies are drafts pending owner review."""
import base64
import hashlib
import hmac
import html
import json
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

import boto3
from botocore.exceptions import ClientError

AUTHORIZATION_ENDPOINT = "https://appcenter.intuit.com/connect/oauth2"
TOKEN_ENDPOINT = "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer"
REVOKE_ENDPOINT = "https://developer.api.intuit.com/v2/oauth2/tokens/revoke"
ACCOUNTING_SCOPE = "com.intuit.quickbooks.accounting"
CONTRACT_VERSION = "2"
MAX_INPUT = 16384
HEADERS = {
    "cache-control": "no-store, no-cache, max-age=0",
    "pragma": "no-cache",
    "referrer-policy": "no-referrer",
    "x-content-type-options": "nosniff",
    "content-security-policy": "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    "x-qbo-contract-version": CONTRACT_VERSION,
}
PAGES = {
    "/": ("QuickBooks CLI", "Private internal bookkeeping tool for Mojility Inc. Customer lookup is supported. No payment processing or AI training. Authorization is started by the CLI. Privacy and terms are proposed drafts pending owner review."),
    "/privacy": ("Proposed privacy policy — owner review required", "This private internal application uses QuickBooks company identifiers, customer records and OAuth tokens for Mojility bookkeeping. The CLI stores credentials in the platform secret store. The AWS broker in ca-central-1 temporarily stores authorization sessions and tokens in DynamoDB until collection or expiry; TTL cleanup is asynchronous. Intuit credentials are stored in AWS Secrets Manager. Disconnect revokes access; local logout alone does not. QuickBooks remains the accounting source of truth. This application does not process payments or use data to train AI. Retention, support contact, third-party terms and organizational commitments require owner review before publication."),
    "/terms": ("Proposed internal-use terms — owner review required", "For authorized Mojility Inc. operators only. Source code is MIT licensed; Intuit services remain subject to Intuit terms. This tool supports customer lookup and OAuth connection management. It provides no payment processing or financial advice. Operators must verify results against QuickBooks. These proposed application terms require owner legal review before publication."),
    "/connect": ("Connect QuickBooks", "Run qbo auth login --broker-url https://qbo-auth.mojility.ca --environment sandbox from your terminal, then approve the Intuit authorization page. The broker is configured for one Intuit environment, sandbox by default; production use needs production credentials and separate owner authorization. Only authorized internal operators should connect a company."),
    "/disconnect": ("Disconnect QuickBooks", "Run qbo auth disconnect to revoke the connection and remove local credentials. If revocation fails, credentials remain so you can retry. You can also disconnect through QuickBooks Connected Apps. Local qbo auth logout only removes local credentials."),
    "/auth/success": ("Authorization complete", "Return to the CLI to finish connecting. You may close this tab."),
    "/auth/failure": ("Authorization failed", "Return to the CLI and start login again. No authorization details are displayed here."),
}
table = boto3.resource("dynamodb").Table(os.environ["SESSION_TABLE"])
secrets = boto3.client("secretsmanager")
# Replaceable boundary clock; tests never read wall time.
now = lambda: int(time.time())


def handler(event, _context):
    path = event.get("rawPath", "/")
    try:
        method = event.get("requestContext", {}).get("http", {}).get("method", "")
        if not isinstance(event.get("body") or "", str) or len(event.get("body") or "") > MAX_INPUT:
            raise ValueError("oversized body")
        query = event.get("rawQueryString", "")
        if len(path) > 256 or len(query) > MAX_INPUT:
            raise ValueError("oversized request")
        params = event.get("queryStringParameters") or {}
        if not isinstance(params, dict) or any(not isinstance(v, str) or len(v) > 4096 for v in params.values()):
            raise ValueError("invalid query")
        # Reject duplicate callback parameters rather than accept ambiguous state.
        if query and any(len(v) != 1 for v in urllib.parse.parse_qs(query, keep_blank_values=True).values()):
            raise ValueError("duplicate query")
        if method == "GET" and path in PAGES:
            return page(*PAGES[path])
        if method == "GET" and path == "/callback":
            return oauth_callback(params)
        if method == "POST" and path == "/sessions":
            return create_session()
        if method == "GET" and path.startswith("/sessions/"):
            return poll_session(path, event)
        if method == "POST" and path in ("/tokens/refresh", "/tokens/revoke"):
            return token_operation(path, read_body(event))
        return response(404, {"error": "not_found"})
    except (ValueError, TypeError, KeyError, UnicodeError):
        return redirect(False) if path == "/callback" else response(400, {"error": "invalid_request"})
    except Exception:
        # AWS/HTTP exceptions can contain credentials or upstream response bodies.
        return redirect(False) if path == "/callback" else response(502, {"error": "service_unavailable"})


def read_body(event):
    body = event.get("body") or "{}"
    if not isinstance(body, str) or len(body) > MAX_INPUT:
        raise ValueError("invalid body")
    if event.get("isBase64Encoded"):
        body = base64.b64decode(body, validate=True).decode("utf-8")
    value = json.loads(body)
    if not isinstance(value, dict):
        raise ValueError("expected object")
    return value


def valid_text(value):
    return isinstance(value, str) and 0 < len(value) <= 4096 and not any(ord(c) < 32 for c in value)


def create_session():
    credentials = client_credentials()
    session_id = uuid.uuid4().hex
    state = f"{session_id}.{uuid.uuid4().hex}"
    poll_secret = uuid.uuid4().hex + uuid.uuid4().hex
    expires_at = now() + int(os.environ.get("SESSION_TTL_SECONDS", "600"))
    table.put_item(Item={"session_id": session_id, "state": state,
                         "poll_secret_sha256": sha256(poll_secret), "status": "pending",
                         "expires_at": expires_at}, ConditionExpression="attribute_not_exists(session_id)")
    query = urllib.parse.urlencode({"client_id": credentials["client_id"], "redirect_uri": os.environ["REDIRECT_URI"],
                                  "response_type": "code", "scope": ACCOUNTING_SCOPE, "state": state})
    return response(201, {"session_id": session_id, "poll_secret": poll_secret,
                          "authorization_url": f"{AUTHORIZATION_ENDPOINT}?{query}", "expires_at": expires_at})


def get_session(session_id):
    return table.get_item(Key={"session_id": session_id}, ConsistentRead=True).get("Item")


def claim(item, expected, target):
    try:
        table.update_item(
            Key={"session_id": item["session_id"]},
            UpdateExpression="SET #status = :target",
            ConditionExpression="#status = :expected AND expires_at > :now",
            ExpressionAttributeNames={"#status": "status"},
            ExpressionAttributeValues={":target": target, ":expected": expected, ":now": now()},
        )
        return True
    except ClientError as error:
        if error.response["Error"]["Code"] == "ConditionalCheckFailedException":
            return False
        raise


def finish(item, status, **fields):
    values = {":status": status, ":exchanging": "exchanging"}
    updates = ["#status = :status"]
    names = {"#status": "status"}
    for key, value in fields.items():
        values[f":{key}"] = value
        names[f"#{key}"] = key
        updates.append(f"#{key} = :{key}")
    table.update_item(Key={"session_id": item["session_id"]},
                      UpdateExpression="SET " + ", ".join(updates),
                      ConditionExpression="#status = :exchanging",
                      ExpressionAttributeNames=names, ExpressionAttributeValues=values)


def oauth_callback(params):
    state = params.get("state", "")
    if not re.fullmatch(r"[0-9a-f]{32}\.[0-9a-f]{32}", state):
        return redirect(False)
    item = get_session(state.split(".")[0])
    if not item or item["expires_at"] <= now() or not hmac.compare_digest(item["state"], state):
        return redirect(False)
    if not claim(item, "pending", "exchanging"):
        return redirect(False)
    if "error" in params or not valid_text(params.get("code")) or not re.fullmatch(r"[0-9]{1,32}", params.get("realmId", "")):
        finish(item, "failed")
        return redirect(False)
    try:
        credentials = client_credentials()
        token = exchange_token(credentials, {"grant_type": "authorization_code", "code": params["code"],
                                             "redirect_uri": os.environ["REDIRECT_URI"]})
        finish(item, "complete", token=token, realm_id=params["realmId"],
               client_id=credentials["client_id"], redirect_uri=os.environ["REDIRECT_URI"])
    except Exception:
        finish(item, "failed")
        return redirect(False)
    return redirect(True)


def poll_session(path, event):
    session_id = path.removeprefix("/sessions/")
    if not re.fullmatch(r"[0-9a-f]{32}", session_id):
        raise ValueError("invalid session")
    headers = {k.lower(): v for k, v in (event.get("headers") or {}).items()}
    authorization = headers.get("authorization", "")
    # v1 query credentials are deliberately rejected: URLs leak into access logs.
    if (event.get("queryStringParameters") or {}).get("poll_secret") is not None or not authorization.startswith("Bearer "):
        return response(403, {"error": "invalid_poll_secret"})
    poll_secret = authorization[7:]
    if not valid_text(poll_secret):
        return response(403, {"error": "invalid_poll_secret"})
    item = get_session(session_id)
    if not item:
        return response(404, {"error": "session_not_found"})
    if not hmac.compare_digest(item["poll_secret_sha256"], sha256(poll_secret)):
        return response(403, {"error": "invalid_poll_secret"})
    if item["expires_at"] <= now():
        return response(410, {"error": "session_expired"})
    if item["status"] in ("pending", "exchanging"):
        return response(202, {"status": "pending"})
    if item["status"] == "failed":
        return response(400, {"error": "authorization_failed"})
    if item["status"] != "complete" or not claim(item, "complete", "delivered"):
        return response(409, {"error": "session_consumed"})
    # Claim precedes delivery. A lost response requires reconnecting, never replay.
    table.delete_item(Key={"session_id": session_id}, ConditionExpression="#status = :delivered",
                      ExpressionAttributeNames={"#status": "status"},
                      ExpressionAttributeValues={":delivered": "delivered"})
    return response(200, {**item["token"], "realm_id": item["realm_id"], "client_id": item.get("client_id"),
                          "redirect_uri": item.get("redirect_uri"), "environment": os.environ.get("ENVIRONMENT", "production")})


class ReauthorizationRequired(Exception):
    """Intuit rejected the grant itself; only a new authorization can recover."""


def token_operation(path, body):
    token = body.get("refresh_token")
    if not valid_text(token):
        return response(400, {"error": "invalid_refresh_token"})
    try:
        credentials = client_credentials()
        if path == "/tokens/revoke":
            upstream(credentials, REVOKE_ENDPOINT, {"token": token}, json_body=True)
            return response(200, {"status": "revoked"})
        result = exchange_token(credentials, {"grant_type": "refresh_token", "refresh_token": token})
        return response(200, result)
    except ReauthorizationRequired:
        if path == "/tokens/refresh":
            return response(401, {"error": "reconnect_required"})
        return response(502, {"error": "revoke_failed"})
    except Exception:
        return response(502, {"error": "revoke_failed" if path.endswith("revoke") else "refresh_failed"})


def client_credentials():
    value = json.loads(secrets.get_secret_value(SecretId=os.environ["CLIENT_SECRET_ID"])["SecretString"])
    if not all(valid_text(value.get(key)) and value[key] != "replace-me" for key in ("client_id", "client_secret")):
        raise ValueError("credentials unavailable")
    return value


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("unexpected upstream redirect")


def open_http(request, timeout):
    """HTTP gateway; tests replace this boundary without opening sockets."""
    return urllib.request.build_opener(NoRedirect()).open(request, timeout=timeout)


def upstream(credentials, endpoint, fields, json_body=False):
    basic = base64.b64encode(f"{credentials['client_id']}:{credentials['client_secret']}".encode()).decode("ascii")
    body = (json.dumps(fields) if json_body else urllib.parse.urlencode(fields)).encode()
    request = urllib.request.Request(endpoint, data=body, method="POST", headers={
        "Authorization": f"Basic {basic}", "Accept": "application/json",
        "Content-Type": "application/json" if json_body else "application/x-www-form-urlencoded"})
    # No retries: exchange/rotation/revocation may already have taken effect.
    try:
        with open_http(request, timeout=10) as reply:
            data = reply.read(MAX_INPUT + 1)
    except urllib.error.HTTPError as error:
        with error:
            rejected = error.code == 400 and upstream_error_code(error.read(MAX_INPUT + 1)) == "invalid_grant"
        # Raise outside the handler so upstream text is never chained into tracebacks.
        if rejected:
            raise ReauthorizationRequired() from None
        raise RuntimeError("upstream request failed") from None
    if len(data) > MAX_INPUT:
        raise ValueError("oversized upstream response")
    return json.loads(data) if data else {}


def upstream_error_code(data):
    """Return only the OAuth error code; descriptions may echo request details."""
    try:
        value = json.loads(data) if len(data) <= MAX_INPUT else None
    except ValueError:
        return None
    return value.get("error") if isinstance(value, dict) else None


def exchange_token(credentials, fields):
    result = upstream(credentials, TOKEN_ENDPOINT, fields)
    if not isinstance(result, dict) or not all(valid_text(result.get(key)) for key in ("access_token", "refresh_token")):
        raise ValueError("invalid token response")
    expiry = result.get("expires_in")
    if isinstance(expiry, bool) or not isinstance(expiry, int) or expiry <= 0:
        raise ValueError("invalid token expiry")
    return {key: result[key] for key in ("access_token", "refresh_token", "expires_in")}


def sha256(value):
    return hashlib.sha256(value.encode()).hexdigest()


def response(status, payload):
    return {"statusCode": status, "headers": {**HEADERS, "content-type": "application/json"}, "body": json.dumps(payload)}


def page(title, message):
    body = f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{html.escape(title)}</title></head><body><main><h1>{html.escape(title)}</h1><p>{html.escape(message)}</p><nav><a href="/">Home</a> <a href="/privacy">Privacy draft</a> <a href="/terms">Terms draft</a> <a href="/connect">Connect</a> <a href="/disconnect">Disconnect</a></nav></main></body></html>'
    return {"statusCode": 200, "headers": {**HEADERS, "content-type": "text/html; charset=utf-8"}, "body": body}


def redirect(success):
    return {"statusCode": 302, "headers": {**HEADERS, "location": "/auth/success" if success else "/auth/failure"}, "body": ""}
