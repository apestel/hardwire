#!/usr/bin/env python3
"""Hardwire end-to-end scenario (standard library only).

Exercises the real server over HTTP: public share pages, downloads (full +
range), JWT auth middleware, Google-login redirect (PKCE), stats endpoints,
7z archive task and the live-update WebSocket handshake.

Environment (all optional, set by e2e/run.sh):
  BASE_URL      default http://localhost:18093
  JWT_SECRET    default e2e-test-secret-0123456789-abcdef-0123456789
  ADMIN_SUB     admin_users.id of the pre-seeded admin (default 1, the
                first row on a fresh DB, created from HARDWIRE_ADMIN_EMAIL)
  ADMIN_EMAIL   default admin@e2e.test (must match HARDWIRE_ADMIN_EMAIL)
  E2E_DATA_DIR  directory the server uses as HARDWIRE_DATA_DIR (only needed
                to compare the downloaded bytes of the generated test file)
"""
import base64, hashlib, hmac, json, os, re, socket, sys, time, urllib.request, urllib.error

BASE = os.environ.get("BASE_URL", "http://localhost:18093")
SECRET = os.environ.get("JWT_SECRET", "e2e-test-secret-0123456789-abcdef-0123456789").encode()
ADMIN_SUB = int(os.environ.get("ADMIN_SUB", "1"))
ADMIN_EMAIL = os.environ.get("ADMIN_EMAIL", "admin@e2e.test")
DATA_DIR = os.environ.get("E2E_DATA_DIR", ".sqlx-test/e2e/data")

HOST, PORT = (BASE.split("//", 1)[1].split("/", 1)[0].split(":") + ["80"])[:2]
PORT = int(PORT)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None  # keep 30x responses so we can inspect the Location


OPENER = urllib.request.build_opener(NoRedirect)


def b64url(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def make_jwt(sub: int, email: str, ttl: int = 3600) -> str:
    header = b64url(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = b64url(json.dumps(
        {"sub": sub, "email": email, "exp": int(time.time()) + ttl},
        separators=(",", ":"),
    ).encode())
    sig = b64url(hmac.new(SECRET, f"{header}.{payload}".encode(), hashlib.sha256).digest())
    return f"{header}.{payload}.{sig}"


GOOD = make_jwt(ADMIN_SUB, ADMIN_EMAIL)
EXPIRED = make_jwt(ADMIN_SUB, ADMIN_EMAIL, ttl=-3600)

results = []


def check(name, cond, detail=""):
    results.append((name, cond))
    print(f"{'PASS' if cond else 'FAIL'}  {name}  {detail}")


def req(method, path, token=None, data=None, headers=None, expect=None):
    url = BASE + path
    body = json.dumps(data).encode() if data is not None else None
    h = {"Content-Type": "application/json"}
    if token:
        h["Authorization"] = f"Bearer {token}"
    if headers:
        h.update(headers)
    r = urllib.request.Request(url, data=body, headers=h, method=method)
    try:
        resp = OPENER.open(r, timeout=30)
        code, payload, rh = resp.status, resp.read(), dict(resp.headers)
    except urllib.error.HTTPError as e:
        code, payload, rh = e.code, e.read(), dict(e.headers)
    except Exception as e:
        return 0, b"", {"error": str(e)}
    if expect is not None and code != expect:
        raise AssertionError(f"{method} {path}: got {code}, want {expect}: {payload[:300]!r}")
    return code, payload, rh


def header(rh, name):
    # dict() of an HTTPMessage keeps wire-case keys (axum sends lowercase)
    return rh.get(name) or rh.get(name.lower()) or ""


# ── 1. healthcheck + SPA ──────────────────────────────────────────────
req("GET", "/healthcheck", expect=200)
check("healthcheck 200", True)
code, body, _ = req("GET", "/admin", expect=200)
check("SPA /admin 200", b"<script" in body or b"<!doctype" in body.lower())

# ── 2. Google login: 303 with a proper PKCE URL (no real credentials) ─
code, body, rh = req("GET", "/admin/auth/google/login")
loc = header(rh, "Location")
check("google/login → 30x vers accounts.google.com",
      code in (302, 303, 307) and "accounts.google.com" in loc, f"code={code} loc={loc[:60]}")
check("login: PKCE S256 + state + nonce",
      "code_challenge_method=S256" in loc and "state=" in loc and "nonce=" in loc)

# ── 3. JWT middleware ─────────────────────────────────────────────────
code, body, _ = req("GET", "/admin/api/users")
check("sans token → 401", code == 401, f"code={code}")
code, body, _ = req("GET", "/admin/api/users", token=EXPIRED)
check("token expiré → 401", code == 401, f"code={code}")
code, body, _ = req("GET", "/admin/api/users", token=GOOD, expect=200)
users = json.loads(body)
check("token valide → users 200",
      isinstance(users, list) and any(u.get("email") == ADMIN_EMAIL for u in users), str(users)[:80])

# ── 4. rescan + list_files ────────────────────────────────────────────
req("POST", "/admin/api/files/rescan", token=GOOD, expect=204)
time.sleep(1)
code, body, _ = req("GET", "/admin/api/list_files", token=GOOD, expect=200)
files = json.loads(body)
check("rescan → list_files contient les 2 fichiers de test",
      "file-a.txt" in json.dumps(files) and "file-b.bin" in json.dumps(files), str(files)[:120])

# ── 5. share links (normal + expired) ─────────────────────────────────
code, body, _ = req("POST", "/admin/api/create_shared_link", token=GOOD,
                    data={"file_paths": ["file-a.txt", "file-b.bin"], "expires_at": None}, expect=200)
share = json.loads(body)
share_id = share["id"]
check("create_shared_link → id + url", bool(share_id) and share_id in share.get("url", ""), str(share)[:100])
code, body, _ = req("POST", "/admin/api/create_shared_link", token=GOOD,
                    data={"file_paths": ["file-a.txt"], "expires_at": 1000000000}, expect=200)
expired_id = json.loads(body)["id"]

# ── 6. public page + downloads ────────────────────────────────────────
code, body, _ = req("GET", f"/s/{share_id}", expect=200)
html = body.decode(errors="replace")
check("page publique 200 (Askama)", b"download" in body.lower(), f"{len(body)} octets")
check("lien expiré → 404", req("GET", f"/s/{expired_id}")[0] == 404)
check("lien inconnu → 404", req("GET", "/s/nonexistent123")[0] == 404)

links = re.findall(r'href="[^"]*/s/' + re.escape(share_id) + r'/(\d+)"\s+download="([^"]+)"', html)
check("liens fichiers extraits de la page (2 fichiers)", len(links) == 2, str(links))
target = next((fid for fid, name in links if name == "file-b.bin"), None)
target_a = next((fid for fid, name in links if name == "file-a.txt"), None)
check("liens file-a.txt et file-b.bin présents", bool(target) and bool(target_a), f"a={target_a} b={target}")

code, dl, _ = req("GET", f"/s/{share_id}/{target}", expect=200)
check("téléchargement file-b.bin 200 (1 MiB)", len(dl) == 1048576, f"{len(dl)} octets")
want = open(os.path.join(DATA_DIR, "file-b.bin"), "rb").read()
check("téléchargement: contenu identique (sha256)",
      hashlib.sha256(dl).hexdigest() == hashlib.sha256(want).hexdigest())

code, dl, _ = req("GET", f"/s/{share_id}/{target}", headers={"Range": "bytes=100-199"})
check("range request → 206 (100 octets)", code == 206 and len(dl) == 100, f"code={code} len={len(dl)}")

code, dl, _ = req("GET", f"/s/{share_id}/{target_a}", expect=200)
check("téléchargement file-a: contenu exact", dl == b"hello hardwire e2e\n", repr(dl[:40]))

# ── 7. stats (download tracking is async — poll for eventual consistency)
code, body, _ = req("GET", "/admin/api/stats/downloads", token=GOOD, expect=200)
s = json.loads(body)
deadline = time.time() + 15
while s["total_downloads"] < 2 and time.time() < deadline:
    time.sleep(0.5)
    code, body, _ = req("GET", "/admin/api/stats/downloads", token=GOOD, expect=200)
    s = json.loads(body)
check("stats/downloads: >= 2 téléchargements persistés", s["total_downloads"] >= 2, str(s)[:110])

code, body, _ = req("GET", "/admin/api/stats/downloads/by_period?period=day", token=GOOD, expect=200)
p = json.loads(body)
deadline = time.time() + 15
while not p.get("data") and time.time() < deadline:
    time.sleep(0.5)
    code, body, _ = req("GET", "/admin/api/stats/downloads/by_period?period=day", token=GOOD, expect=200)
    p = json.loads(body)
check("stats/by_period?period=day (strftime lié en paramètre, sqlx 0.9) a un bucket",
      len(p.get("data", [])) >= 1, str(p)[:110])
code, body, _ = req("GET", "/admin/api/stats/downloads/by_period?period=month", token=GOOD, expect=200)
check("stats/by_period?period=month", isinstance(json.loads(body).get("data"), list))
code, body, _ = req("GET", "/admin/api/stats/downloads/recent?limit=5", token=GOOD, expect=200)
r = json.loads(body)
check("stats/recent >= 2 entrées", len(r) >= 2, str(r)[:100])
code, body, _ = req("GET", "/admin/api/stats/downloads/status", token=GOOD, expect=200)
check("stats/status 200", True, str(json.loads(body))[:100])

# ── 8. 7z archive task (worker + download) ────────────────────────────
code, body, _ = req("POST", "/admin/api/tasks", token=GOOD,
                    data={"type": "CreateArchive",
                          "data": {"files": ["file-a.txt", "file-b.bin"], "output_path": "e2e-archive"}},
                    expect=200)
task = json.loads(body)
task_id = task.get("id") or task.get("task_id")
check("task d'archivage créée", bool(task_id), str(task_id))
final = None
deadline = time.time() + 90
while time.time() < deadline:
    code, body, _ = req("GET", f"/admin/api/tasks/{task_id}", token=GOOD, expect=200)
    final = json.loads(body)
    if final.get("status") in ("Completed", "Failed"):
        break
    time.sleep(1)
check("archive: task Completed", bool(final) and final.get("status") == "Completed",
      str(final)[:150] if final else "timeout")
if final and final.get("status") == "Completed":
    code, body, _ = req("GET", f"/admin/api/tasks/{task_id}/download", token=GOOD)
    check("archive: téléchargement 7z (magic 37 7A BC AF 27 1C)",
          len(body) > 6 and body[:6] == bytes.fromhex("377abcaf271c"),
          f"{len(body)} octets, header={body[:6].hex()}")
else:
    check("archive: téléchargement 7z (magic 37 7A BC AF 27 1C)", False, "task non complétée")

# ── 9. WebSocket live_update (handshake 101) ──────────────────────────
ws_ok, ws_detail = False, ""
try:
    s = socket.create_connection((HOST, PORT), timeout=5)
    s.sendall((f"GET /admin/live_update?token={GOOD} HTTP/1.1\r\n"
               f"Host: {HOST}:{PORT}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
               "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
    resp = s.recv(4096).decode(errors="replace")
    ws_ok = "101" in resp.split("\r\n", 1)[0]
    ws_detail = resp.split("\r\n", 1)[0]
    s.close()
except Exception as e:
    ws_detail = str(e)
check("WebSocket /admin/live_update → 101 Switching Protocols", ws_ok, ws_detail[:60])

# ── summary ───────────────────────────────────────────────────────────
fails = [name for name, ok in results if not ok]
print(f"\n=== e2e: {len(results) - len(fails)}/{len(results)} checks OK ===")
if fails:
    print("FAILED:", ", ".join(fails))
sys.exit(1 if fails else 0)