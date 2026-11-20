"""A fake CapCut common_task API for recordings (no network).

The Rust replay implements the same server in crates/capdraft-core/tests/common/mod.rs.
Markers in a caption text pick a behavior: #HTTP500 #RET1 #NOTASK (submit), #QUERY500
#QRET1 #FAILTASK #SHORT #NOURL (poll), #DLFAIL #DL500 #EMPTY (download).
"""

from __future__ import annotations

import hashlib
import json
import re

import requests

VOICE_JSON_URL_PREFIX = "https://raw.githubusercontent.com/"


PROSODY = re.compile(r'<prosody rate="[^"]*">(.*?)</prosody>', re.S)


class FakeResp:
    def __init__(self, status, body: bytes, url=""):
        self.status_code = status
        self.content = body
        self.url = url
        self.reason = {200: "OK", 404: "Not Found", 500: "Internal Server Error"}.get(status, "")

    @property
    def text(self):
        return self.content.decode("utf-8", "replace")

    def json(self):
        return json.loads(self.text)

    def raise_for_status(self):
        if self.status_code >= 400:
            kind = "Client" if self.status_code < 500 else "Server"
            raise requests.HTTPError(f"{self.status_code} {kind} Error: {self.reason} for url: {self.url}")

    def iter_content(self, chunk_size=8192):
        for i in range(0, len(self.content), chunk_size):
            yield self.content[i:i + chunk_size]


def compact(o):
    return json.dumps(o, ensure_ascii=False, separators=(",", ":"))


class FakeCapCut:
    def __init__(self):
        self.tasks = {}
        self.log = []

    def post(self, url, headers=None, data=None, timeout=None):
        body = data.decode("utf-8") if isinstance(data, bytes) else data
        self.log.append({"method": "POST", "url": url, "headers": [[k, v] for k, v in headers.items()], "body": body})
        req = json.loads(body)
        if "/lv/v1/common_task/new" in url:
            ssml = json.loads(req["tasks"][0]["payload"])["ssml"]
            texts = PROSODY.findall(ssml)
            joined = "".join(texts)
            if "#HTTP500" in joined:
                return FakeResp(500, b"oops")
            if "#RET1" in joined:
                return FakeResp(200, compact({"ret": "1", "errmsg": "bad request"}).encode())
            if "#NOTASK" in joined:
                return FakeResp(200, compact({"ret": "0", "data": {"tasks": []}}).encode())
            tid = "T" + hashlib.md5(ssml.encode()).hexdigest()[:10]
            self.tasks[tid] = {"texts": texts, "polls": 0}
            return FakeResp(200, compact({"ret": "0", "data": {"tasks": [{"id": tid, "token": "K" + tid}]}}).encode())
        tid = req["tasks"][0]["id"]
        t = self.tasks[tid]
        t["polls"] += 1
        joined = "".join(t["texts"])
        if "#QUERY500" in joined and t["polls"] == 1:
            return FakeResp(500, b"busy")
        if "#QRET1" in joined and t["polls"] == 3:
            return FakeResp(200, compact({"ret": "5", "errmsg": "poll broke"}).encode())
        if t["polls"] < 3:
            return FakeResp(200, compact({"ret": "0", "data": {"tasks": [{"id": tid, "status": "running"}]}}).encode())
        if "#FAILTASK" in joined:
            return FakeResp(200, compact({"ret": "0", "data": {"tasks": [{"id": tid, "status": "failed"}]}}).encode())
        subs = []
        for i, tx in enumerate(t["texts"]):
            if "#NOURL" in tx:
                subs.append({"duration": 1000})
            else:
                subs.append({"speech_url": f"https://cdn.fake/{tid}/{i}.mp3", "duration": 1000 + 37 * len(tx)})
        if "#SHORT" in joined:
            subs = subs[:-1]
        payload = compact({"audio_subtitles": subs})
        return FakeResp(200, compact({"ret": "0", "data": {"tasks": [{"id": tid, "status": "succeed",
                                                                    "payload": payload}]}}).encode())

    def get(self, url, stream=False, timeout=None, headers=None):
        if url.startswith(VOICE_JSON_URL_PREFIX):
            return FakeResp(404, b"", url)  # the voice catalog: not part of the transcript
        self.log.append({"method": "GET", "url": url})
        m = re.match(r"https://cdn\.fake/(\w+)/(\d+)\.mp3", url)
        tid, i = m.group(1), int(m.group(2))
        text = self.tasks[tid]["texts"][i]
        if "#DLFAIL" in text:
            return FakeResp(404, b"", url)
        if "#DL500" in text:
            return FakeResp(500, b"", url)
        if "#EMPTY" in text:
            return FakeResp(200, b"", url)
        return FakeResp(200, f"MP3:{tid}:{i}:{text}".encode(), url)
