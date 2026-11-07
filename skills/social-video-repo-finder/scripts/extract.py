#!/usr/bin/env python3
"""
social-video-repo-finder / extract.py

Takes one or more saved posts ("favoritos") from YouTube or Instagram and
produces a structured JSON dossier that an AI agent can turn into:

  * a summary of what the video talks about,
  * the list of code repositories mentioned (and where exactly to find the code),
  * a classification (explainer/tutorial vs. showcase vs. other).

Inputs accepted (mix freely):
  - Direct URLs (YouTube video/short/playlist, Instagram reel/post/tv)
  - A .txt file with one URL per line
  - Instagram "Download your information" export: saved_posts.json /
    saved_collections.json
  - Google Takeout YouTube playlist CSV (e.g. "Liked videos.csv", "Watch later.csv")
  - Google Takeout watch-history.json

Dependencies:
  required : yt-dlp
  optional : youtube-transcript-api (better YouTube transcripts)
             faster-whisper or openai-whisper (transcribe Reels without captions)

Usage:
  python3 extract.py URL [URL ...] [--input FILE] [--out dossier.json]
                     [--cookies-from-browser chrome] [--whisper] [--lang es,en]
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass, field
from typing import Iterable

# --------------------------------------------------------------------------- #
# URL / repository detection (pure functions, unit-tested)
# --------------------------------------------------------------------------- #

URL_RE = re.compile(r"""https?://[^\s<>"'`)\]}]+""", re.IGNORECASE)
TRAILING_PUNCT = ".,;:!?¡¿…»”’"

# host -> regex capturing the canonical "owner/repo" part
REPO_HOSTS = {
    "github": re.compile(r"(?:www\.)?github\.com/([A-Za-z0-9-]+)/([A-Za-z0-9._-]+)", re.I),
    "gitlab": re.compile(r"(?:www\.)?gitlab\.com/([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+)", re.I),
    "bitbucket": re.compile(r"(?:www\.)?bitbucket\.org/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)", re.I),
    "huggingface": re.compile(
        r"(?:www\.)?huggingface\.co/((?:spaces|datasets)/)?([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)", re.I
    ),
    "codeberg": re.compile(r"(?:www\.)?codeberg\.org/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)", re.I),
}

# Other places where code usually lives
CODE_HOSTS = (
    "gist.github.com", "colab.research.google.com", "replit.com", "codesandbox.io",
    "stackblitz.com", "codepen.io", "kaggle.com", "npmjs.com", "pypi.org",
    "glitch.com", "jsfiddle.net", "vercel.app", "netlify.app", "github.io",
)

# Link shorteners / link-in-bio pages we should try to expand
SHORTENERS = (
    "bit.ly", "t.co", "tinyurl.com", "goo.gl", "ow.ly", "buff.ly", "rebrand.ly",
    "linktr.ee", "lnk.bio", "beacons.ai", "bio.link", "shorturl.at", "cutt.ly",
    "is.gd", "youtu.be/redirect", "l.instagram.com", "youtube.com/redirect",
)

GITHUB_RESERVED = {
    "orgs", "features", "topics", "marketplace", "sponsors", "settings", "about",
    "pricing", "login", "join", "explore", "collections", "trending", "apps",
    "enterprise", "security", "customer-stories", "readme", "site", "search",
}
REPO_TAIL_JUNK = {"blob", "tree", "issues", "pulls", "releases", "wiki", "actions"}

# Spoken / written mentions without a full URL, e.g.
#   "github punto com barra midudev barra ..." or "github.com/user/repo" w/o scheme
SPOKEN_GITHUB_RE = re.compile(
    r"github\s*(?:\.|dot|punto)\s*com\s*(?:/|slash|barra)\s*([A-Za-z0-9-]+)"
    r"(?:\s*(?:/|slash|barra)\s*([A-Za-z0-9._-]+))?",
    re.I,
)
BARE_GITHUB_RE = re.compile(r"(?<![/\w])github\.com/[A-Za-z0-9-]+(?:/[A-Za-z0-9._-]+)?", re.I)

LINK_HINT_RE = re.compile(
    r"(link (?:in|en) (?:la |el )?(?:bio|descripci[oó]n|description|comments?|comentarios?)"
    r"|enlace en (?:la |el )?(?:bio|descripci[oó]n|comentarios?)"
    r"|te lo dejo (?:en|abajo)|link below|repo (?:in|en) (?:la )?(?:bio|descripci[oó]n))",
    re.I,
)

KEYWORD_SIGNALS = {
    "repo": ["repositorio", "repository", "repo", "github", "gitlab", "open source",
             "código abierto", "clona", "clone", "git clone", "fork", "star"],
    "explainer": ["tutorial", "cómo", "como hacer", "how to", "explico", "explain",
                  "paso a paso", "step by step", "aprende", "learn", "guía", "guide",
                  "curso", "course", "introducción", "introduction", "en este vídeo",
                  "in this video", "vamos a ver", "let's"],
}


def clean_url(url: str) -> str:
    url = url.strip()
    while url and url[-1] in TRAILING_PUNCT:
        url = url[:-1]
    # unbalanced closing paren from markdown "(https://..)"
    if url.endswith(")") and url.count("(") < url.count(")"):
        url = url[:-1]
    return url


def find_urls(text: str) -> list[str]:
    if not text:
        return []
    seen, out = set(), []
    for m in URL_RE.finditer(text):
        u = clean_url(m.group(0))
        if u and u not in seen:
            seen.add(u)
            out.append(u)
    for m in BARE_GITHUB_RE.finditer(text):
        u = "https://" + clean_url(m.group(0))
        if u not in seen and not any(u in s for s in seen):
            seen.add(u)
            out.append(u)
    return out


def classify_repo_url(url: str) -> dict | None:
    """Return {'host','repo','url','path'} if url points to a code repository."""
    for host, rx in REPO_HOSTS.items():
        m = rx.search(url)
        if not m:
            continue
        if host == "github":
            owner, repo = m.group(1), m.group(2)
            if owner.lower() in GITHUB_RESERVED:
                return None
            repo = re.sub(r"\.git$", "", repo)
            if repo.lower() in REPO_TAIL_JUNK:
                return None
            tail = url[m.end():]
            path = None
            pm = re.match(r"/(?:blob|tree)/([^/?#]+)/?([^?#]*)", tail)
            if pm:
                path = {"ref": pm.group(1), "path": pm.group(2) or ""}
            return {"host": "github", "repo": f"{owner}/{repo}",
                    "url": f"https://github.com/{owner}/{repo}", "path": path}
        if host == "gitlab":
            full = m.group(1).split("/-/")[0].rstrip("/")
            full = re.sub(r"\.git$", "", full)
            if full.count("/") < 1:
                return None
            return {"host": "gitlab", "repo": full, "url": f"https://gitlab.com/{full}", "path": None}
        if host == "huggingface":
            kind = (m.group(1) or "").rstrip("/")
            owner, name = m.group(2), m.group(3)
            prefix = f"{kind}/" if kind else ""
            return {"host": "huggingface", "repo": f"{prefix}{owner}/{name}",
                    "url": f"https://huggingface.co/{prefix}{owner}/{name}", "path": None}
        owner, repo = m.group(1), re.sub(r"\.git$", "", m.group(2))
        base = {"bitbucket": "bitbucket.org", "codeberg": "codeberg.org"}[host]
        return {"host": host, "repo": f"{owner}/{repo}", "url": f"https://{base}/{owner}/{repo}", "path": None}
    return None


def is_code_host(url: str) -> bool:
    low = url.lower()
    return any(h in low for h in CODE_HOSTS)


def is_shortener(url: str) -> bool:
    low = url.lower().split("://", 1)[-1]
    return any(low.startswith(s) or low.startswith("www." + s) for s in SHORTENERS)


def find_spoken_github(text: str) -> list[dict]:
    """Detect 'github punto com barra user barra repo' style mentions in transcripts."""
    out = []
    for m in SPOKEN_GITHUB_RE.finditer(text or ""):
        owner, repo = m.group(1), m.group(2)
        out.append({
            "host": "github",
            "owner": owner,
            "repo": f"{owner}/{repo}" if repo else None,
            "url": f"https://github.com/{owner}/{repo}" if repo else f"https://github.com/{owner}",
            "evidence": m.group(0),
        })
    return out


def keyword_hits(text: str) -> dict[str, list[str]]:
    low = (text or "").lower()
    return {k: sorted({w for w in words if w in low}) for k, words in KEYWORD_SIGNALS.items()}


def detect_platform(url: str) -> str:
    low = url.lower()
    if "instagram.com" in low:
        return "instagram"
    if "youtube.com" in low or "youtu.be" in low:
        return "youtube"
    if "tiktok.com" in low:
        return "tiktok"
    return "other"


def youtube_id(url: str) -> str | None:
    m = re.search(r"(?:v=|youtu\.be/|shorts/|embed/|live/)([A-Za-z0-9_-]{11})", url)
    return m.group(1) if m else None


# --------------------------------------------------------------------------- #
# Input parsing (favourites exports)
# --------------------------------------------------------------------------- #

def _walk_json_for_urls(obj) -> Iterable[str]:
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in ("href", "url", "value", "titleUrl") and isinstance(v, str) and v.startswith("http"):
                yield v
            else:
                yield from _walk_json_for_urls(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from _walk_json_for_urls(v)


def load_inputs(path: str) -> list[str]:
    """Read URLs from txt / Instagram JSON export / Takeout CSV or JSON."""
    with open(path, encoding="utf-8-sig") as fh:
        raw = fh.read()
    lower = path.lower()
    urls: list[str] = []
    if lower.endswith(".json"):
        data = json.loads(raw)
        urls = [u for u in _walk_json_for_urls(data) if detect_platform(u) != "other"]
    elif lower.endswith(".csv"):
        rows = list(csv.reader(raw.splitlines()))
        for row in rows:
            for cell in row:
                cell = cell.strip()
                if cell.startswith("http"):
                    urls.append(cell)
                elif re.fullmatch(r"[A-Za-z0-9_-]{11}", cell):  # Takeout playlist CSV: Video ID
                    urls.append(f"https://www.youtube.com/watch?v={cell}")
    else:
        urls = find_urls(raw)
    seen, out = set(), []
    for u in urls:
        if u not in seen:
            seen.add(u)
            out.append(u)
    return out


# --------------------------------------------------------------------------- #
# Network side: yt-dlp metadata, transcripts, repo enrichment
# --------------------------------------------------------------------------- #

UA = "social-video-repo-finder/1.0"


def _ytdlp_base(args) -> list[str]:
    exe = shutil.which("yt-dlp")
    cmd = [exe] if exe else [sys.executable, "-m", "yt_dlp"]
    cmd += ["--no-warnings", "--ignore-errors"]
    if args.cookies_from_browser:
        cmd += ["--cookies-from-browser", args.cookies_from_browser]
    if args.cookies:
        cmd += ["--cookies", args.cookies]
    return cmd


def expand_playlist(url: str, args) -> list[str]:
    """If url is a playlist (e.g. Liked videos LL / Watch later WL), list its entries."""
    if "list=" not in url or youtube_id(url):
        return [url]
    cmd = _ytdlp_base(args) + ["--flat-playlist", "-J", url]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180).stdout
        data = json.loads(out)
        return [e.get("url") or f"https://www.youtube.com/watch?v={e['id']}"
                for e in data.get("entries", []) if e]
    except Exception as exc:  # noqa: BLE001
        print(f"  ! playlist expansion failed: {exc}", file=sys.stderr)
        return [url]


def fetch_metadata(url: str, args) -> dict:
    cmd = _ytdlp_base(args) + ["-J", "--skip-download", url]
    if args.with_comments:
        cmd += ["--write-comments", "--extractor-args", "youtube:max_comments=40,all,0,0"]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=240)
    if proc.returncode != 0 and not proc.stdout.strip():
        raise RuntimeError(proc.stderr.strip()[-500:] or "yt-dlp failed")
    return json.loads(proc.stdout)


def _vtt_to_text(vtt: str) -> str:
    lines, last = [], None
    for line in vtt.splitlines():
        line = line.strip()
        if not line or "-->" in line or line.startswith(("WEBVTT", "Kind:", "Language:", "NOTE")):
            continue
        line = re.sub(r"<[^>]+>", "", line)
        if line != last:
            lines.append(line)
            last = line
    return " ".join(lines)


def transcript_youtube_api(video_id: str, langs: list[str]) -> str | None:
    try:
        from youtube_transcript_api import YouTubeTranscriptApi  # type: ignore
    except ImportError:
        return None
    try:
        api = YouTubeTranscriptApi()
        if hasattr(api, "fetch"):  # >= 1.0
            fetched = api.fetch(video_id, languages=langs)
            return " ".join(s.text for s in fetched)
        segs = YouTubeTranscriptApi.get_transcript(video_id, languages=langs)  # legacy
        return " ".join(s["text"] for s in segs)
    except Exception:  # noqa: BLE001
        return None


def transcript_ytdlp_subs(url: str, langs: list[str], args) -> str | None:
    with tempfile.TemporaryDirectory() as tmp:
        cmd = _ytdlp_base(args) + [
            "--skip-download", "--write-subs", "--write-auto-subs",
            "--sub-langs", ",".join(f"{l}.*" for l in langs) + "," + ",".join(langs),
            "--sub-format", "vtt", "-o", os.path.join(tmp, "s.%(ext)s"), url,
        ]
        subprocess.run(cmd, capture_output=True, text=True, timeout=240)
        vtts = sorted(f for f in os.listdir(tmp) if f.endswith(".vtt"))
        if not vtts:
            return None
        # prefer the first requested language
        for lang in langs:
            for f in vtts:
                if f".{lang}" in f:
                    return _vtt_to_text(open(os.path.join(tmp, f), encoding="utf-8").read())
        return _vtt_to_text(open(os.path.join(tmp, vtts[0]), encoding="utf-8").read())


def transcript_whisper(url: str, args) -> str | None:
    """Download audio and transcribe locally (needed for most Instagram Reels)."""
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "a.%(ext)s")
        cmd = _ytdlp_base(args) + ["-x", "--audio-format", "mp3", "-o", out, url]
        subprocess.run(cmd, capture_output=True, text=True, timeout=600)
        files = [os.path.join(tmp, f) for f in os.listdir(tmp)]
        if not files:
            return None
        audio = files[0]
        try:
            from faster_whisper import WhisperModel  # type: ignore
            model = WhisperModel(args.whisper_model, compute_type="int8")
            segs, _ = model.transcribe(audio)
            return " ".join(s.text.strip() for s in segs)
        except ImportError:
            pass
        try:
            import whisper  # type: ignore
            return whisper.load_model(args.whisper_model).transcribe(audio)["text"]
        except ImportError:
            print("  ! no whisper backend installed (pip install faster-whisper)", file=sys.stderr)
            return None


def resolve_redirect(url: str) -> str:
    try:
        req = urllib.request.Request(url, method="HEAD", headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.geturl()
    except Exception:  # noqa: BLE001
        return url


def scrape_links_page(url: str) -> list[str]:
    """Linktree-style pages: pull every outbound link."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=10) as resp:
            html = resp.read(800_000).decode("utf-8", "replace")
        return find_urls(html)
    except Exception:  # noqa: BLE001
        return []


def _gh_get(path: str) -> dict | list | None:
    headers = {"User-Agent": UA, "Accept": "application/vnd.github+json"}
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    try:
        req = urllib.request.Request(f"https://api.github.com{path}", headers=headers)
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        return {"_error": e.code}
    except Exception as e:  # noqa: BLE001
        return {"_error": str(e)}


def enrich_github(repo: str, readme_chars: int = 3000) -> dict:
    info = _gh_get(f"/repos/{repo}")
    if not isinstance(info, dict) or "_error" in info:
        return {"exists": False, "error": (info or {}).get("_error") if isinstance(info, dict) else None}
    out = {
        "exists": True,
        "full_name": info.get("full_name"),
        "description": info.get("description"),
        "stars": info.get("stargazers_count"),
        "language": info.get("language"),
        "topics": info.get("topics", []),
        "default_branch": info.get("default_branch"),
        "pushed_at": info.get("pushed_at"),
        "archived": info.get("archived"),
        "homepage": info.get("homepage"),
        "clone": f"git clone https://github.com/{info.get('full_name')}.git",
    }
    tree = _gh_get(f"/repos/{repo}/contents")
    if isinstance(tree, list):
        out["top_level"] = [f"{e['name']}{'/' if e.get('type') == 'dir' else ''}" for e in tree][:40]
    rd = _gh_get(f"/repos/{repo}/readme")
    if isinstance(rd, dict) and rd.get("content"):
        import base64
        try:
            out["readme_excerpt"] = base64.b64decode(rd["content"]).decode("utf-8", "replace")[:readme_chars]
        except Exception:  # noqa: BLE001
            pass
    return out


def search_github(query: str, n: int = 3) -> list[dict]:
    """Fallback when the video mentions a project but gives no link."""
    from urllib.parse import quote
    res = _gh_get(f"/search/repositories?q={quote(query)}&sort=stars&per_page={n}")
    if not isinstance(res, dict) or "items" not in res:
        return []
    return [{"repo": i["full_name"], "url": i["html_url"], "stars": i["stargazers_count"],
             "description": i.get("description")} for i in res["items"]]


# --------------------------------------------------------------------------- #
# Orchestration
# --------------------------------------------------------------------------- #

@dataclass
class Dossier:
    input_url: str
    platform: str
    ok: bool = True
    error: str | None = None
    title: str | None = None
    uploader: str | None = None
    uploader_url: str | None = None
    upload_date: str | None = None
    duration_s: int | None = None
    description: str | None = None
    chapters: list = field(default_factory=list)
    tags: list = field(default_factory=list)
    transcript: str | None = None
    transcript_source: str | None = None
    links: list = field(default_factory=list)
    repositories: list = field(default_factory=list)
    code_links: list = field(default_factory=list)
    spoken_mentions: list = field(default_factory=list)
    link_hints: list = field(default_factory=list)
    signals: dict = field(default_factory=dict)
    github_search_suggestions: list = field(default_factory=list)


def process(url: str, args) -> Dossier:
    d = Dossier(input_url=url, platform=detect_platform(url))
    print(f"→ {url}", file=sys.stderr)
    try:
        meta = fetch_metadata(url, args)
    except Exception as exc:  # noqa: BLE001
        d.ok, d.error = False, f"metadata: {exc}"
        if d.platform == "instagram":
            d.error += " (Instagram suele requerir --cookies-from-browser <navegador>)"
        return d

    d.title = meta.get("title") or meta.get("fulltitle")
    d.uploader = meta.get("uploader") or meta.get("channel")
    d.uploader_url = meta.get("uploader_url") or meta.get("channel_url")
    d.upload_date = meta.get("upload_date")
    d.duration_s = meta.get("duration")
    d.description = meta.get("description") or ""
    d.chapters = [{"start": c.get("start_time"), "title": c.get("title")} for c in meta.get("chapters") or []]
    d.tags = meta.get("tags") or []

    # Transcript cascade
    langs = [l.strip() for l in args.lang.split(",") if l.strip()]
    vid = youtube_id(meta.get("webpage_url") or url)
    if d.platform == "youtube" and vid:
        d.transcript = transcript_youtube_api(vid, langs)
        d.transcript_source = "youtube-transcript-api" if d.transcript else None
    if not d.transcript:
        d.transcript = transcript_ytdlp_subs(url, langs, args)
        d.transcript_source = "yt-dlp subtitles" if d.transcript else None
    if not d.transcript and args.whisper:
        d.transcript = transcript_whisper(url, args)
        d.transcript_source = f"whisper ({args.whisper_model})" if d.transcript else None
    if d.transcript and len(d.transcript) > args.max_transcript:
        d.transcript = d.transcript[: args.max_transcript] + " …[truncated]"

    # Link harvesting: description/caption, pinned/top comments, transcript
    comments_text = ""
    if meta.get("comments"):
        top = sorted(meta["comments"], key=lambda c: (not c.get("is_pinned"), -(c.get("like_count") or 0)))
        own = [c for c in top if c.get("author_is_uploader") or c.get("is_pinned")]
        comments_text = "\n".join(c.get("text", "") for c in (own + top)[:40])

    sources = {"description": d.description, "comments": comments_text, "transcript": d.transcript or ""}
    raw_links: list[tuple[str, str]] = []
    for src, text in sources.items():
        for u in find_urls(text):
            raw_links.append((u, src))

    # expand shorteners / link-in-bio pages one level
    expanded: list[tuple[str, str]] = []
    for u, src in raw_links:
        expanded.append((u, src))
        if args.no_network_extras or not is_shortener(u):
            continue
        final = resolve_redirect(u)
        if final != u:
            expanded.append((final, f"{src}→redirect"))
        if any(h in final for h in ("linktr.ee", "lnk.bio", "beacons.ai", "bio.link")):
            for sub in scrape_links_page(final):
                expanded.append((sub, f"{src}→link-in-bio"))

    seen_repo = {}
    for u, src in expanded:
        d.links.append({"url": u, "source": src})
        repo = classify_repo_url(u)
        if repo:
            key = (repo["host"], repo["repo"].lower())
            if key in seen_repo:
                seen_repo[key]["sources"].add(src)
                continue
            repo["sources"] = {src}
            seen_repo[key] = repo
        elif is_code_host(u):
            d.code_links.append({"url": u, "source": src})

    for sm in find_spoken_github(d.transcript or "") + find_spoken_github(d.description):
        d.spoken_mentions.append(sm)
        if sm["repo"] and ("github", sm["repo"].lower()) not in seen_repo:
            seen_repo[("github", sm["repo"].lower())] = {
                "host": "github", "repo": sm["repo"], "url": sm["url"], "path": None,
                "sources": {"spoken"},
            }

    for repo in seen_repo.values():
        repo["sources"] = sorted(repo["sources"])
        if repo["host"] == "github" and not args.no_network_extras:
            repo["github"] = enrich_github(repo["repo"])
        d.repositories.append(repo)

    all_text = "\n".join([d.title or "", d.description, comments_text, d.transcript or ""])
    d.link_hints = sorted({m.group(0) for m in LINK_HINT_RE.finditer(all_text)})
    d.signals = keyword_hits(all_text)

    if not d.repositories and d.signals.get("repo") and not args.no_network_extras and d.title:
        d.github_search_suggestions = search_github(d.title[:80])
    return d


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("urls", nargs="*", help="YouTube / Instagram URLs")
    p.add_argument("--input", "-i", action="append", default=[],
                   help="txt / Instagram saved_posts.json / Takeout CSV or JSON (repeatable)")
    p.add_argument("--out", "-o", default="-", help="output JSON path (default stdout)")
    p.add_argument("--lang", default="es,en", help="preferred transcript languages")
    p.add_argument("--cookies-from-browser", help="chrome|firefox|edge|safari… (needed for Instagram / private lists)")
    p.add_argument("--cookies", help="Netscape cookies.txt file")
    p.add_argument("--whisper", action="store_true", help="transcribe audio locally when no captions exist")
    p.add_argument("--whisper-model", default="small")
    p.add_argument("--with-comments", action="store_true", help="also scan pinned/top comments for links")
    p.add_argument("--max-transcript", type=int, default=60000)
    p.add_argument("--limit", type=int, default=0, help="process at most N items")
    p.add_argument("--no-network-extras", action="store_true",
                   help="skip GitHub API, redirect expansion and link-in-bio scraping")
    args = p.parse_args(argv)

    urls = list(args.urls)
    for path in args.input:
        urls += load_inputs(path)
    if not urls:
        p.error("give at least one URL or --input file")

    expanded: list[str] = []
    for u in urls:
        expanded += expand_playlist(u, args) if detect_platform(u) == "youtube" else [u]
    if args.limit:
        expanded = expanded[: args.limit]

    results = [asdict(process(u, args)) for u in expanded]
    payload = json.dumps({"items": results, "count": len(results)}, ensure_ascii=False, indent=2)
    if args.out == "-":
        print(payload)
    else:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(payload)
        print(f"✓ dossier written to {args.out} ({len(results)} items)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
