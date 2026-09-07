# Single image, two entrypoints: the web console and the terminal dashboard.
#
# Runs as an unprivileged user with a read-only root filesystem in the compose
# file; only /data (the SQLite store) is writable.
FROM python:3.12-slim AS base

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

WORKDIR /app

# Dependencies first, so application edits do not invalidate the wheel layer.
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY btc5m ./btc5m
COPY web ./web
COPY config ./config
COPY tools ./tools
COPY pyproject.toml ./

# Unprivileged runtime user; /data is the only path the app needs to write.
RUN useradd --system --create-home --uid 10001 btc5m \
 && mkdir -p /data \
 && chown -R btc5m:btc5m /data /app
USER btc5m

ENV BTC5M_DB=/data/btc5m.db \
    BTC5M_CONFIG=/app/config/btc_5m_profiles.yaml \
    PYTHONPATH=/app

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=4s --start-period=10s --retries=3 \
  CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://127.0.0.1:8000/healthz', timeout=3).status==200 else 1)"

# Default: the hardened web console. Override the command for the terminal UI:
#   docker compose run --rm tui
CMD ["python", "-m", "uvicorn", "web.app:app", "--host", "0.0.0.0", "--port", "8000", \
     "--proxy-headers", "--forwarded-allow-ips", "*"]
