"""Checks the website in site/ before it is committed or published.

    python scripts/site-check.py            # offline checks
    python scripts/site-check.py --online   # also asks GitHub about every link
    python scripts/site-check.py --strict   # fails if the private pattern list is missing
    python scripts/site-check.py --history origin/master..HEAD
                                            # also greps those commits (patches,
                                            # messages) and checks their dates

Offline it checks the size budgets, that every local link, image and
#anchor resolves with exact case, that every <img> has alt text and
width/height at half its pixels (the skin size table in app.js too), that
PNG/WebP files carry only pixel chunks, the contrast of the colour tokens
in style.css, and greps the published files for paths, time zones and
hardware names that should never go public.

The privacy grep has two pattern sets. Generic ones live in this file.
Personal ones (names, places, hardware) are read from the untracked
.git/info/site-privacy-patterns.txt, one regex per line, '#' comments,
so they are never committed. Hits are reported as file and line only;
neither the patterns nor the matched text are printed.

Exits 1 if any check fails. Needs Pillow.
"""

import argparse
import html.parser
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
SELF = Path(__file__).resolve()
LIVE = "https://superhelten.github.io/chronodesk/"
REPO_URL = "https://github.com/superhelten/chronodesk"

KB = 1024
TEXT_BUDGET = 60 * KB
IMAGE_BUDGET = 100 * KB
IMAGE_BUDGETS = {"hero-ring-anim.webp": 600 * KB, "og.png": 200 * KB}
SITE_BUDGET = 2.5 * KB * KB
SETUP_SIZE = 5.8 * KB * KB

PNG_CHUNKS = {b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"}
WEBP_CHUNKS = {b"VP8L", b"VP8 ", b"VP8X", b"ALPH", b"ANIM", b"ANMF"}

GENERIC_PATTERNS = [
    r"[A-Za-z]:\\Users\\",
    r"\\Users\\[^%\\]",
    r"[A-Za-z]:/Users/",
    r"AppData\\Local\\Temp",
    r"[D-Z]:\\(repos|SteamLibrary)",
    r"\b(Europe|America|Asia|Australia|Africa|Pacific|Atlantic)/[A-Z]",
    r"UTC[+-]\d",
    r"GMT[+-]\d",
    r"\bCES?T\b",
    r"\b(NVIDIA|GeForce|RTX|Radeon)\b",
]
PRIVATE_FILE = "info/site-privacy-patterns.txt"

failures = []
warnings = []


def fail(msg):
    failures.append(msg)
    print("FAIL " + msg)


def warn(msg):
    warnings.append(msg)
    print("WARN " + msg)


def ok(msg, since=None):
    """Reports a pass, unless the check has failed since `since` failures."""
    if since is None or len(failures) == since:
        print("ok   " + msg)


def rel(path):
    return path.relative_to(ROOT).as_posix()


# --- HTML -----------------------------------------------------------------

class Page(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.tags = []  # (tag, attrs dict, line)
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        self.tags.append((tag, a, self.getpos()[0]))
        if "id" in a:
            self.ids.add(a["id"])

    handle_startendtag = handle_starttag


def exists_exact(path):
    """True if the path exists under site/ with exactly this case."""
    cur = SITE
    for part in Path(path).parts:
        if not cur.is_dir() or part not in {p.name for p in cur.iterdir()}:
            return False
        cur = cur / part
    return cur.is_file()


def check_html(page, text):
    start = len(failures)
    refs = 0
    for tag, a, line in page.tags:
        for attr in ("href", "src"):
            v = a.get(attr)
            if v is None:
                continue
            where = f"index.html:{line} {attr}={v!r}"
            if v.startswith("//") or v.startswith("/"):
                fail(f"root-absolute path {where}")
            elif v.startswith("#"):
                refs += 1
                if v[1:] not in page.ids:
                    fail(f"no element with that id: {where}")
            elif re.match(r"[a-z]+:", v):
                continue
            else:
                refs += 1
                if not exists_exact(v.split("#")[0].split("?")[0]):
                    fail(f"missing file (exact case): {where}")
        if tag == "meta" and re.match(r"(og|twitter):(url|image)$", a.get("property", a.get("name", ""))):
            if not a.get("content", "").startswith(LIVE):
                fail(f"index.html:{line} {a.get('property', a.get('name'))} is not under {LIVE}")
        if tag == "link" and a.get("rel") == "canonical" and a.get("href") != LIVE:
            fail(f"index.html:{line} canonical is not {LIVE}")
    if not any(t == "link" and a.get("rel") == "canonical" for t, a, _ in page.tags):
        fail("index.html has no canonical link")
    ok(f"G9 {refs} local links and anchors resolve", start)


def natural(path):
    with Image.open(path) as im:
        return im.size


def check_images(page):
    start = len(failures)
    n = 0
    for tag, a, line in page.tags:
        if tag != "img":
            continue
        n += 1
        src = a.get("src", "")
        if "alt" not in a:
            fail(f"index.html:{line} <img src={src!r}> has no alt attribute")
        path = SITE / src
        if not path.is_file():
            continue  # reported by the link check
        w, h = natural(path)
        want = (str(w // 2), str(h // 2))
        if (a.get("width"), a.get("height")) != want:
            fail(f"index.html:{line} {src} width/height {a.get('width')}x{a.get('height')}, "
                 f"want {want[0]}x{want[1]} (half of {w}x{h})")
    ok(f"G12 {n} <img> tags checked", start)


def check_app_js():
    start = len(failures)
    js = (SITE / "app.js").read_text(encoding="utf-8")
    m = re.search(r"const DIM = \{(.*?)\};", js, re.S)
    c = re.search(r"const COL = \{(.*?)\};", js, re.S)
    if not m or not c:
        fail("app.js: DIM or COL table not found")
        return
    dim = {f: (int(w), int(h)) for f, w, h in re.findall(r"(\w+): \[(\d+), (\d+)\]", m.group(1))}
    colours = re.findall(r"(\w+): \[", c.group(1))
    want = {f"skin-{f}-{col}.webp" for f in dim for col in colours}
    have = {p.name for p in (SITE / "img").glob("skin-*.webp")}
    for name in sorted(have - want):
        fail(f"site/img/{name} is not reachable from the app.js tables")
    for f, size in dim.items():
        for col in colours:
            path = SITE / "img" / f"skin-{f}-{col}.webp"
            if not path.is_file():
                fail(f"app.js points at missing {rel(path)}")
                continue
            w, h = natural(path)
            if (w // 2, h // 2) != size:
                fail(f"app.js DIM.{f} = {size[0]}x{size[1]}, but {path.name} is {w}x{h}")
    for lit in re.findall(r"'(img/[^'$]+)'", js):
        if not exists_exact(lit):
            fail(f"app.js points at missing site/{lit}")
    ok(f"G12 app.js size table: {len(dim)} faces x {len(colours)} colours", start)


# --- Files ----------------------------------------------------------------

def check_budgets():
    text = sum((SITE / n).stat().st_size for n in ("index.html", "style.css", "app.js"))
    msg = f"G1 html+css+js {text / KB:.1f} KB (budget {TEXT_BUDGET // KB} KB)"
    ok(msg) if text <= TEXT_BUDGET else fail(msg)
    total = 0
    for p in sorted(SITE.rglob("*")):
        if not p.is_file():
            continue
        size = p.stat().st_size
        total += size
        if p.suffix in (".png", ".webp"):
            cap = IMAGE_BUDGETS.get(p.name, IMAGE_BUDGET)
            if size > cap:
                fail(f"G3 {rel(p)} {size / KB:.1f} KB over {cap // KB} KB")
    msg = f"G3 site/ {total / KB / KB:.2f} MB (budget {SITE_BUDGET / KB / KB} MB)"
    ok(msg) if total <= SITE_BUDGET else fail(msg)


def png_chunks(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    i = 8
    while i < len(data):
        n = int.from_bytes(data[i:i + 4], "big")
        yield data[i + 4:i + 8]
        i += 12 + n


def webp_chunks(data):
    if data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        raise ValueError("not a WebP")
    i = 12
    while i < len(data):
        n = int.from_bytes(data[i + 4:i + 8], "little")
        yield data[i:i + 4]
        i += 8 + n + (n & 1)


def check_chunks():
    start = len(failures)
    n = 0
    for p in sorted(SITE.rglob("*")):
        kind = {".png": (png_chunks, PNG_CHUNKS), ".webp": (webp_chunks, WEBP_CHUNKS)}.get(p.suffix)
        if not kind or not p.is_file():
            continue
        n += 1
        walk, allowed = kind
        try:
            extra = sorted({c.decode("latin-1") for c in walk(p.read_bytes())} - {c.decode() for c in allowed})
        except ValueError as e:
            fail(f"G10 {rel(p)}: {e}")
            continue
        if extra:
            fail(f"G10 {rel(p)} has chunks {', '.join(extra)}")
    ok(f"G10 {n} images carry pixel chunks only", start)


# --- Contrast -------------------------------------------------------------

def luminance(hexcol):
    rgb = [int(hexcol[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    lin = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb]
    return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]


def ratio(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def check_contrast():
    css = (SITE / "style.css").read_text(encoding="utf-8")
    root = re.search(r":root\s*\{(.*?)\}", css, re.S).group(1)
    tok = {k: v.lower() for k, v in re.findall(r"--([\w-]+):\s*(#[0-9a-fA-F]{6})\s*;", root)}
    walls = ["wall-0", "wall-1", "wall-2", "wall-3"]
    rows = []
    for fg in ("text", "text-2", "led", "amber"):
        rows += [(fg, bg, 4.5) for bg in ("bg", "surface", "surface-2")]
    rows += [("text-3", bg, 4.5) for bg in ("bg", "surface")]
    rows += [("bg", "led", 4.5)]
    rows += [(fg, bg, 3.0) for fg in ("line-strong", "led") for bg in ("surface", "surface-2")]
    rows += [(fg, bg, 3.0) for fg in ("text-2", "led") for bg in walls]
    print("     G6 contrast            ratio  need")
    bad = 0
    for fg, bg, need in rows:
        if fg not in tok or bg not in tok:
            fail(f"G6 token --{fg} or --{bg} missing from :root")
            continue
        r = ratio(tok[fg], tok[bg])
        mark = "  " if r >= need else "<-"
        print(f"     {fg:>11} on {bg:<9} {r:5.2f}  {need:3.1f} {mark}")
        if r < need:
            bad += 1
            fail(f"G6 --{fg} on --{bg} is {r:.2f}:1, needs {need}:1")
    if not bad:
        ok(f"G6 {len(rows)} colour pairs meet their contrast")


# --- Privacy --------------------------------------------------------------

def git(*args):
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True,
                          encoding="utf-8", errors="replace")


def private_patterns():
    res = git("rev-parse", "--git-path", PRIVATE_FILE)
    path = Path(res.stdout.strip()) if res.returncode == 0 and res.stdout.strip() else ROOT / ".git" / PRIVATE_FILE
    if not path.is_absolute():
        path = ROOT / path
    if not path.is_file():
        return None
    pats = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            pats.append(re.compile(line, re.I))
    return pats


def grep(name, text, sets):
    hits = 0
    for kind, pats in sets:
        for i, pat in enumerate(pats):
            for m in pat.finditer(text):
                hits += 1
                line = text.count("\n", 0, m.start()) + 1
                fail(f"G11 {name}:{line} matches {kind} pattern #{i + 1}")
    return hits


def privacy_files():
    files = [p for p in sorted(SITE.rglob("*")) if p.is_file()]
    files += [ROOT / "scripts" / "site-assets.py", ROOT / ".github" / "workflows" / "pages.yml"]
    return [p for p in files if p.is_file() and p.resolve() != SELF]


def readme_diff():
    """README lines added since the last pushed commit (the rest is already public)."""
    base = "origin/master" if git("rev-parse", "-q", "--verify", "origin/master").returncode == 0 else "HEAD"
    diff = git("diff", base, "--", "README.md").stdout
    return "\n".join(l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++"))


def check_privacy(strict, history):
    generic = [re.compile(p, re.I) for p in GENERIC_PATTERNS]
    private = private_patterns()
    if private is None:
        (fail if strict else warn)(f"G11 no .git/{PRIVATE_FILE}: personal patterns not checked")
        private = []
    else:
        print(f"     {len(private)} private patterns loaded")
    sets = [("generic", generic), ("private", private)]
    files = privacy_files()
    hits = sum(grep(rel(p), p.read_bytes().decode("latin-1"), sets) for p in files)
    hits += grep("README.md (added lines)", readme_diff(), sets)
    if not hits:
        ok(f"G11 privacy grep clean over {len(files)} files and the README diff")
    if history:
        # Every commit message, then every patch except this file's own
        # (its generic pattern list would match itself).
        msgs = git("log", "--format=commit %H%n%an <%ae>%n%B", history)
        patches = git("log", "-p", "--format=commit %H", history, "--", ".", ":(exclude)scripts/site-check.py")
        if msgs.returncode or patches.returncode:
            fail(f"git log {history} failed")
            return
        hits = grep(f"git log {history}", msgs.stdout, sets)
        hits += grep(f"git log -p {history}", patches.stdout, sets)
        if not hits:
            ok(f"G11 history {history} clean (messages and patches)")
        dates = git("log", "--format=%h %ad %cd", "--date=iso", history).stdout.split("\n")
        bad = [d.split()[0] for d in dates if d and not (d.endswith("+0000") and " +0000 " in d)]
        for h in bad:
            fail(f"commit {h} has a date outside +0000")
        if not bad:
            ok(f"every date in {history} is +0000")


# --- Online ---------------------------------------------------------------

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def head(url, follow=False):
    req = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "chronodesk-site-check"})
    opener = urllib.request.build_opener() if follow else urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(req, timeout=20) as r:
            return r.status, r.headers
    except urllib.error.HTTPError as e:
        return e.code, e.headers


def check_online(text):
    start = len(failures)
    urls = sorted({u.split("#")[0] for u in re.findall(re.escape(REPO_URL) + r"[^\"'\s<>]*", text)})
    for bare in urls:
        try:
            status, headers = head(bare)
        except OSError as e:
            fail(f"G9 {bare}: {e}")
            continue
        if status not in (200, 301, 302):
            fail(f"G9 {bare} returned {status}")
            continue
        m = re.search(r"/releases/latest/download/(.+)$", bare)
        if m:
            loc = headers.get("Location", "")
            if not loc.endswith("/" + m.group(1)):
                fail(f"G9 {bare} redirects to {loc or 'nothing'}, not a file named {m.group(1)}")
                continue
            if m.group(1) == "ChronoDesk-Setup.exe":
                size = int(head(bare, follow=True)[1].get("Content-Length") or 0)
                if abs(size - SETUP_SIZE) > SETUP_SIZE * 0.1:
                    warn(f"ChronoDesk-Setup.exe is {size / KB / KB:.1f} MB; the page says ~5.8 MB")
        print(f"     {status} {bare}")
    ok(f"G9 {len(urls)} GitHub links answer", start)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--online", action="store_true", help="HEAD every GitHub link")
    ap.add_argument("--strict", action="store_true", help="fail if the private pattern list is missing")
    ap.add_argument("--history", metavar="RANGE", help="also grep `git log -p RANGE` and check its dates")
    args = ap.parse_args()

    text = (SITE / "index.html").read_text(encoding="utf-8")
    page = Page()
    page.feed(text)

    check_budgets()
    check_html(page, text)
    check_images(page)
    check_app_js()
    check_chunks()
    check_contrast()
    check_privacy(args.strict, args.history)
    if args.online:
        check_online(text)

    print(f"\n{len(failures)} failed, {len(warnings)} warnings")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
