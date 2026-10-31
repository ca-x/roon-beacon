"""Contract-fixture browser QA; run with python3 tests/frontend_qa.py from the repository root."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / "crates/roon-beacon/src/web"
FIXTURE = {"scenario": "playing", "required": False, "requests": [], "position": 42, "volume": 36,
           "length": 255, "track_title": "A quiet afternoon", "zone_id": "zone-one", "output_id": "output-one"}


def player_fixture():
    if FIXTURE["scenario"] == "empty":
        return {"cores": [], "selected_core_id": None, "zones": [], "discovery_error": None}
    core = {"core_id": "core-one", "name": "Music room", "host": "192.0.2.1", "http_port": 9100,
            "connection": "connected", "error": None}
    if FIXTURE["scenario"] == "authorization":
        core["connection"] = "awaiting_authorization"
        return {"cores": [core], "selected_core_id": core["core_id"], "zones": [], "discovery_error": None}
    zone = {"zone_id": FIXTURE["zone_id"], "display_name": "Living room", "state": "playing",
            "is_previous_allowed": False, "is_next_allowed": True, "is_pause_allowed": True,
            "is_play_allowed": False, "is_seek_allowed": True, "seek_position": FIXTURE["position"],
            "now_playing": {"length": FIXTURE["length"], "image_key": "qa-cover", "one_line": {"line1": FIXTURE["track_title"]},
                            "three_line": {"line1": FIXTURE["track_title"], "line2": "QA artist", "line3": "Browser fixture · not production data"}},
            "outputs": [
                {"output_id": FIXTURE["output_id"], "display_name": "Living room", "volume": {
                    "type": "number", "min": 10, "max": FIXTURE.get("volume_max", 80), "step": 2, "value": FIXTURE["volume"], "is_muted": False}},
                {"output_id": "output-fixed", "display_name": "Fixed receiver", "volume": {
                    "type": "fixed", "min": 0, "max": 100, "value": 100, "step": 1}},
                {"output_id": "output-db", "display_name": "Amplifier", "volume": {
                    "type": "db", "min": -60, "max": 0, "value": -24.5, "step": .5, "is_muted": True}},
                {"output_id": "output-relative", "display_name": "Relative receiver", "volume": {
                    "type": "incremental", "min": 0, "max": 0, "value": 0, "step": 1}}
            ]}
    if FIXTURE["scenario"] == "no-zones":
        return {"cores": [core], "selected_core_id": core["core_id"], "zones": [], "discovery_error": None}
    other_zone = {**zone, "zone_id": "zone-two", "display_name": "Headphones", "state": "stopped", "outputs": [],
                  "now_playing": None, "is_previous_allowed": False, "is_next_allowed": False,
                  "is_play_allowed": True, "is_pause_allowed": False, "is_seek_allowed": False}
    return {"cores": [core, {**core, "core_id": "core-two", "name": "Other Core", "connection": "offline"}],
            "selected_core_id": None if FIXTURE["scenario"] == "unselected" else core["core_id"],
            "zones": [zone, other_zone] if FIXTURE["scenario"] != "unselected" else [], "discovery_error": None}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def reply(self, data, status=200, cookie=None):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        if cookie:
            self.send_header("set-cookie", cookie)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.route()

    def do_POST(self):
        self.route()

    def route(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"{}")
        path = self.path.split("?")[0]
        if path == "/__qa/state":
            if self.command == "POST":
                FIXTURE.update(body)
            return self.reply(FIXTURE)
        if path == "/" or path.startswith("/assets/") or path in {"/favicon.png", "/apple-touch-icon.png"}:
            name = "index.html" if path == "/" else path.removeprefix("/assets/").lstrip("/")
            if name not in {"index.html", "styles.css", "app.js", "logo.png", "favicon.png", "apple-touch-icon.png"}:
                return self.reply({"error": "Not found"}, 404)
            content = (WEB / name).read_bytes()
            if name == "app.js":
                content = b"Object.defineProperty(crypto, 'randomUUID', {value: undefined});\n" + content
            self.send_response(200)
            self.send_header("content-type", {"index.html": "text/html", "styles.css": "text/css", "app.js": "application/javascript"}.get(name, "image/png"))
            self.send_header("content-length", str(len(content)))
            self.end_headers()
            return self.wfile.write(content)
        authenticated = "qa_session=yes" in self.headers.get("cookie", "")
        if path == "/api/auth":
            return self.reply({"required": FIXTURE["required"], "authenticated": authenticated})
        if path == "/api/login":
            if body.get("token") != "fixture-token":
                return self.reply({"error": "Invalid web access token"}, 401)
            return self.reply({"ok": True}, cookie="qa_session=yes; HttpOnly; SameSite=Strict; Path=/")
        if FIXTURE["required"] and not authenticated:
            return self.reply({"error": "Authentication required"}, 401)
        FIXTURE["requests"].append({"path": path, "method": self.command, "body": body})
        if path == "/api/logout":
            return self.reply({"ok": True}, cookie="qa_session=; Max-Age=0; Path=/")
        if path == "/api/player":
            if FIXTURE["scenario"] == "offline":
                return self.reply({"error": "Fixture Core is offline"}, 503)
            return self.reply(player_fixture())
        if path == "/api/player/core" and FIXTURE.get("core_expire"):
            FIXTURE["core_expire"] = False
            return self.reply({"error": "Session expired while selecting Core"}, 401, cookie="qa_session=; Max-Age=0; Path=/")
        if path == "/api/status":
            return self.reply({"version": "qa", "local_ip": None, "remote_ip": None, "uptime_secs": 1234,
                               "config": {"name": "Fixture Beacon", "broadcast_interval_secs": 30,
                                          "live_proxy_timeout_ms": 800, "reverse_proxy": False, "proxy_ip": None},
                               "metrics": {"sood_queries": 0, "live_core_responses": 0, "template_fallbacks": 0, "tcp_connections": 0}})
        if path == "/api/audio/status":
            disabled = FIXTURE["scenario"] in {"empty", "authorization"}
            return self.reply({"enabled": not disabled, "available": not disabled, "state": "disabled" if disabled else "waiting",
                               "name": "Fixture Beacon", "listeners": 0, "bytes_streamed": 0, "pcm_bytes_received": 0,
                               "receiver_diagnostics": "<script>fixture diagnostic</script>", "encoder_diagnostics": None, "error": None})
        if path == "/api/audio/network":
            if self.command == "POST":
                if FIXTURE.get("network_save_error"):
                    return self.reply({"error": "Fixture network save failed"}, 500)
                FIXTURE["audio_interface"] = body["interface"]
                return self.reply({"ok": True})
            return self.reply({"interface": FIXTURE.get("audio_interface"), "interfaces": [
                {"name": "eth0", "addresses": ["192.0.2.10", "2001:db8::10"]},
                {"name": "wlan0", "addresses": ["192.0.2.11"]}]})
        if path.startswith("/api/player/image/"):
            return self.reply({"error": "Cover unavailable in fixture"}, 404)
        if path == "/api/player/browse":
            assert 0 < body["count"] <= 100
            assert body["session"] and len(body["session"]) <= 128
            if FIXTURE.get("navigation_fixture"):
                title, level = "Library", 1
                items = [{"title": "Search", "item_key": "nav-search", "input_prompt": {"prompt": "Search", "action": "Go"}}]
                items += [{"title": label, "item_key": "nav-" + label.lower(), "hint": "list"}
                          for label in ("Artists", "Albums", "Tracks", "Composers", "Tags")]
                if body.get("item_key") == "nav-artists":
                    title, level = "Artists", 2
                    items = [{"title": label, "item_key": "artist-" + label.lower(), "hint": "list"}
                             for label in ("Library", "Search", "Artists")]
                elif body.get("item_key") == "artist-library":
                    title, level = "Library", 3
                    items = [{"title": "Search", "item_key": "song-search", "hint": "action_list"}]
                elif body.get("item_key") == "song-search":
                    title, level = "Search", 4
                    items = [{"title": label, "item_key": "action-" + label.lower(), "hint": "action"}
                             for label in ("Play Now", "Add Next", "Queue")]
                elif "input" in body:
                    title, level = "Search results", 2
                    items = [{"title": label, "item_key": "artist-" + label.lower(), "hint": "list"}
                             for label in ("Library", "Search", "Artists")]
                return self.reply({"action": "list", "list": {"title": title, "count": len(items), "level": level},
                                   "items": items, "offset": 0})
            if body.get("item_key") == "fail":
                return self.reply({"error": "Fixture browse failure"}, 502)
            if body.get("item_key") == "play-now":
                return self.reply({"action": "message", "message": "Roon queued this album.", "is_error": False,
                                   "list": None, "items": [], "offset": 0})
            if body.get("item_key") == "roon-error":
                return self.reply({"action": "message", "message": "Roon cannot play this selection.", "is_error": True,
                                   "list": None, "items": [], "offset": 0})
            if body.get("item_key") == "album":
                return self.reply({"action": "list", "list": {"title": "Album actions", "count": 2, "level": 2, "hint": "action_list"},
                                   "items": [{"title": "Play now (from Roon)", "item_key": "play-now", "hint": "action"},
                                             {"title": "Unavailable Roon action", "item_key": "roon-error", "hint": "action"}], "offset": 0})
            if body["hierarchy"] == "search" and "input" not in body:
                items = [{"title": "Search Roon", "item_key": "search-prompt", "input_prompt": {"prompt": "Artist, album or track", "action": "Search", "value": "", "is_password": False}}]
                return self.reply({"action": "list", "list": {"title": "Search", "count": 1, "level": 0}, "items": items, "offset": 0})
            offset = body.get("offset", 0)
            items = [{"title": "A quiet afternoon", "subtitle": "QA artist · album", "item_key": "album", "image_key": "qa-cover"},
                     {"title": "Test an error", "item_key": "fail"},
                     {"title": "<img src=x onerror=alert(1)>", "subtitle": "Untrusted text is never HTML", "item_key": "album"},
                     {"title": "A" * 71, "subtitle": "Long unbroken title regression", "item_key": "album"}]
            if offset:
                items = [{"title": "Second page", "item_key": "album"}]
            else:
                items += [{"title": f"Fixture library item {index}", "subtitle": "QA only", "item_key": "album"} for index in range(4, 40)]
            return self.reply({"action": "list", "list": {"title": "Search results" if "input" in body else "Library", "count": 41, "level": 1}, "items": items, "offset": offset})
        if path == "/api/player/control" and body.get("control") == "stop":
            return self.reply({"error": "Fixture transport rejected stop"}, 502)
        if path == "/api/player/seek" and body.get("zone_id") == FIXTURE["zone_id"]:
            FIXTURE["position"] = body["seconds"]
        if path == "/api/player/volume" and body.get("output_id") == FIXTURE["output_id"]:
            FIXTURE["volume"] = body["value"]
        if path == "/api/config" and body.get("name") == "Invalid fixture":
            content = b"Failed to deserialize the JSON body into the target type: fixture validation"
            self.send_response(422)
            self.send_header("content-type", "text/plain")
            self.send_header("content-length", str(len(content)))
            self.end_headers()
            return self.wfile.write(content)
        if path in {"/api/player/control", "/api/player/seek", "/api/player/volume", "/api/player/mute", "/api/player/core", "/api/config"}:
            return self.reply({"ok": True})
        return self.reply({"error": "Not found"}, 404)


def main():
    candidates = list((Path.home() / ".local/share/mise/installs/npm-agent-browser").glob("*/node_modules/agent-browser/bin/agent-browser-linux-x64"))
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", default=str(candidates[-1]) if candidates else "agent-browser")
    parser.add_argument("--screenshots", default="/tmp/roon-beacon-react-qa")
    parser.add_argument("--skip-pointer-context", action="store_true", help="Skip slow context-poll loops while iterating unrelated assertions")
    parser.add_argument("--localization-only", action="store_true", help="Run focused Roon navigation, prompt, action and metadata localization regressions")
    parser.add_argument("--appearance-only", action="store_true", help="Check full theme labels at narrow widths and enlarged font sizes")
    parser.add_argument("--audio-network-only", action="store_true", help="Check audio network editing, persistence, errors and waiting feedback")
    arguments = parser.parse_args()
    FIXTURE["navigation_fixture"] = arguments.localization_only
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_port}"
    environment = {**os.environ, "AGENT_BROWSER_SESSION": f"roon-beacon-react-{uuid.uuid4().hex[:8]}"}
    screenshots = Path(arguments.screenshots)
    screenshots.mkdir(parents=True, exist_ok=True)

    def browser(*args):
        result = subprocess.run([arguments.browser, *args], env=environment, capture_output=True, text=True, timeout=45)
        if result.returncode:
            raise AssertionError(f"Browser command failed: {args}\n{result.stdout}\n{result.stderr}")
        return result.stdout

    def evaluate(script):
        return browser("eval", f"(async () => {{ const assert = (value, message) => {{ if (!value) throw new Error(message); }}; {script} return 'PASS'; }})()")

    def wait():
        time.sleep(.3)

    def requests_to(path):
        return [request for request in FIXTURE["requests"] if request["path"] == path]

    def poll(path="/api/player"):
        count = len(requests_to(path))
        deadline = time.monotonic() + 10
        while len(requests_to(path)) <= count:
            if time.monotonic() > deadline:
                raise AssertionError(f"Polling did not refresh {path}")
            time.sleep(.05)
        wait()

    def pointer_down(selector, fraction=.5):
        browser("scrollintoview", selector)
        coordinates = json.loads(browser("--json", "eval", f"(() => {{ const rect = document.querySelector({json.dumps(selector)}).getBoundingClientRect(); return {{x: rect.x + rect.width * {fraction}, y: rect.y + rect.height / 2}}; }})()"))["data"]["result"]
        browser("mouse", "move", str(round(coordinates["x"])), str(round(coordinates["y"])))
        browser("mouse", "down")

    def audit(name):
        raw = browser("a11y", "--tags", "wcag2a,wcag2aa", "--json")
        (screenshots / f"accessibility-{name}.json").write_text(raw)
        findings = json.loads(raw)
        assert findings["success"] and findings["data"]["counts"]["violations"] == 0, raw
        assert findings["data"]["counts"]["incomplete"] == 0, raw

    def audio_network_regressions():
        browser("click", "[data-lang=zh]")
        browser("click", "#settings summary")
        browser("wait", "--fn", "Boolean(document.querySelector('#audioNetworkSelect option[value=eth0]'))")
        evaluate("assert(document.querySelector('#audioNetworkHeading').textContent === '浏览器音频网络', 'Localized audio network heading'); assert(document.querySelector('#audioNetworkSelect option[value=eth0]').textContent.includes('192.0.2.10'), 'LAN address visible beside interface');")
        browser("select", "#audioNetworkSelect", "wlan0")
        FIXTURE["audio_interface"] = "eth0"
        poll("/api/audio/network")
        evaluate("assert(document.querySelector('#audioNetworkSelect').value === 'wlan0', 'Polling never overwrites unsaved selection');")
        FIXTURE["network_save_error"] = True
        browser("click", "#audioNetworkSave")
        browser("wait", "#audioNetworkError")
        evaluate("assert(document.querySelector('#audioNetworkError').textContent.includes('Fixture network save failed') && document.querySelector('#audioNetworkSelect').value === 'wlan0', 'Save failure retains selection and shows error');")
        FIXTURE["network_save_error"] = False
        browser("click", "#audioNetworkSave")
        browser("wait", "--fn", "document.querySelector('#audioNetworkFeedback').textContent.includes('已保存')")
        assert [item for item in requests_to("/api/audio/network") if item["method"] == "POST"][-1]["body"] == {"interface": "wlan0"}
        browser("reload")
        browser("wait", "#browseItems button")
        browser("click", "#settings summary")
        browser("wait", "--fn", "document.querySelector('#audioNetworkSelect').value === 'wlan0'")
        FIXTURE["audio_interface"] = "removed-interface"
        poll("/api/audio/network")
        evaluate("assert(document.querySelector('#audioNetworkSelect').value === 'removed-interface' && document.querySelector('#audioNetworkSave').disabled, 'Missing saved interface remains visible and recoverable');")
        browser("select", "#audioNetworkSelect", "")
        browser("click", "#audioNetworkSave")
        browser("wait", "--fn", "document.querySelector('#audioNetworkFeedback').textContent.includes('已保存')")
        assert any(item["method"] == "POST" and item["body"] == {"interface": None} for item in requests_to("/api/audio/network"))
        browser("set", "viewport", "375", "812")
        evaluate("assert(document.documentElement.scrollWidth <= innerWidth, 'Audio network settings fit mobile'); const audio = document.querySelector('#browserAudio'); audio.play = () => new Promise(() => {});")
        browser("click", "#audioJoin")
        evaluate("assert(document.querySelector('#audioStatus').textContent.includes('尚未收到音频'), 'Waiting receiver is not misrepresented as browser buffering'); assert(document.querySelector('.audio-diagnostics pre').textContent === '<script>fixture diagnostic</script>', 'Receiver diagnostic text is escaped');")
        browser("screenshot", "--full", str(screenshots / "audio-network-375-zh.png"))
        browser("click", "#audioLeave")
        evaluate("delete document.querySelector('#browserAudio').play;")
        browser("click", "[data-lang=en]")
        evaluate("assert(document.querySelector('#audioNetworkHeading').textContent === 'Browser audio network', 'Audio network language switches immediately');")
        browser("click", "#settings summary")

    try:
        browser("open", origin)
        browser("wait", "#browseItems button")
        if arguments.appearance_only:
            for width in (320, 375, 768, 1440):
                browser("set", "viewport", str(width), "900")
                for language in ("zh", "en"):
                    browser("click", f"[data-lang={language}]")
                    for font_size in (16, 20, 24, 32):
                        evaluate(f"document.documentElement.style.fontSize = '{font_size}px';")
                        for theme in ("light", "dark", "system"):
                            browser("select", "#themeSelect", theme)
                            evaluate("""
                                const select = document.querySelector('#themeSelect');
                                const style = getComputedStyle(select);
                                const context = document.createElement('canvas').getContext('2d');
                                context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
                                const labelWidth = context.measureText(select.selectedOptions[0].textContent).width;
                                const available = select.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
                                assert(labelWidth <= available, `Full theme label fits: ${select.selectedOptions[0].textContent}, text=${labelWidth}, available=${available}`);
                                const control = select.getBoundingClientRect();
                                const brand = document.querySelector('.brand').getBoundingClientRect();
                                assert(control.left >= 0 && control.right <= innerWidth, 'Theme control stays inside viewport');
                                assert(control.left >= brand.right || control.top >= brand.bottom, 'Theme control never overlaps logo');
                                assert(document.querySelector('.masthead').getBoundingClientRect().right <= innerWidth, 'Header fits viewport');
                            """)
                    browser("screenshot", str(screenshots / f"appearance-{width}-{language}-200pct.png"))
                    evaluate("document.documentElement.style.fontSize = '';")
            print(f"PASS: full EN/ZH theme labels and header layout at 320/375/768/1440px, 100/125/150/200% font sizes. Screenshots: {screenshots}", flush=True)
            return
        if not arguments.localization_only:
            audio_network_regressions()
            if arguments.audio_network_only:
                print("PASS: audio network addresses, dirty selection across polling, failure retention, save/reload, missing-interface recovery, automatic mode, waiting feedback and escaped diagnostics; mobile375.", flush=True)
                return
            FIXTURE["requests"].clear()
        if arguments.localization_only:
            browser("click", "[data-lang=zh]")
            evaluate("assert(document.querySelector('#browseTitle').textContent === '音乐库', 'Roon Library heading localized'); assert(Array.from(document.querySelectorAll('#browseItems .item-title'), item => item.textContent).join(',') === '搜索,艺人,专辑,曲目,作曲家,标签', 'All six Roon navigation labels localized');")
            browser("click", "#browseItems li:first-child button")
            evaluate("assert(document.querySelector('label[for=browseInput]').textContent === '搜索' && document.querySelector('#promptSubmit').textContent === '确定', 'Roon Search prompt and Go localized');")
            browser("fill", "#browseInput", "Library")
            browser("click", "[data-lang=en]")
            evaluate("assert(document.querySelector('#browseInput').value === 'Library' && document.querySelector('#promptSubmit').textContent === 'Go', 'Language switch preserves search input and restores Roon verb');")
            browser("click", "[data-lang=zh]")
            browser("click", "#promptSubmit")
            browser("wait", "--fn", "document.querySelector('#browseTitle').textContent === '搜索结果'")
            evaluate("assert(Array.from(document.querySelectorAll('#browseItems .item-title'), item => item.textContent).join(',') === 'Library,Search,Artists', 'Search-result music metadata is never translated');")
            browser("click", "#browseRoot")
            browser("wait", "--fn", "document.querySelectorAll('#browseItems button').length === 6")
            browser("click", "#browseItems li:nth-child(2) button")
            browser("wait", "--fn", "document.querySelector('#browseTitle').textContent === '艺人'")
            evaluate("assert(Array.from(document.querySelectorAll('#browseItems .item-title'), item => item.textContent).join(',') === 'Library,Search,Artists', 'Artist names matching navigation labels remain original');")
            browser("click", "#browseBack")
            browser("wait", "--fn", "document.querySelector('#browseTitle').textContent === '音乐库'")
            evaluate("assert(document.querySelector('#browseItems li:nth-child(2) .item-title').textContent === '艺人', 'Back restores localized navigation context');")
            browser("click", "#browseItems li:nth-child(2) button")
            browser("wait", "--fn", "document.querySelector('#browseTitle').textContent === '艺人'")
            browser("click", "#browseItems li:first-child button")
            browser("wait", "--fn", "document.querySelector('#browseTitle').textContent === 'Library'")
            evaluate("assert(document.querySelector('#browseItems .item-title').textContent === 'Search', 'Album and song titles remain original');")
            browser("click", "#browseItems button")
            browser("wait", "--fn", "document.querySelectorAll('#browseItems button').length === 3")
            evaluate("assert(document.querySelector('#browseTitle').textContent === 'Search', 'Song title remains original in action list'); assert(Array.from(document.querySelectorAll('#browseItems .item-title'), item => item.textContent).join(',') === '立即播放,下一首播放,加入队列', 'Roon action verbs localized');")
            browser("click", "#browseRoot")
            browser("wait", "--fn", "document.querySelectorAll('#browseItems button').length === 6")
            browser("set", "viewport", "375", "812")
            evaluate("assert(document.documentElement.scrollWidth <= innerWidth, 'Localized navigation fits mobile');")
            browser("screenshot", "--full", str(screenshots / "localized-navigation-375.png"))
            browser("click", "[data-lang=en]")
            evaluate("assert(document.querySelector('#browseTitle').textContent === 'Library' && document.querySelector('#browseItems li:nth-child(2) .item-title').textContent === 'Artists', 'Switching to English restores original navigation');")
            print("PASS: Roon navigation, prompts, actions, language switching, back navigation and original music metadata; mobile375.", flush=True)
            return
        evaluate("assert(crypto.randomUUID === undefined, 'LAN-insecure UUID fallback exercised'); assert(document.querySelector('#trackTitle').textContent === 'A quiet afternoon', 'Playing metadata'); assert(document.querySelector('#previous').disabled && !document.querySelector('#next').disabled, 'Transport capabilities'); assert(document.querySelectorAll('#volumeControls input:disabled').length === 2, 'Fixed and relative volume disabled'); assert(document.querySelectorAll('#volumeControls button:disabled').length === 2, 'Mute support'); const range = document.querySelector('#volumeControls input'); assert(range.min === '10' && range.max === '80' && range.step === '2', 'Volume bounds'); assert(!document.querySelector('#browserAudio').hasAttribute('src'), 'No autoplay'); assert(!document.querySelector('#browseItems img[src=x]'), 'Untrusted browse text is escaped'); window.qaSeek = document.querySelector('#seek'); window.qaVolume = range; window.qaOption = document.querySelector('#zoneSelect option');")
        for width, height in [(1440, 1000), (768, 1024), (375, 812)]:
            browser("set", "viewport", str(width), str(height))
            evaluate("const logo = document.querySelector('.brand-logo'); assert(logo.complete && logo.naturalWidth === 256 && logo.alt === 'Roon Beacon', 'Original branded logo loaded'); assert(logo.clientWidth >= 48 && logo.clientWidth <= 64, 'Responsive logo size'); assert(document.documentElement.scrollWidth <= innerWidth, 'No horizontal overflow including long titles');")
            browser("screenshot", "--full", str(screenshots / f"playing-{width}.png"))
            before = len(requests_to("/api/player/seek"))
            pointer_down("#seek", .7)
            browser("mouse", "up")
            wait()
            submitted = requests_to("/api/player/seek")[before:]
            assert submitted and 150 <= submitted[-1]["body"]["seconds"] <= 200, "Real mouse seek commits the chosen value, not the polled value"
            if width == 1440:
                target = submitted[-1]["body"]["seconds"]
                FIXTURE["position"] = 200
                poll()
                before = len(requests_to("/api/player/seek"))
                pointer_down("#seek", .7)
                browser("mouse", "up")
                wait()
                assert requests_to("/api/player/seek")[before:][-1]["body"]["seconds"] == target, "New gesture can submit the same target again after external position changes"
                FIXTURE["position"] = 42
                poll()
            before = len(requests_to("/api/player/volume"))
            pointer_down('#volumeControls input[aria-label="Living room volume"]', .7)
            browser("mouse", "up")
            wait()
            submitted = requests_to("/api/player/volume")[before:]
            assert submitted and 50 <= submitted[-1]["body"]["value"] <= 65, "Real mouse volume commits chosen value"
            if arguments.skip_pointer_context:
                continue
            pointer_down("#seek", .6)
            evaluate("window.qaDraggedSeek = document.querySelector('#seek').value;")
            poll()
            evaluate("assert(document.querySelector('#seek').value === qaDraggedSeek, 'Active native seek gesture survives polling');")
            before = len(requests_to("/api/player/seek"))
            FIXTURE.update(length=45, track_title="Brief interlude", position=3)
            poll()
            evaluate("assert(document.querySelector('#seek').max === '45' && document.querySelector('#seek').value === '3', 'Track change invalidates active seek bounds'); assert(document.querySelector('#duration').textContent === '0:45', 'Track duration refreshed');")
            browser("mouse", "up")
            wait()
            assert len(requests_to("/api/player/seek")) == before, "Stale old-track gesture is cancelled"
            FIXTURE.update(length=255, track_title="A quiet afternoon", position=42)
            poll()
            pointer_down('#volumeControls input[aria-label="Living room volume"]')
            before = len(requests_to("/api/player/volume"))
            FIXTURE.update(zone_id="zone-regrouped", volume=40, volume_max=50)
            poll()
            evaluate("const range = document.querySelector('#volumeControls input[aria-label=\"Living room volume\"]'); assert(range.max === '50' && range.value === '40', 'Regrouped volume adopts new bounds/context');")
            browser("mouse", "up")
            wait()
            assert len(requests_to("/api/player/volume")) == before, "Old-zone volume gesture cannot commit"
            pointer_down('#volumeControls input[aria-label="Living room volume"]')
            FIXTURE.update(output_id="output-replacement", volume=30)
            poll()
            browser("mouse", "up")
            wait()
            assert len(requests_to("/api/player/volume")) == before, "Detached output cannot commit"
            FIXTURE.update(zone_id="zone-one", output_id="output-one", volume=36, volume_max=80)
            poll()
            print(f"PASS: real pointer/context regressions at {width}px", flush=True)
        browser("set", "viewport", "1440", "1000")
        evaluate("document.querySelector('#seek').focus(); window.qaSeek = document.querySelector('#seek'); window.qaVolume = document.querySelector('#volumeControls input'); window.qaOption = document.querySelector('#zoneSelect option');")
        FIXTURE["position"] = 128
        poll()
        evaluate("assert(document.activeElement === qaSeek && qaSeek.value === '128', 'Idle focused seek follows polling without losing focus');")
        FIXTURE.update(length=45, track_title="Brief interlude", position=3)
        poll()
        evaluate("assert(qaSeek.max === '45' && qaSeek.value === '3', 'Idle focused seek updates short-track bounds'); assert(document.querySelector('#elapsed').textContent === '0:03', 'New-track elapsed agrees');")
        FIXTURE.update(length=255, track_title="A quiet afternoon", position=42, volume=54)
        evaluate("qaVolume.focus();")
        poll()
        evaluate("assert(qaVolume.value === '54' && document.activeElement === qaVolume, 'Idle focused volume follows polling'); assert(document.querySelector('#seek') === qaSeek && document.querySelector('#volumeControls input') === qaVolume && document.querySelector('#zoneSelect option') === qaOption, 'Stable React DOM across polling');")
        browser("focus", "#seek")
        browser("press", "Home")
        browser("press", "ArrowRight")
        wait()
        assert any(request["body"].get("seconds") == 1 for request in requests_to("/api/player/seek")), "Keyboard seek works"
        browser("select", "#zoneSelect", "zone-two")
        evaluate("assert(document.querySelector('#seek').disabled && document.querySelector('#next').disabled, 'Zone capabilities'); assert(document.querySelector('#playPause').getAttribute('aria-label') === 'Play', 'Stopped zone action');")
        browser("select", "#zoneSelect", "zone-one")
        browser("click", "#playPause")
        wait()
        assert any(request["body"].get("control") == "pause" for request in requests_to("/api/player/control"))
        browser("click", '#volumeControls button[aria-label="Mute Living room"]')
        wait()
        assert any(request["body"].get("is_muted") is True for request in requests_to("/api/player/mute"))
        browser("click", "#stop")
        browser("wait", "#playerError")
        evaluate("assert(document.querySelector('#playerError').textContent.includes('rejected stop'), 'Transport error shown');")
        browser("click", "#pageNext")
        browser("wait", "--text", "Second page")
        browser("click", "#pagePrevious")
        browser("wait", "--fn", "document.querySelector('#browseItems li:first-child')?.textContent.includes('A quiet afternoon') && !document.querySelector('#browseItems button')?.disabled")
        browser("click", "#browseItems li:first-child button")
        browser("wait", "--text", "Play now (from Roon)")
        browser("click", "#browseItems li:first-child button")
        browser("wait", "--text", "Roon queued this album.")
        evaluate("assert(document.querySelector('#browseError').hidden, 'Roon message is a notification');")
        browser("click", "#browseItems li:nth-child(2) button")
        browser("wait", "#browseError")
        evaluate("assert(document.querySelector('#browseError').textContent === 'Roon cannot play this selection.' && document.querySelector('#browseError').getAttribute('role') === 'alert', 'Roon is_error message is an accessible error'); assert(!document.querySelector('#browseStatus').textContent.includes('queued this album'), 'Success notice cleared');")
        browser("click", "#browseBack")
        browser("wait", "--text", "Test an error")
        assert any(request["body"].get("pop_levels") == 1 for request in requests_to("/api/player/browse"))
        browser("click", "#browseItems li:nth-child(2) button")
        browser("wait", "#browseError")
        evaluate("assert(document.querySelector('#browseError').textContent === 'Fixture browse failure', 'HTTP browse error shown');")
        browser("click", "#searchTab")
        browser("wait", "--text", "Search Roon")
        browser("click", "#browseItems button")
        browser("fill", "#browseInput", "Miles Davis")
        poll()
        evaluate("assert(document.querySelector('#browseInput').value === 'Miles Davis' && document.activeElement.id === 'browseInput', 'Prompt preserved by polling');")
        browser("click", "#promptSubmit")
        browser("wait", "--text", "Search results")
        assert any(request["body"].get("input") == "Miles Davis" and request["body"].get("hierarchy") == "search" for request in requests_to("/api/player/browse"))
        evaluate("window.qaPlayCalls = 0; HTMLMediaElement.prototype.play = function() { window.qaPlayCalls++; window.qaPlayWasGesture = navigator.userActivation.isActive; return Promise.resolve(); }; HTMLMediaElement.prototype.load = function() {}; HTMLMediaElement.prototype.pause = function() {}; assert(qaPlayCalls === 0, 'Audio requires gesture');")
        browser("click", "#audioJoin")
        evaluate("assert(qaPlayCalls === 1 && qaPlayWasGesture && document.querySelector('#browserAudio').getAttribute('src') === '/api/audio/stream', 'Synchronous gesture starts cookie-auth stream');")
        before = len(requests_to("/api/player/volume"))
        browser("focus", "#browserVolume")
        browser("press", "Home")
        browser("press", "ArrowRight")
        evaluate("assert(document.querySelector('#browserAudio').volume === .01 && document.querySelector('#browserVolumeValue').textContent === '1%', 'Browser loudness is local');")
        assert len(requests_to("/api/player/volume")) == before
        browser("click", "#audioLeave")
        evaluate("assert(!document.querySelector('#browserAudio').hasAttribute('src'), 'Leave tears down stream'); HTMLMediaElement.prototype.play = function() { return new Promise((resolve, reject) => { window.qaOldReject = reject; }); };")
        browser("click", "#audioJoin")
        browser("click", "#audioLeave")
        evaluate("HTMLMediaElement.prototype.play = function() { return Promise.resolve(); };")
        browser("click", "#audioJoin")
        evaluate("qaOldReject(new Error('Cancelled attempt')); await Promise.resolve(); assert(!document.querySelector('#audioLeave').disabled, 'Stale rejection cannot cancel a new join');")
        browser("click", "#audioLeave")
        browser("click", "#settings summary")
        browser("fill", "#name", "Unsaved edit")
        poll("/api/status")
        evaluate("assert(document.querySelector('#name').value === 'Unsaved edit', 'Settings edits survive polling');")
        browser("fill", "#name", "Invalid fixture")
        browser("click", "#configSave")
        browser("wait", "#configError")
        evaluate("assert(document.querySelector('#configError').textContent.includes('fixture validation'), 'Axum text validation fallback');")
        browser("fill", "#name", "Saved fixture")
        browser("click", "#configSave")
        browser("wait", "--text", "Settings saved")
        browser("click", "#settings summary")
        browser("set", "viewport", "375", "812")
        browser("click", "[data-lang=zh]")
        browser("select", "#themeSelect", "dark")
        evaluate("assert(document.documentElement.lang === 'zh-CN' && document.querySelector('#libraryHeading').textContent === '你的音乐库', 'Chinese UI'); assert(document.documentElement.dataset.theme === 'dark', 'Dark theme'); assert(document.documentElement.scrollWidth <= innerWidth, 'Chinese dark header fits mobile');")
        browser("reload")
        browser("wait", "#browseItems button")
        evaluate("assert(document.documentElement.lang === 'zh-CN' && document.querySelector('#themeSelect').value === 'dark' && document.documentElement.dataset.theme === 'dark', 'Language and theme persisted across reload');")
        browser("screenshot", "--full", str(screenshots / "dark-375-zh.png"))
        audit("dark")
        browser("click", "[data-lang=en]")
        browser("select", "#themeSelect", "light")
        audit("light")
        browser("select", "#themeSelect", "system")
        browser("set", "media", "dark")
        browser("wait", "--fn", "document.documentElement.dataset.theme === 'dark' && matchMedia('(prefers-color-scheme: dark)').matches")
        evaluate("assert(document.documentElement.dataset.theme === 'dark', 'System theme follows dark preference changes');")
        browser("set", "media", "light")
        browser("wait", "--fn", "document.documentElement.dataset.theme === 'light' && !matchMedia('(prefers-color-scheme: dark)').matches")
        evaluate("assert(document.documentElement.dataset.theme === 'light', 'System theme follows light preference changes');")
        browser("set", "media", "reduced-motion")
        browser("press", "Tab")
        evaluate("assert(matchMedia('(prefers-reduced-motion: reduce)').matches, 'Reduced motion'); assert(document.documentElement.dataset.input === 'keyboard', 'Keyboard mode'); assert(getComputedStyle(document.querySelector('#audioJoin')).transitionDuration === '0s', 'No keyboard animation');")
        FIXTURE["scenario"] = "empty"
        poll()
        poll("/api/audio/status")
        evaluate("assert(!document.querySelector('#onboarding').hidden, 'Honest extension/AirPlay onboarding'); assert(document.querySelector('#playPause').disabled && document.querySelector('#audioJoin').disabled, 'No fake playback');")
        browser("screenshot", "--full", str(screenshots / "empty-375.png"))
        FIXTURE["scenario"] = "authorization"
        poll()
        evaluate("assert(document.querySelector('#connectionState').textContent === 'Enable extension in Roon', 'Extension authorization guidance');")
        FIXTURE["scenario"] = "unselected"
        poll()
        evaluate("assert(document.querySelector('#coreSelect').value === '' && document.querySelector('#coreSelect option').textContent === 'Choose a Core', 'Null selected Core');")
        FIXTURE["scenario"] = "no-zones"
        poll()
        evaluate("assert(document.querySelector('#playPause').disabled && document.querySelector('#zoneSelect').disabled, 'No zones safe');")
        FIXTURE["scenario"] = "offline"
        poll()
        evaluate("assert(document.querySelector('#appError').textContent === 'Fixture Core is offline' && document.querySelector('#next').disabled, 'Offline error and capabilities');")
        FIXTURE.update(scenario="playing", required=True)
        browser("wait", "#loginDialog")
        evaluate("assert(document.querySelector('#main').inert && document.querySelector('#loginDialog').getAttribute('role') === 'dialog', 'Expired session locks with Radix dialog');")
        browser("press", "Tab")
        evaluate("assert(document.querySelector('#loginDialog').contains(document.activeElement), 'Modal traps keyboard focus');")
        browser("fill", "#accessToken", "wrong-token")
        browser("click", "#loginSubmit")
        browser("wait", "#loginError")
        evaluate("assert(document.querySelector('#loginError').textContent === 'Invalid web access token', 'Invalid token shown');")
        browser("fill", "#accessToken", "fixture-token")
        browser("click", "#loginSubmit")
        browser("wait", "#browseItems button")
        evaluate("assert(!document.querySelector('#loginDialog') && !document.querySelector('#main').inert, 'Cookie login unlocks'); assert(!Object.values(localStorage).includes('fixture-token') && !document.cookie.includes('qa_session'), 'No stored token or readable HttpOnly cookie');")
        FIXTURE["core_expire"] = True
        browser("select", "#coreSelect", "core-two")
        browser("wait", "#loginDialog")
        browser("fill", "#accessToken", "fixture-token")
        browser("click", "#loginSubmit")
        browser("wait", "#browseItems button")
        evaluate("assert(!document.querySelector('#next').disabled && !document.querySelector('#coreSelect').disabled, 'Core-selection 401 cannot leave controls selecting/disabled after login');")
        sessions = {request["body"]["session"] for request in requests_to("/api/player/browse")}
        assert len(sessions) == 2, "One session per document, stable through polls/auth; reload creates a new document"
        browser("tab", "new", origin)
        browser("wait", "#browseItems button")
        assert len({request["body"]["session"] for request in requests_to("/api/player/browse")}) == 3, "Each tab has a distinct browse session"
        browser("tab", "close")
        browser("click", "#authButton")
        browser("wait", "#loginDialog")
        evaluate("assert(document.querySelector('#main').inert, 'Logout locks remote');")
        print(f"PASS: React contracts, real pointer/focus regressions, stable DOM, long titles, logo, browse messages/prompts/actions, auth recovery, audio gestures, EN/ZH, persisted light/dark/system, reduced motion; 375/768/1440; axe zero violations/incomplete in light and dark. Context polling {'SKIPPED' if arguments.skip_pointer_context else 'PASSED'}. Screenshots: {screenshots}", flush=True)
    except Exception:
        print(browser("snapshot", "-i"), flush=True)
        print(json.dumps([request for request in FIXTURE["requests"] if request["method"] == "POST"][-10:], indent=2), flush=True)
        browser("screenshot", "--full", str(screenshots / "failure.png"))
        raise
    finally:
        browser("close")
        server.shutdown()


if __name__ == "__main__":
    main()
