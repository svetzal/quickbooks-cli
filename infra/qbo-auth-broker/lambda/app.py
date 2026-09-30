import base64
import hashlib
import hmac
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

import boto3

AUTHORIZATION_ENDPOINT = "https://appcenter.intuit.com/connect/oauth2"
TOKEN_ENDPOINT = "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer"
ACCOUNTING_SCOPE = "com.intuit.quickbooks.accounting"

table = boto3.resource("dynamodb").Table(os.environ["SESSION_TABLE"])
secrets = boto3.client("secretsmanager")


def handler(event, _context):
    method = event["requestContext"]["http"]["method"]
    path = event.get("rawPath", "/")

    if method == "POST" and path == "/sessions":
        return create_session()

    if method == "GET" and path == "/callback":
        return oauth_callback(event)

    if method == "GET" and path.startswith("/sessions/"):
        return poll_session(path, event)

    if method == "POST" and path == "/tokens/refresh":
        return refresh_token(event)

    return response(404, {"error": "not_found"})


def create_session():
    credentials = client_credentials()
    session_id = uuid.uuid4().hex
    state = f"{session_id}.{uuid.uuid4().hex}"
    poll_secret = uuid.uuid4().hex + uuid.uuid4().hex
    now = int(time.time())
    expires_at = now + int(os.environ.get("SESSION_TTL_SECONDS", "600"))

    table.put_item(
        Item={
            "session_id": session_id,
            "state": state,
            "poll_secret_sha256": sha256(poll_secret),
            "status": "pending",
            "created_at": now,
            "expires_at": expires_at,
        }
    )

    query = urllib.parse.urlencode(
        {
            "client_id": credentials["client_id"],
            "redirect_uri": os.environ["REDIRECT_URI"],
            "response_type": "code",
            "scope": ACCOUNTING_SCOPE,
            "state": state,
        }
    )

    return response(
        201,
        {
            "session_id": session_id,
            "poll_secret": poll_secret,
            "authorization_url": f"{AUTHORIZATION_ENDPOINT}?{query}",
            "expires_at": expires_at,
        },
    )


def oauth_callback(event):
    params = event.get("queryStringParameters") or {}
    state = params.get("state", "")
    session_id = state.split(".", 1)[0]

    if not session_id:
        return html_response(400, "Missing OAuth state.")

    item = get_session(session_id)
    if not item:
        return html_response(404, "OAuth session was not found or expired.")

    if expired(item):
        return html_response(410, "OAuth session expired. Start login again.")

    if not hmac.compare_digest(item.get("state", ""), state):
        return html_response(400, "OAuth state mismatch.")

    if "error" in params:
        mark_failed(session_id, params.get("error_description") or params["error"])
        return html_response(400, "QuickBooks authorization failed.")

    code = params.get("code")
    realm_id = params.get("realmId")
    if not code or not realm_id:
        mark_failed(session_id, "callback missing code or realmId")
        return html_response(400, "QuickBooks callback was missing required fields.")

    try:
        credentials = client_credentials()
        token = exchange_token(
            credentials,
            {
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": os.environ["REDIRECT_URI"],
            },
        )
    except Exception as error:
        mark_failed(session_id, str(error))
        return html_response(502, "Token exchange failed. Return to the CLI for details.")

    table.update_item(
        Key={"session_id": session_id},
        UpdateExpression=(
            "SET #status = :complete, realm_id = :realm_id, token = :token, "
            "client_id = :client_id, redirect_uri = :redirect_uri"
        ),
        ExpressionAttributeNames={"#status": "status"},
        ExpressionAttributeValues={
            ":complete": "complete",
            ":realm_id": realm_id,
            ":token": token,
            ":client_id": credentials["client_id"],
            ":redirect_uri": os.environ["REDIRECT_URI"],
        },
    )

    return html_response(200, "QuickBooks authorization complete. You can close this tab.")


def poll_session(path, event):
    session_id = path.rsplit("/", 1)[-1]
    params = event.get("queryStringParameters") or {}
    poll_secret = params.get("poll_secret", "")
    item = get_session(session_id)

    if not item:
        return response(404, {"error": "session_not_found"})

    if expired(item):
        table.delete_item(Key={"session_id": session_id})
        return response(410, {"error": "session_expired"})

    if not hmac.compare_digest(item.get("poll_secret_sha256", ""), sha256(poll_secret)):
        return response(403, {"error": "invalid_poll_secret"})

    if item["status"] == "pending":
        return response(202, {"status": "pending"})

    if item["status"] == "failed":
        table.delete_item(Key={"session_id": session_id})
        return response(400, {"error": "authorization_failed", "message": item.get("error", "")})

    token = item["token"]
    table.delete_item(Key={"session_id": session_id})
    return response(
        200,
        {
            "access_token": token["access_token"],
            "refresh_token": token["refresh_token"],
            "expires_in": int(token["expires_in"]),
            "realm_id": item["realm_id"],
            "client_id": item.get("client_id"),
            "redirect_uri": item.get("redirect_uri"),
            "environment": os.environ.get("ENVIRONMENT", "production"),
        },
    )


def refresh_token(event):
    body = json.loads(event.get("body") or "{}")
    refresh_token_value = body.get("refresh_token")
    if not refresh_token_value:
        return response(400, {"error": "missing_refresh_token"})

    try:
        token = exchange_token(
            client_credentials(),
            {"grant_type": "refresh_token", "refresh_token": refresh_token_value},
        )
    except Exception as error:
        return response(502, {"error": "refresh_failed", "message": str(error)})

    return response(
        200,
        {
            "access_token": token["access_token"],
            "refresh_token": token["refresh_token"],
            "expires_in": int(token["expires_in"]),
        },
    )


def get_session(session_id):
    result = table.get_item(Key={"session_id": session_id})
    return result.get("Item")


def expired(item):
    return int(item["expires_at"]) < int(time.time())


def mark_failed(session_id, message):
    table.update_item(
        Key={"session_id": session_id},
        UpdateExpression="SET #status = :failed, error = :error",
        ExpressionAttributeNames={"#status": "status"},
        ExpressionAttributeValues={":failed": "failed", ":error": message[:500]},
    )


def client_credentials():
    secret = secrets.get_secret_value(SecretId=os.environ["CLIENT_SECRET_ID"])["SecretString"]
    value = json.loads(secret)
    if value.get("client_id") == "replace-me" or value.get("client_secret") == "replace-me":
        raise RuntimeError("Intuit client credentials secret has not been configured")
    return value


def exchange_token(credentials, fields):
    body = urllib.parse.urlencode(fields).encode("utf-8")
    basic = base64.b64encode(
        f"{credentials['client_id']}:{credentials['client_secret']}".encode("utf-8")
    ).decode("ascii")
    request = urllib.request.Request(
        TOKEN_ENDPOINT,
        data=body,
        headers={
            "Authorization": f"Basic {basic}",
            "Accept": "application/json",
            "Content-Type": "application/x-www-form-urlencoded",
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=10) as reply:
            return json.loads(reply.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8")
        raise RuntimeError(f"Intuit token endpoint returned {error.code}: {detail}") from error


def sha256(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def response(status_code, payload):
    return {
        "statusCode": status_code,
        "headers": {"content-type": "application/json"},
        "body": json.dumps(payload),
    }


def html_response(status_code, message):
    body = (
        "<!doctype html><html><head><title>QBO Authorization</title></head>"
        f"<body><main><h1>QBO Authorization</h1><p>{escape_html(message)}</p></main></body></html>"
    )
    return {"statusCode": status_code, "headers": {"content-type": "text/html"}, "body": body}


def escape_html(value):
    return (
        value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&#x27;")
    )
