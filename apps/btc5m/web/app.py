"""FastAPI application.

Security posture
----------------
* Every page and every ``/api`` route requires an authenticated session; only
  ``/healthz`` and the static assets are public, and ``/healthz`` leaks nothing.
* State-changing requests need a CSRF token that matches the server-side session
  record, submitted as a form field or the ``X-CSRF-Token`` header.
* Controls are refused outright unless the operator set
  ``BTC5M_WEB_READONLY=0``, so the default deployment can watch but not trade.
* Responses are built from explicit allow-lists. Configuration is exposed
  through ``WebSettings.public_dict`` only, which cannot return a secret.
"""

from __future__ import annotations

import hmac
import logging
import secrets
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, Optional

from fastapi import Depends, FastAPI, Form, HTTPException, Request, Response, status
from fastapi.responses import HTMLResponse, JSONResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from starlette.middleware.trustedhost import TrustedHostMiddleware

from btc5m.config import load_profiles
from btc5m.errors import Btc5mError
from btc5m.logging_setup import configure
from btc5m.models import now_ts
from btc5m.reporting import build_report

from .auth import CSRF_COOKIE, SESSION_COOKIE, AuthManager, SessionRecord
from .runner import SessionRunner
from .security import RateLimitMiddleware, SecurityHeadersMiddleware, client_ip
from .settings import WebSettings

log = logging.getLogger(__name__)
HERE = Path(__file__).resolve().parent


def create_app(settings: Optional[WebSettings] = None) -> FastAPI:
    settings = settings or WebSettings()
    configure(json_logs=True)

    if settings.generated_secret:
        log.warning("BTC5M_WEB_SECRET is unset: generated an ephemeral key, sessions end on restart")
    if settings.generated_password:
        log.warning(
            "No credentials configured. Temporary login -> user=%s password=%s "
            "(set BTC5M_WEB_PASSWORD_HASH for anything but local use)",
            settings.username, settings.generated_password,
        )

    profiles = load_profiles()
    auth = AuthManager(settings)
    runner = SessionRunner(
        profiles=profiles, profile_name=settings.profile, source=settings.source,
        equity_usd=settings.equity_usd, db_path=settings.db_path, live=False,
    )

    @asynccontextmanager
    async def lifespan(_: FastAPI):
        yield
        runner.close()

    # OpenAPI and the docs UIs are disabled: an operator console has no reason to
    # publish its own attack surface map.
    app = FastAPI(title="btc5m", docs_url=None, redoc_url=None, openapi_url=None,
                  lifespan=lifespan)
    app.state.settings = settings
    app.state.auth = auth
    app.state.runner = runner

    if settings.allowed_hosts and settings.allowed_hosts != ["*"]:
        app.add_middleware(TrustedHostMiddleware, allowed_hosts=settings.allowed_hosts)
    app.add_middleware(RateLimitMiddleware, per_minute=settings.api_rate_per_minute,
                       trust_proxy=settings.trust_proxy_headers)
    app.add_middleware(SecurityHeadersMiddleware, https_only=settings.https_only)

    app.mount("/static", StaticFiles(directory=HERE / "static"), name="static")
    templates = Jinja2Templates(directory=str(HERE / "templates"))
    templates.env.autoescape = True

    # -- helpers ------------------------------------------------------------- #
    def set_session_cookies(response: Response, cookie: str, record: SessionRecord) -> None:
        common = {
            "httponly": True,
            "secure": settings.cookie_secure,
            "samesite": "strict",
            "path": "/",
            "max_age": settings.session_max_hours * 3600,
        }
        response.set_cookie(SESSION_COOKIE, cookie, **common)
        response.set_cookie(CSRF_COOKIE, record.csrf_token, **common)

    def clear_session_cookies(response: Response) -> None:
        response.delete_cookie(SESSION_COOKIE, path="/")
        response.delete_cookie(CSRF_COOKIE, path="/")

    def current_session(request: Request) -> Optional[SessionRecord]:
        return auth.resolve(request.cookies.get(SESSION_COOKIE))

    def require_session(request: Request) -> SessionRecord:
        record = current_session(request)
        if record is None:
            wants_json = request.url.path.startswith("/api")
            raise HTTPException(
                status_code=status.HTTP_401_UNAUTHORIZED,
                detail="authentication required",
                headers={} if wants_json else {"Location": "/login"},
            )
        return record

    def require_csrf(request: Request, record: SessionRecord, submitted: Optional[str]) -> None:
        token = submitted or request.headers.get("x-csrf-token")
        if not auth.csrf_ok(record, token):
            raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail="invalid csrf token")

    def require_writable() -> None:
        if settings.read_only:
            raise HTTPException(
                status_code=status.HTTP_403_FORBIDDEN,
                detail="read-only deployment: set BTC5M_WEB_READONLY=0 to enable controls",
            )

    @app.exception_handler(HTTPException)
    async def on_http_error(request: Request, exc: HTTPException):
        if exc.status_code == 401 and not request.url.path.startswith("/api"):
            return RedirectResponse("/login", status_code=status.HTTP_303_SEE_OTHER)
        return JSONResponse({"error": exc.detail}, status_code=exc.status_code)

    # -- public -------------------------------------------------------------- #
    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        """Liveness only — deliberately says nothing about the session."""
        return {"status": "ok"}

    # -- auth ---------------------------------------------------------------- #
    def login_page(request: Request, csrf_token: str, error: Optional[str] = None,
                   status_code: int = 200):
        return templates.TemplateResponse(
            request, "login.html",
            {"csrf_token": csrf_token, "error": error, "read_only": settings.read_only},
            status_code=status_code,
        )

    @app.get("/login", response_class=HTMLResponse)
    async def login_form(request: Request):
        if current_session(request) is not None:
            return RedirectResponse("/", status_code=status.HTTP_303_SEE_OTHER)
        # A fresh token per render, so a token planted by a third party can never
        # be the one the form submits.
        token = secrets.token_urlsafe(32)
        response = login_page(request, token)
        response.set_cookie(CSRF_COOKIE, token, httponly=True, secure=settings.cookie_secure,
                            samesite="strict", path="/", max_age=900)
        return response

    @app.post("/login")
    async def login(request: Request, username: str = Form(...), password: str = Form(...),
                    csrf_token: str = Form(...)):
        # Pre-session CSRF: double-submit against the cookie set by GET /login.
        cookie_token = request.cookies.get(CSRF_COOKIE)
        if not cookie_token:
            # The usual cause is a misconfigured deployment rather than an
            # attack: BTC5M_WEB_HTTPS=1 marks the cookie Secure, so a browser on
            # plain http:// never sends it back and login becomes impossible.
            hint = (" The CSRF cookie is marked Secure (BTC5M_WEB_HTTPS=1) but this request"
                    " arrived over plain HTTP, so the browser withheld it. Serve over TLS or"
                    " set BTC5M_WEB_HTTPS=0 for local use.") if settings.https_only else ""
            log.warning("login rejected: no CSRF cookie present.%s", hint)
            return login_page(request, secrets.token_urlsafe(32),
                              "Session cookie missing — reload the page and try again."
                              + hint, status.HTTP_403_FORBIDDEN)
        if not hmac.compare_digest(cookie_token, csrf_token):
            raise HTTPException(status_code=403, detail="invalid csrf token")

        ip = client_ip(request, settings.trust_proxy_headers)
        remaining = auth.throttle.locked_for(ip)
        if remaining:
            return login_page(request, csrf_token,
                              f"Too many attempts. Try again in {int(remaining)}s.",
                              status.HTTP_429_TOO_MANY_REQUESTS)

        if not auth.verify(username, password):
            auth.throttle.record_failure(ip)
            log.warning("failed login from %s", ip)
            # One message for every failure mode: no user enumeration.
            return login_page(request, csrf_token, "Invalid credentials.",
                              status.HTTP_401_UNAUTHORIZED)

        auth.throttle.reset(ip)
        auth.purge_expired()
        cookie, record = auth.create_session(username, ip)
        log.info("login succeeded for %s from %s", username, ip)
        response = RedirectResponse("/", status_code=status.HTTP_303_SEE_OTHER)
        set_session_cookies(response, cookie, record)
        return response

    @app.post("/logout")
    async def logout(request: Request, record: SessionRecord = Depends(require_session),
                     csrf_token: Optional[str] = Form(None)):
        require_csrf(request, record, csrf_token)
        auth.revoke(request.cookies.get(SESSION_COOKIE))
        response = RedirectResponse("/login", status_code=status.HTTP_303_SEE_OTHER)
        clear_session_cookies(response)
        return response

    # -- dashboard ----------------------------------------------------------- #
    @app.get("/", response_class=HTMLResponse)
    async def dashboard(request: Request, record: SessionRecord = Depends(require_session)):
        return templates.TemplateResponse(
            request, "dashboard.html",
            {
                "csrf_token": record.csrf_token,
                "username": record.username,
                "read_only": settings.read_only,
                "profiles": profiles.names,
                "settings": settings.public_dict(),
            },
        )

    # -- api ----------------------------------------------------------------- #
    @app.get("/api/state")
    async def api_state(record: SessionRecord = Depends(require_session)) -> dict[str, Any]:
        return runner.state()

    @app.get("/api/report")
    async def api_report(limit: int = 25, record: SessionRecord = Depends(require_session)) -> dict[str, Any]:
        limit = max(1, min(200, limit))
        return build_report(runner.store, at=now_ts(), limit=limit)

    @app.get("/api/profiles")
    async def api_profiles(record: SessionRecord = Depends(require_session)) -> dict[str, Any]:
        return {name: profiles.get(name).as_dict() for name in profiles.names}

    @app.post("/api/control")
    async def api_control(request: Request, record: SessionRecord = Depends(require_session)) -> dict[str, Any]:
        payload = await request.json()
        require_csrf(request, record, payload.get("csrf_token"))
        action = str(payload.get("action") or "")
        profile = payload.get("profile")

        # kill-switch is a safety action and stays available even read-only.
        if action == "kill":
            log.warning("kill switch engaged by %s", record.username)
            return {"result": runner.engage_kill_switch(f"web:{record.username}")}

        require_writable()
        try:
            if action == "start":
                return {"result": runner.start(profile)}
            if action == "stop":
                return {"result": runner.stop()}
            if action == "release":
                log.warning("kill switch released by %s", record.username)
                return {"result": runner.release_kill_switch()}
            if action == "profile":
                if not profile:
                    raise HTTPException(status_code=400, detail="profile is required")
                return {"result": runner.switch_profile(str(profile))}
        except Btc5mError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        raise HTTPException(status_code=400, detail=f"unknown action {action!r}")

    return app


app = create_app()
