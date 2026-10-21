"""Build the Deck-ready ports list published as index/deck-ready.json.

Sources (all open or ours; no third-party port table is read or copied):
  1. This repo's ports.json, the Ports Launcher catalog (GPL-3.0, same license as the fork).
  2. Maintainers' own .recomp.json files, found with GitHub code search (open format, each
     maintainer owns their file).
  3. GitHub repository topics (recomp, source-port, decomp, ...).

Each candidate's recent releases are read with batched GraphQL and rated by release asset names:
Deck build > Linux build > Proton only > Other assets > Source only. The list keeps Deck and Linux
x86_64 builds of games. Ratings come from file names, not test launches.

Each run is compared with the committed file. New projects are added, changed ones updated, and
projects that fell off the list stay one cycle with a `missing_since` date before they are removed.
The file is only rewritten when something changed.

Needs `gh` logged in (locally) or GH_TOKEN set (Actions). Code search requires authentication.

Usage (run with python -I):
  python -I tools/deck_ready_index.py [--out index/deck-ready.json] [--summary summary.md]
                                      [--cache DIR] [--allow-shrink]
"""

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = 1
GRACE_DAYS = 6  # weekly runs: a project missing for one full cycle is removed on the next run
MIN_KEPT_RATIO = 0.7  # refuse to publish a list that shrank by more than 30 percent
SUMMARY_LIST_LIMIT = 60

TOPICS = ["recomp", "n64recomp", "static-recompilation", "recompilation", "xenonrecomp",
          "source-port", "pc-port", "decomp", "decompilation"]
GRAPHQL_BATCH = 25
API_ATTEMPTS = 5

DECK_RE = re.compile(r"steam[-_ .]?deck|(?<![a-z])deck(?![a-z])|steamos", re.I)
LINUX_RE = re.compile(r"linux|appimage|flatpak|ubuntu|(?<![a-z])lin(?:64)?(?![a-z])|\.deb$|\.rpm$", re.I)
ARM_RE = re.compile(r"aarch64|arm64|armv\d|armhf|(?<![a-z])arm(?![a-z])", re.I)
WIN_RE = re.compile(r"win(?:dows|32|64)?(?![a-z])|\.exe$|\.msi$|msvc", re.I)
JUNK_RE = re.compile(r"\.(sha\d*|sig|asc|md5|txt|sbom|json|pdb|dsym|apk|ipa|dmg)$|checksum|sha256|debug-sym", re.I)

CONSOLE_RE = re.compile(
    r"\b(n64|nintendo ?64|game ?cube|wii ?u|wii|ps1|psx|ps2|ps3|psp|playstation|xbox ?360|xbox|dreamcast|"
    r"saturn|snes|super nintendo|genesis|mega ?drive|game ?boy|gba|gbc|nds|nintendo ds|3ds|neo ?geo|"
    r"arcade|ms-?dos|dos game|pc port|native port)\b", re.I)
GAME_TOPICS = {"game", "games", "video-game", "videogame", "n64", "nintendo64", "nintendo-64", "gamecube",
               "playstation", "ps1", "ps2", "psx", "xbox", "xbox360", "xbox-360", "dreamcast", "snes", "nes",
               "gba", "nds", "pc-port", "retro-games", "retrogaming"}
# Ports Launcher catalog tags that name a system.
CATALOG_SYSTEM_TAGS = {"n64": "N64", "ps1": "PS1", "ps2": "PS2", "ps4": "PS4", "psp": "PSP", "360": "Xbox 360",
                       "xbox": "Xbox", "xone": "Xbox One", "gc": "GameCube", "wii": "Wii", "wiiu": "Wii U",
                       "switch": "Switch", "snes": "SNES", "nes": "NES", "gba": "GBA", "gb": "Game Boy",
                       "ds": "NDS", "3ds": "3DS", "smd": "Genesis", "arcade": "Arcade", "msdos": "DOS",
                       "pc": "PC", "macos": "Mac"}
TOOL_RE = re.compile(
    r"\b(tool|tools|toolkit|library|framework|toolchain|decompiler|disassembler|sdk|plugin|ghidra|ida pro|"
    r"binary ninja|emulator|compiler|linker|assembler|utility|utilities|template|boilerplate|runtime)\b", re.I)

OUTPUT_FIELDS = ("id", "github_id", "name", "system", "repo", "repo_url", "host", "catalog_folder", "catalog_disabled",
                 "kind", "deck_rating", "readme_mentions_deck", "archived", "sources", "evidence",
                 "recomp_json", "first_seen")
ABOUT = ("Projects with a Deck or Linux x86_64 build in a recent release. Sources: Ports Launcher catalog, "
         "maintainers' .recomp.json files, GitHub topics. Ratings come from release asset names and are not "
         "hand-checked; a test launch on the Deck is the proof. Entries with missing_since were not found in "
         "the latest refresh and are removed after one more cycle. `generated` is the date the list last "
         "changed, not the date it was last checked.")


def log(msg):
    print(msg, flush=True)


def gh_api(args, body=None):
    """Run `gh api ...`, retrying transient failures and rate limits. Returns parsed JSON.

    A GraphQL response with partial errors still exits non-zero but carries data; that is returned
    so the caller can look at the individual errors.
    """
    last = ""
    for attempt in range(API_ATTEMPTS):
        proc = subprocess.run(["gh", "api", *args], input=body, capture_output=True, text=True,
                              encoding="utf-8", timeout=300)
        try:
            payload = json.loads(proc.stdout) if proc.stdout.strip() else None
        except json.JSONDecodeError:
            payload = None
        if proc.returncode == 0 and payload is not None:
            return payload
        if isinstance(payload, dict) and payload.get("data"):
            return payload
        last = (proc.stderr + proc.stdout).strip()[:300]
        limited = re.search(r"rate limit|secondary|abuse", last, re.I)
        wait = 65 if limited else 5 * (attempt + 1)
        log(f"  gh api failed (attempt {attempt + 1}/{API_ATTEMPTS}), waiting {wait}s: {last[:120]}")
        time.sleep(wait)
    raise RuntimeError(f"gh api {' '.join(args[:3])} kept failing: {last}")


def github_slug(url):
    m = re.search(r"github\.com/([^/\s]+)/([^/\s#?]+)", url or "", re.I)
    if not m:
        return None
    repo = m.group(2)
    if repo.lower().endswith(".git"):
        repo = repo[:-4]
    return f"{m.group(1)}/{repo}"


def gitlab_slug(url):
    m = re.search(r"gitlab\.com/(.+?)(?:/-/|\.git$|/?$)", url or "", re.I)
    return m.group(1).rstrip("/") if m else None


def add(cands, key, **fields):
    entry = cands.setdefault(key, {"key": key, "sources": []})
    for src in fields.pop("sources", []):
        if src not in entry["sources"]:
            entry["sources"].append(src)
    for k, v in fields.items():
        if v is not None and k not in entry:
            entry[k] = v


def collect_catalog(cands, catalog_path):
    data = json.loads(Path(catalog_path).read_text(encoding="utf-8-sig"))
    for p in data["ports"]:
        tags = [t.lower() for t in p.get("tags", []) if isinstance(t, str)]
        if "android_only" in tags:
            continue
        name = p.get("name") or p.get("-name")
        folder = p.get("folder") or p.get("-folder")
        disabled = not isinstance(p.get("name"), str) or not isinstance(p.get("folder"), str)
        src = p.get("source")
        linux_url = src.get("linux") if isinstance(src, dict) else None
        url = linux_url or (src if isinstance(src, str) else next(iter(src.values()), None) if isinstance(src, dict) else None)
        if url and not re.match(r"https?://", url):
            url = None  # placeholders such as "??" are not sources
        meta = {"name": name, "catalog_folder": folder, "catalog_disabled": disabled, "catalog_tags": tags,
                "sources": ["catalog"]}
        if github_slug(url):
            add(cands, "github:" + github_slug(url).lower(), host="github", repo=github_slug(url), **meta)
        elif gitlab_slug(url):
            add(cands, "gitlab:" + gitlab_slug(url).lower(), host="gitlab", repo=gitlab_slug(url), **meta)
        elif url:
            add(cands, "url:" + (folder or name or url).lower(), host="direct", download_url=url,
                linux_specific_url=bool(linux_url), **meta)
        else:
            add(cands, "catalog:" + (folder or name or "?").lower(), host="none", **meta)


def search_pages(query, kind, delay, sort=None):
    """Yield every result item of a GitHub search, page by page (the API caps at 1000).

    Repository searches are sorted by stars so that a topic past the 1000 cap loses its least-starred
    tail instead of reshuffling week to week.
    """
    extra = ["-f", f"sort={sort}", "-f", "order=desc"] if sort else []
    for page in range(1, 11):
        for attempt in range(3):
            res = gh_api(["-X", "GET", f"search/{kind}", "-f", f"q={query}", "-f", "per_page=100",
                          "-f", f"page={page}", *extra])
            if not res.get("incomplete_results"):
                break
            log(f"  incomplete search results, retrying page {page}")
            time.sleep(10)
        else:
            raise RuntimeError(f"search {kind} {query!r} page {page} stayed incomplete")
        items = res.get("items", [])
        yield from items
        if len(items) < 100:
            return
        time.sleep(delay)


def collect_recomp_json(cands):
    found = 0
    for it in search_pages("filename:.recomp.json", "code", 7):
        if it.get("path") != ".recomp.json":
            continue
        repo = it["repository"]["full_name"]
        add(cands, "github:" + repo.lower(), host="github", repo=repo, sources=["recomp_json"])
        found += 1
    log(f"recomp_json repos: {found}")
    if found == 0:
        raise RuntimeError("code search found no .recomp.json files; refusing to continue")


def collect_topics(cands):
    for topic in TOPICS:
        n = 0
        for it in search_pages(f"topic:{topic} fork:false", "repositories", 2.5, sort="stars"):
            add(cands, "github:" + it["full_name"].lower(), host="github", repo=it["full_name"],
                sources=[f"topic:{topic}"])
            n += 1
        log(f"topic {topic}: {n}")
        if n == 0:
            raise RuntimeError(f"topic search for {topic} returned nothing; refusing to continue")
        time.sleep(2.5)


REPO_FIELDS = """
  databaseId nameWithOwner url isArchived isFork description stargazerCount
  repositoryTopics(first: 20) { nodes { topic { name } } }
  releases(first: 5, orderBy: {field: CREATED_AT, direction: DESC}) {
    nodes { tagName publishedAt isDraft isPrerelease releaseAssets(first: 60) { nodes { name } } }
  }
  recomp: object(expression: "HEAD:.recomp.json") { ... on Blob { text } }
  readme: object(expression: "HEAD:README.md") { ... on Blob { text } }
"""


def graphql_batch(batch):
    """Read one batch of repos. Returns {repo: (node or None, status)}; raises if a batch keeps failing.

    A repo that GitHub reports as not found, or that has any other error scoped to it (takedown,
    forbidden), gets node None and a status instead of failing the run. A missing node with no error
    at all means a bad response, so the batch is retried.
    """
    parts = []
    for j, repo in enumerate(batch):
        owner, name = repo.split("/", 1)
        parts.append(f"r{j}: repository(owner: {json.dumps(owner)}, name: {json.dumps(name)}) {{{REPO_FIELDS}}}")
    query = "query {\n" + "\n".join(parts) + "\n rateLimit { remaining cost } }"
    for attempt in range(API_ATTEMPTS):
        res = gh_api(["graphql", "--input", "-"], body=json.dumps({"query": query}))
        data = res.get("data") or {}
        errors = {}
        for e in res.get("errors", []):
            path = e.get("path") or []
            if len(path) == 1:
                errors[path[0]] = e.get("type") or "error"
            else:
                log(f"  graphql error below repo level: {json.dumps(e)[:200]}")
        out, ok = {}, True
        for j, repo in enumerate(batch):
            node = data.get(f"r{j}")
            if node is not None:
                out[repo] = (node, "ok")
            elif f"r{j}" in errors:
                kind = errors[f"r{j}"]
                out[repo] = (None, "not_found" if kind == "NOT_FOUND" else f"error:{kind}")
            else:
                ok = False
                break
        if ok:
            log(f"  rate remaining: {(data.get('rateLimit') or {}).get('remaining')}")
            return out
        log(f"  graphql batch incomplete (attempt {attempt + 1}/{API_ATTEMPTS}): "
            f"{json.dumps(res.get('errors', []))[:200]}")
        time.sleep(10 * (attempt + 1))
    raise RuntimeError("graphql batch kept returning incomplete data")


def fetch_github(repos, cache):
    fetched = {}
    todo = []
    for repo in repos:
        f = cache / (repo.replace("/", "__") + ".json") if cache else None
        if f and f.exists():
            fetched[repo] = json.loads(f.read_text(encoding="utf-8"))
        else:
            todo.append(repo)
    log(f"github repos: {len(repos)}, to fetch: {len(todo)}")
    for i in range(0, len(todo), GRAPHQL_BATCH):
        batch = todo[i:i + GRAPHQL_BATCH]
        for repo, (node, status) in graphql_batch(batch).items():
            if node:
                readme = ((node.pop("readme") or {}).get("text") or "")
                node["readme_mentions_deck"] = bool(re.search(r"steam\s*deck|steamos", readme, re.I))
            fetched[repo] = {"status": status, "node": node}
            if cache:
                cache.mkdir(parents=True, exist_ok=True)
                (cache / (repo.replace("/", "__") + ".json")).write_text(json.dumps(fetched[repo]), encoding="utf-8")
        log(f"  {i + len(batch)}/{len(todo)}")
    return fetched


def fetch_gitlab(repos):
    fetched = {}
    for repo in repos:
        url = f"https://gitlab.com/api/v4/projects/{urllib.parse.quote(repo, safe='')}/releases?per_page=5"
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "deck-ready-index"}),
                                        timeout=30) as r:
                rels = json.loads(r.read().decode("utf-8"))
            status = "ok"
        except Exception as e:  # network or 404: record and move on
            rels, status = [], f"error: {e}"
        fetched[repo] = {"status": status, "releases": rels}
    return fetched


def classify(assets):
    names = [a for a in assets if not JUNK_RE.search(a)]
    deck = [a for a in names if DECK_RE.search(a) and not ARM_RE.search(a)]
    linux = [a for a in names if (LINUX_RE.search(a) or a.lower().endswith(".appimage")) and not ARM_RE.search(a)]
    win = [a for a in names if WIN_RE.search(a) and not LINUX_RE.search(a)]
    return names, deck, linux, win


def rate(names, deck, linux, win):
    if not names:
        return "source_only"
    if deck:
        return "deck_build"
    if linux:
        return "linux_build"
    if win:
        return "proton_only"
    return "other_assets"


def pick_release(nodes):
    """Newest stable release with assets, else newest pre-release with assets."""
    usable = [n for n in nodes if not n.get("isDraft") and (n.get("releaseAssets") or {}).get("nodes")]
    stable = [n for n in usable if not n.get("isPrerelease")]
    return (stable or usable or [None])[0]


def kind_of(entry, node):
    if "catalog" in entry["sources"] or "recomp_json" in entry["sources"]:
        return "game"
    text = " ".join([entry.get("repo", ""), (node or {}).get("description") or ""])
    topics = set(entry.get("topics", []))
    signal = bool(CONSOLE_RE.search(text)) or bool(topics & GAME_TOPICS)
    if not signal:
        return "not_a_game"
    return "maybe_tool" if TOOL_RE.search((node or {}).get("description") or "") else "likely_game"


def build_entry(c, gh, gl):
    e = dict(c)
    e["sources"] = list(c["sources"])
    node = None
    if c.get("host") == "github":
        cached = gh.get(c["repo"]) or {"status": "not_fetched"}
        node = cached.get("node")
        e["check"] = cached["status"]
        if node:
            e["repo"] = node["nameWithOwner"]
            e["github_id"] = node.get("databaseId")
            e["repo_url"] = node["url"]
            e["archived"] = node["isArchived"]
            e["description"] = node.get("description")
            e["topics"] = [t["topic"]["name"] for t in (node.get("repositoryTopics") or {}).get("nodes", [])]
            e["readme_mentions_deck"] = node.get("readme_mentions_deck")
            try:
                rj = json.loads(((node.get("recomp") or {}).get("text")) or "null")
            except json.JSONDecodeError:
                rj = None
            if isinstance(rj, dict):
                e["recomp_json"] = {k: rj.get(k) for k in ("game", "system", "type", "status", "targets", "toolchain")}
                if not e.get("name") and isinstance(rj.get("game"), str):
                    e["name"] = rj["game"]
                if "recomp_json" not in e["sources"]:
                    e["sources"].append("recomp_json")
            rel = pick_release((node.get("releases") or {}).get("nodes", []))
            assets = [a["name"] for a in (rel or {}).get("releaseAssets", {}).get("nodes", [])]
            release = {"tag": rel["tagName"], "date": rel["publishedAt"], "prerelease": rel["isPrerelease"]} if rel else None
        else:
            assets, release = [], None
    elif c.get("host") == "gitlab":
        cached = gl.get(c["repo"]) or {"status": "not_fetched", "releases": []}
        e["check"] = cached["status"]
        e["repo_url"] = f"https://gitlab.com/{c['repo']}"
        rel = (cached.get("releases") or [None])[0]
        links = ((rel or {}).get("assets") or {}).get("links", [])
        assets = [l.get("name") or l.get("url", "").rsplit("/", 1)[-1] for l in links]
        release = {"tag": rel.get("tag_name"), "date": rel.get("released_at"), "prerelease": rel.get("upcoming_release")} if rel else None
    elif c.get("host") == "direct":
        e["check"] = "direct_url"
        url = c["download_url"]
        assets = [url.rsplit("/", 1)[-1]]
        if c.get("linux_specific_url") and not LINUX_RE.search(assets[0]):
            assets = [assets[0] + " (linux)"]
        release = None
    else:
        e["check"] = "no_source"
        assets, release = [], None

    names, deck, linux, win = classify(assets)
    e["deck_rating"] = rate(names, deck, linux, win)
    e["evidence"] = {"release": release, "deck_assets": sorted(deck), "linux_assets": sorted(linux),
                     "windows_assets": sorted(win), "asset_count": len(names)}
    if not e.get("name"):
        e["name"] = (e.get("repo") or c["key"]).split("/")[-1]
    tag_system = next((CATALOG_SYSTEM_TAGS[t] for t in e.get("catalog_tags", []) if t in CATALOG_SYSTEM_TAGS), None)
    e["system"] = (e.get("recomp_json") or {}).get("system") or tag_system or \
        next((m.group(0) for m in [CONSOLE_RE.search(f"{e.get('description') or ''} {e['name']}")] if m), None)
    e["kind"] = kind_of(e, node)
    return e


def project_id(e):
    """Stable id: host plus the resolved repo (so a renamed repo keeps one id), else the catalog key."""
    if e.get("host") in ("github", "gitlab"):
        return f"{e['host']}:{e['repo'].lower()}"
    return e["key"]


def ready_projects(cands, gh, gl):
    """Rate every candidate and return the ones that belong on the list, as output records."""
    merged = {}
    for c in cands.values():
        e = build_entry(c, gh, gl)
        e["id"] = project_id(e)
        if e["id"] in merged:  # renamed repos can resolve to the same project from two old names
            merged[e["id"]]["sources"] = sorted(set(merged[e["id"]]["sources"]) | set(e["sources"]))
        else:
            merged[e["id"]] = e
    out = []
    for e in merged.values():
        if e["deck_rating"] in ("deck_build", "linux_build") and e["kind"] in ("game", "likely_game"):
            rec = {k: e.get(k) for k in OUTPUT_FIELDS if k != "first_seen"}
            rec["sources"] = sorted(e["sources"])
            out.append(rec)
    return out


def merge_with_previous(new_ready, old_projects, today, grace_days=GRACE_DAYS):
    """Combine a fresh list with the committed one. Returns (projects, changes)."""
    old_by_id = {p["id"]: p for p in old_projects}
    old_by_gid = {p["github_id"]: p for p in old_projects if p.get("github_id")}
    matched = set()
    changes = {"added": [], "changed": [], "revived": [], "newly_missing": [], "still_missing": [], "removed": []}
    result = []
    for p in new_ready:
        # A renamed GitHub repo keeps its numeric id, so it stays the same project.
        old = old_by_id.get(p["id"]) or old_by_gid.get(p.get("github_id"))
        if old is not None:
            matched.add(old["id"])
        if old is None:
            p["first_seen"] = today
            changes["added"].append(p)
        else:
            p["first_seen"] = old.get("first_seen", today)
            if "missing_since" in old:
                changes["revived"].append(p)
            diff = sorted(k for k in set(p) | set(old) if k != "missing_since" and p.get(k) != old.get(k))
            if diff:
                changes["changed"].append((p, diff))
        result.append(p)
    for old in old_projects:
        if old["id"] in matched:
            continue
        since = old.get("missing_since")
        if since is None:
            kept = dict(old, missing_since=today)
            result.append(kept)
            changes["newly_missing"].append(kept)
        elif (date.fromisoformat(today) - date.fromisoformat(since)).days >= grace_days:
            changes["removed"].append(old)
        else:
            result.append(old)
            changes["still_missing"].append(old)
    result.sort(key=lambda x: (x["name"].lower(), x["id"]))
    return result, changes


def render_summary(changes, total, active):
    lines = ["## Deck-ready index", "", f"{active} projects on the list, {total - active} no longer found "
             f"(kept one cycle), {total} entries in the file.", ""]
    sections = [("Added", "added"), ("Changed", "changed"), ("Back again", "revived"),
                ("No longer found (kept one cycle)", "newly_missing"),
                ("Removed after one cycle", "removed")]
    any_change = False
    for title, key in sections:
        items = changes[key]
        if not items:
            continue
        any_change = True
        lines.append(f"### {title} ({len(items)})")
        for it in items[:SUMMARY_LIST_LIMIT]:
            if key == "changed":
                p, fields = it
                lines.append(f"- {p['name']} (`{p['repo'] or p['id']}`): {', '.join(fields)}")
            else:
                lines.append(f"- {it['name']} (`{it.get('repo') or it['id']}`)")
        if len(items) > SUMMARY_LIST_LIMIT:
            lines.append(f"- and {len(items) - SUMMARY_LIST_LIMIT} more")
        lines.append("")
    if not any_change:
        lines.append("No changes since the committed list.")
    return "\n".join(lines) + "\n"


def dump(doc):
    return json.dumps(doc, indent=1, ensure_ascii=False) + "\n"


def load_previous(out_path):
    if not out_path.exists():
        return []
    doc = json.loads(out_path.read_text(encoding="utf-8"))
    if doc.get("schema") != SCHEMA:
        raise RuntimeError(f"{out_path} has schema {doc.get('schema')}, expected {SCHEMA}")
    return doc["projects"]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--catalog", default=str(ROOT / "ports.json"))
    ap.add_argument("--out", default=str(ROOT / "index" / "deck-ready.json"))
    ap.add_argument("--summary", help="write the change summary (markdown) here")
    ap.add_argument("--cache", help="reuse repo reads from this folder (for local runs)")
    ap.add_argument("--allow-shrink", action="store_true", help="publish even if the list shrank by over 30 percent")
    args = ap.parse_args()
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # project names are not always ASCII

    out_path = Path(args.out)
    today = date.today().isoformat()
    old_projects = load_previous(out_path)

    cands = {}
    collect_catalog(cands, args.catalog)
    collect_recomp_json(cands)
    collect_topics(cands)
    log(f"candidates: {len(cands)}")

    gh = fetch_github(sorted({c["repo"] for c in cands.values() if c.get("host") == "github"}),
                      Path(args.cache) if args.cache else None)
    gl = fetch_gitlab(sorted({c["repo"] for c in cands.values() if c.get("host") == "gitlab"}))

    new_ready = ready_projects(cands, gh, gl)
    projects, changes = merge_with_previous(new_ready, old_projects, today)
    active = sum(1 for p in projects if "missing_since" not in p)
    old_active = sum(1 for p in old_projects if "missing_since" not in p)
    log(f"on the list: {active} (previous {old_active})")
    summary = render_summary(changes, len(projects), active)
    print(summary)
    if args.summary:
        Path(args.summary).write_text(summary, encoding="utf-8")
    if old_active >= 50 and active < old_active * MIN_KEPT_RATIO and not args.allow_shrink:
        raise SystemExit(f"list shrank from {old_active} to {active}; a source probably failed. "
                         "Check the logs, then rerun with --allow-shrink if the drop is real.")

    if projects == old_projects:
        log("no changes; file left as is")
        return
    out_path.parent.mkdir(parents=True, exist_ok=True)
    doc = {"schema": SCHEMA, "generated": today, "about": ABOUT, "count": active,
           "no_longer_found": len(projects) - active, "projects": projects}
    out_path.write_bytes(dump(doc).encode("utf-8"))
    log(f"wrote {out_path}")


if __name__ == "__main__":
    sys.exit(main())
