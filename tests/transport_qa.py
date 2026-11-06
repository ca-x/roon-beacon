"""Isolated transport, output-dialog and desktop-session browser contracts."""

import json
import os
from pathlib import Path
import subprocess
import threading
import time
import uuid
from http.server import ThreadingHTTPServer

import frontend_qa as fixture


STATE = {"settings": {"shuffle": False, "loop": "disabled", "auto_radio": False}, "native": False, "fail": False}
TOKEN = "a" * 64
ORIGINAL_PLAYER = fixture.player_fixture


def player():
    data = ORIGINAL_PLAYER()
    zone = data["zones"][0]
    zone["settings"] = STATE["settings"]
    zone["queue_items_remaining"] = 7
    primary = zone["outputs"][0]
    primary["can_group_with_output_ids"] = ["output-two"]
    primary["source_controls"] = [{"control_key": "amplifier", "display_name": "Amplifier", "status": "selected", "supports_standby": True}]
    secondary = {"output_id": "output-two", "display_name": "Headphones", "can_group_with_output_ids": [primary["output_id"]]}
    relative = zone["outputs"][-1]
    zone["outputs"] = [primary]
    data["zones"][1]["outputs"] = [secondary]
    data["outputs"] = [primary, secondary, relative]
    return data


class Handler(fixture.Handler):
    def route(self):
        if self.path == "/api/player/transport":
            body = json.loads(self.rfile.read(int(self.headers["content-length"])))
            fixture.FIXTURE["requests"].append({"path": self.path, "body": body})
            if STATE["fail"]:
                return self.reply({"error": "Device rejected this action"}, 502)
            if body["action"] == "settings":
                for field in ("shuffle", "auto_radio"):
                    if field in body:
                        STATE["settings"][field] = body[field]
                if "loop_mode" in body:
                    STATE["settings"]["loop"] = body["loop_mode"]
            return self.reply({"ok": True})
        if STATE["native"] and self.path == "/api/login":
            body = json.loads(self.rfile.read(int(self.headers["content-length"])))
            assert body == {"token": TOKEN}
            return self.reply({"ok": True}, cookie="qa_session=yes; HttpOnly; SameSite=Strict; Path=/")
        if STATE["native"] and self.path == "/api/audio/status":
            return self.reply({"mode": "native", "enabled": True, "available": True, "state": "waiting", "name": "Roon Beacon Desktop"})
        return super().route()


def main():
    fixture.player_fixture = player
    candidates = list((Path.home() / ".local/share/mise/installs/npm-agent-browser").glob("*/node_modules/agent-browser/bin/agent-browser-linux-x64"))
    executable = str(candidates[-1]) if candidates else "agent-browser"
    environment = {**os.environ, "AGENT_BROWSER_SESSION": f"beacon-transport-{uuid.uuid4().hex[:8]}"}
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_port}"
    screenshots = Path("/tmp/roon-beacon-transport-qa")
    screenshots.mkdir(exist_ok=True)

    def browser(*args):
        result = subprocess.run([executable, *args], env=environment, capture_output=True, text=True, timeout=45)
        assert result.returncode == 0, f"{args}: {result.stdout}\n{result.stderr}"
        return result.stdout

    def evaluate(code):
        return browser("eval", f"(() => {{ const assert = (ok, message) => {{ if (!ok) throw Error(message); }}; {code}; return 'PASS'; }})()")

    def action(selector):
        browser("scrollintoview", selector)
        browser("click", selector)
        time.sleep(.4)

    def last_action():
        return [item["body"] for item in fixture.FIXTURE["requests"] if item["path"] == "/api/player/transport"][-1]

    try:
        browser("open", origin)
        browser("wait", "#playPause")
        action('[data-lang="en"]')
        action('.mode-button[aria-label="Shuffle"]')
        assert last_action() == {"core_id": "core-one", "action": "settings", "zone_id": "zone-one", "shuffle": True}
        evaluate("assert(document.querySelector('[aria-label=Shuffle]').getAttribute('aria-pressed') === 'true', 'Server-confirmed shuffle')")
        action('.mode-button[aria-label="Repeat off"]')
        assert last_action()["loop_mode"] == "loop"
        action('.mode-button[aria-label="Repeat queue"]')
        assert last_action()["loop_mode"] == "loop_one"
        action('.radio-mode')
        assert last_action()["auto_radio"] is True
        action('.output-manager-trigger')
        browser("wait", ".output-dialog")
        evaluate("assert(document.querySelector('.output-dialog').contains(document.activeElement), 'Dialog owns focus')")
        action('.output-choice:has(input:not(:checked))')
        action('.output-routing-actions .primary')
        assert last_action()["action"] == "group" and last_action()["output_ids"] == ["output-one", "output-two"]
        browser("select", ".transfer-destination select", "zone-two")
        action('.transfer-section .secondary')
        assert last_action()["action"] == "transfer" and last_action()["to_zone_id"] == "zone-two"
        action('.source-controls button:nth-child(2)')
        assert last_action()["command"] == "standby"
        action('.device-step-volume button:last-child')
        assert last_action()["steps"] == 1
        STATE["fail"] = True
        action('.pause-all-row button')
        evaluate("assert(document.querySelector('#outputError').textContent === 'Device rejected this action', 'Roon failure visible')")
        STATE["fail"] = False
        for width in (375, 1440):
            browser("set", "viewport", str(width), "900")
            for theme in ("light", "dark"):
                evaluate(f"document.documentElement.dataset.theme = '{theme}'")
                evaluate("document.querySelector('.output-dialog-scroll').scrollTop = 0")
                findings = json.loads(browser("a11y", "--selector", ".output-dialog", "--tags", "wcag2a,wcag2aa", "--json"))
                assert findings["success"] and findings["data"]["counts"]["violations"] == 0, findings
                for issue in findings["data"]["incomplete"]:
                    assert issue["id"] == "color-contrast", issue
                    assert all(node["html"].startswith(('<h2 id="radix-', '<p id="outputDescription"')) for node in issue["nodes"]), issue
                evaluate("const luminance = color => { const channels = color.match(/[\\d.]+/g).slice(0, 3).map(Number).map(value => { value /= 255; return value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4; }); return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722; }; const background = luminance(getComputedStyle(document.querySelector('.output-dialog-heading')).backgroundColor); for (const element of document.querySelectorAll('.output-dialog-heading h2, #outputDescription')) { const foreground = luminance(getComputedStyle(element).color); assert((Math.max(background, foreground) + .05) / (Math.min(background, foreground) + .05) >= 4.5, 'Header contrast independently verified despite axe overlap uncertainty'); }")
                browser("screenshot", str(screenshots / f"outputs-{width}-{theme}.png"))
            evaluate("assert(document.documentElement.scrollWidth <= innerWidth, 'No horizontal overflow')")
        browser("press", "Escape")
        evaluate("assert(!document.querySelector('.output-dialog') && document.activeElement.classList.contains('output-manager-trigger'), 'Escape restores focus')")
        action('[data-lang="zh"]')
        action('.output-manager-trigger')
        evaluate("assert(document.querySelector('.output-dialog h2').textContent === '想在哪里听？', 'Chinese output dialog')")
        browser("press", "Escape")
        STATE["native"] = True
        fixture.FIXTURE["required"] = True
        browser("open", "about:blank")
        browser("open", f"{origin}/#desktop-token={TOKEN}")
        browser("wait", "#nativeHeading")
        evaluate("assert(!location.hash, 'Bootstrap token removed'); assert(!document.querySelector('#browserAudio'), 'Native output never joins browser MP3'); assert(!document.querySelector('#loginDialog'), 'Desktop session authenticated'); assert(!Object.values(localStorage).includes('a'.repeat(64)), 'Token never persisted')")
        browser("screenshot", "--full", str(screenshots / "linux-desktop-zh.png"))
        print(f"PASS: typed transport bodies, server-confirmed modes, grouping, transfers, device controls, errors, focus, EN/ZH, 375/1440 light/dark axe, native audio and desktop token bootstrap. Screenshots: {screenshots}")
    except Exception:
        print(browser("snapshot", "-i"), flush=True)
        browser("screenshot", str(screenshots / "failure.png"))
        raise
    finally:
        browser("close")
        server.shutdown()


if __name__ == "__main__":
    main()
