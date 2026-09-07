"""Web hardening: authentication, CSRF, session lifetime, headers, throttling.

Everything here is a security property, so each one is asserted rather than
assumed. The app is built per-test from an explicit WebSettings, never from
whatever happens to be in the ambient environment.
"""

from __future__ import annotations

import re

import pytest
from fastapi.testclient import TestClient

from web.app import create_app
from web.auth import AuthManager, LoginThrottle
from web.settings import WebSettings

PASSWORD = "correct horse battery staple"


def settings_for(tmp_path, **overrides) -> WebSettings:
    base = dict(
        username="admin", password_plain=PASSWORD,
        secret_key="unit-test-key", read_only=False, https_only=False,
        profile="demo", source="simulated", db_path=str(tmp_path / "web.db"),
        login_max_attempts=3, login_lockout_seconds=300, api_rate_per_minute=1000,
        allowed_hosts=["*"],
    )
    base.update(overrides)
    return WebSettings(**base)


def login(client: TestClient) -> str:
    """Sign in and return the dashboard CSRF token."""
    page = client.get("/login")
    token = re.search(r'name="csrf_token" value="([^"]+)"', page.text).group(1)
    response = client.post(
        "/login",
        data={"username": "admin", "password": PASSWORD, "csrf_token": token},
        follow_redirects=False,
    )
    assert response.status_code == 303, response.text
    return re.search(r'name="csrf-token" content="([^"]+)"', client.get("/").text).group(1)


@pytest.fixture
def client(tmp_path):
    with TestClient(create_app(settings_for(tmp_path)), base_url="http://testserver") as c:
        yield c


# --------------------------------------------------------------------------- #
# access control
# --------------------------------------------------------------------------- #
@pytest.mark.parametrize("path", ["/api/state", "/api/report", "/api/profiles"])
def test_api_requires_authentication(client, path):
    assert client.get(path).status_code == 401


def test_control_requires_authentication(client):
    assert client.post("/api/control", json={"action": "start"}).status_code == 401


def test_dashboard_redirects_anonymous_users(client):
    response = client.get("/", follow_redirects=False)
    assert response.status_code == 303
    assert response.headers["location"] == "/login"


def test_healthz_is_public_and_reveals_nothing(client):
    response = client.get("/healthz")
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}


def test_authenticated_api_returns_state(client):
    login(client)
    state = client.get("/api/state").json()
    assert state["profile"] == "demo"
    assert state["mode"] == "paper"


def test_state_never_carries_credentials(client):
    login(client)
    body = client.get("/api/state").text.lower()
    assert "password" not in body and "secret" not in body and PASSWORD not in body


# --------------------------------------------------------------------------- #
# headers
# --------------------------------------------------------------------------- #
def test_security_headers_are_present(client):
    headers = client.get("/login").headers
    csp = headers["content-security-policy"]
    assert "unsafe-inline" not in csp and "unsafe-eval" not in csp
    assert "frame-ancestors 'none'" in csp
    assert "default-src 'none'" in csp
    assert headers["x-frame-options"] == "DENY"
    assert headers["x-content-type-options"] == "nosniff"
    assert headers["referrer-policy"] == "no-referrer"
    assert headers["cache-control"] == "no-store"


def test_hsts_only_when_serving_tls(tmp_path):
    with TestClient(create_app(settings_for(tmp_path)), base_url="http://testserver") as plain:
        assert "strict-transport-security" not in plain.get("/login").headers
    app = create_app(settings_for(tmp_path, https_only=True))
    with TestClient(app, base_url="https://testserver") as tls:
        assert "strict-transport-security" in tls.get("/login").headers


def test_cookies_are_secure_under_tls(tmp_path):
    app = create_app(settings_for(tmp_path, https_only=True))
    with TestClient(app, base_url="https://testserver") as c:
        cookie = c.get("/login").headers["set-cookie"]
        assert "Secure" in cookie and "HttpOnly" in cookie and "strict" in cookie.lower()


# --------------------------------------------------------------------------- #
# CSRF
# --------------------------------------------------------------------------- #
def test_login_rejects_a_forged_csrf_token(client):
    client.get("/login")
    response = client.post(
        "/login", data={"username": "admin", "password": PASSWORD, "csrf_token": "forged"},
        follow_redirects=False,
    )
    assert response.status_code == 403


def test_login_without_the_csrf_cookie_is_explained(client):
    """A missing cookie is a deployment problem, so say so instead of '403'."""
    client.get("/login")
    client.cookies.clear()
    response = client.post(
        "/login", data={"username": "admin", "password": PASSWORD, "csrf_token": "x"},
        follow_redirects=False,
    )
    assert response.status_code == 403
    assert "reload the page" in response.text.lower()


def test_control_rejects_a_forged_csrf_token(client):
    login(client)
    assert client.post("/api/control", json={"action": "start", "csrf_token": "forged"}).status_code == 403


def test_control_accepts_the_header_form(client):
    csrf = login(client)
    response = client.post("/api/control", json={"action": "stop"}, headers={"X-CSRF-Token": csrf})
    assert response.status_code == 200


def test_logout_requires_csrf(client):
    login(client)
    assert client.post("/logout", data={"csrf_token": "forged"}, follow_redirects=False).status_code == 403


# --------------------------------------------------------------------------- #
# credentials and sessions
# --------------------------------------------------------------------------- #
def test_wrong_password_is_rejected_without_enumeration(client):
    page = client.get("/login")
    token = re.search(r'name="csrf_token" value="([^"]+)"', page.text).group(1)
    bad_password = client.post("/login", data={"username": "admin", "password": "no",
                                               "csrf_token": token}, follow_redirects=False)
    bad_user = client.post("/login", data={"username": "ghost", "password": "no",
                                           "csrf_token": token}, follow_redirects=False)
    assert bad_password.status_code == bad_user.status_code == 401
    assert "Invalid credentials." in bad_password.text
    assert "Invalid credentials." in bad_user.text


def test_lockout_after_repeated_failures(client):
    page = client.get("/login")
    token = re.search(r'name="csrf_token" value="([^"]+)"', page.text).group(1)
    for _ in range(3):
        client.post("/login", data={"username": "admin", "password": "no", "csrf_token": token},
                    follow_redirects=False)
    blocked = client.post("/login", data={"username": "admin", "password": PASSWORD,
                                          "csrf_token": token}, follow_redirects=False)
    assert blocked.status_code == 429


def test_logout_revokes_the_session_server_side(client):
    csrf = login(client)
    cookie = client.cookies.get("btc5m_session")
    client.post("/logout", data={"csrf_token": csrf}, follow_redirects=False)
    client.cookies.set("btc5m_session", cookie)     # replay the stolen cookie
    assert client.get("/api/state").status_code == 401


def test_a_forged_session_cookie_is_rejected(client):
    client.cookies.set("btc5m_session", "not-a-signed-value")
    assert client.get("/api/state").status_code == 401


def test_session_expires_when_idle():
    auth = AuthManager(WebSettings(secret_key="k", password_plain=PASSWORD,
                                   session_idle_minutes=1, session_max_hours=8))
    cookie, _ = auth.create_session("admin", "1.2.3.4", now=1000.0)
    assert auth.resolve(cookie, now=1030.0) is not None
    assert auth.resolve(cookie, now=1000.0 + 61 + 30) is None


def test_session_expires_at_the_absolute_lifetime():
    auth = AuthManager(WebSettings(secret_key="k", password_plain=PASSWORD,
                                   session_idle_minutes=600, session_max_hours=1))
    cookie, _ = auth.create_session("admin", "1.2.3.4", now=1000.0)
    assert auth.resolve(cookie, now=1000.0 + 3599) is not None
    assert auth.resolve(cookie, now=1000.0 + 3601) is None


def test_purge_expired_bounds_the_registry():
    auth = AuthManager(WebSettings(secret_key="k", password_plain=PASSWORD, session_idle_minutes=1))
    for i in range(5):
        auth.create_session("admin", "1.2.3.4", now=1000.0 + i)
    assert auth.active_sessions == 5
    auth.purge_expired(now=2000.0)
    assert auth.active_sessions == 0


def test_throttle_window_expires():
    throttle = LoginThrottle(max_attempts=2, lockout_seconds=60)
    throttle.record_failure("ip", now=100.0)
    throttle.record_failure("ip", now=100.0)
    assert throttle.locked_for("ip", now=110.0) > 0
    assert throttle.locked_for("ip", now=200.0) == 0


# --------------------------------------------------------------------------- #
# read-only posture and rate limiting
# --------------------------------------------------------------------------- #
@pytest.mark.parametrize("action", ["start", "stop", "release", "profile"])
def test_read_only_refuses_controls(tmp_path, action):
    app = create_app(settings_for(tmp_path, read_only=True))
    with TestClient(app, base_url="http://testserver") as c:
        csrf = login(c)
        response = c.post("/api/control", json={"action": action, "csrf_token": csrf, "profile": "demo"})
        assert response.status_code == 403


def test_kill_switch_works_even_read_only(tmp_path):
    """Stopping is always allowed; only starting is gated."""
    app = create_app(settings_for(tmp_path, read_only=True))
    with TestClient(app, base_url="http://testserver") as c:
        csrf = login(c)
        assert c.post("/api/control", json={"action": "kill", "csrf_token": csrf}).status_code == 200
        assert c.get("/api/state").json()["risk"]["kill_switch"] is True


def test_unknown_control_action_is_rejected(client):
    csrf = login(client)
    assert client.post("/api/control", json={"action": "drop", "csrf_token": csrf}).status_code == 400


def test_api_is_rate_limited(tmp_path):
    app = create_app(settings_for(tmp_path, api_rate_per_minute=5))
    with TestClient(app, base_url="http://testserver") as c:
        login(c)
        codes = [c.get("/api/state").status_code for _ in range(15)]
        assert 429 in codes


def test_pages_are_not_rate_limited(tmp_path):
    app = create_app(settings_for(tmp_path, api_rate_per_minute=1))
    with TestClient(app, base_url="http://testserver") as c:
        login(c)
        assert all(c.get("/").status_code == 200 for _ in range(5))


def test_disallowed_host_is_rejected(tmp_path):
    app = create_app(settings_for(tmp_path, allowed_hosts=["testserver"]))
    with TestClient(app, base_url="http://evil.example") as c:
        assert c.get("/healthz").status_code == 400


def test_report_limit_is_clamped(client):
    login(client)
    assert client.get("/api/report?limit=100000").status_code == 200
    assert client.get("/api/report?limit=-5").status_code == 200
